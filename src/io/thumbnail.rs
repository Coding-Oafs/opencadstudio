//! DWG preview thumbnails.
//!
//! - [`from_snapshot`] rasterizes the drawing exactly as it is framed on screen —
//!   the live camera's pan / zoom / rotation, only the currently visible region,
//!   not the whole extent — into a small [`acadrust::Preview`] embedded on save
//!   so OCS drawings show a thumbnail in file browsers and other CAD apps.
//! - [`read_handle`] / [`extract_to_png`] read a DWG's *embedded* preview back
//!   for the Start page and the OS file-manager thumbnailer. Extraction lives in
//!   the shared [`dwg_thumbnailer`] core crate (also used by the Windows/macOS
//!   thumbnail handlers).

use acadrust::{Preview, PreviewFormat};
use iced::Rectangle;
use image::{ImageFormat, Rgb, RgbImage};
use std::io::Cursor;

use crate::scene::WireModel;
use crate::scene::view::camera::Camera;

/// Longest edge of the generated thumbnail, in pixels.
const MAX_DIM: u32 = 256;

/// Build a preview matching what is currently on screen: the drawing's wires
/// projected through the live camera into a `viewport`-aspect canvas, so the
/// thumbnail is the visible framing (pan / zoom / rotation, only the on-screen
/// region), not the whole extent. `viewport` is the model pane's pixel size.
/// `None` when the drawing is empty (clears any stale preview).
///
/// `png` picks the encoding: a line drawing on a flat background is almost all
/// one colour, so a **PNG** collapses to a few KB where the uncompressed
/// **BMP/DIB** stays ~180 KB at 256². PNG previews are only valid from R2013
/// (AC1027) on, so the caller passes `false` for older targets → BMP/DIB.
///
/// Native background saves retain
/// the resident wire `Arc` and a small camera copy, then run the point scan and
/// image encoding on the save worker instead of blocking the UI thread.
pub fn from_snapshot(
    wires: &[WireModel],
    camera: &Camera,
    bg_color: [f32; 4],
    png: bool,
    viewport: (f32, f32),
) -> Option<Preview> {
    if wires.is_empty() {
        return None;
    }
    let (vw, vh) = viewport;
    if !(vw > 0.0 && vh > 0.0) {
        return None;
    }

    // Canvas keeps the viewport's aspect so the framing is undistorted; longest
    // edge = MAX_DIM. Projecting with the canvas rectangle as the camera bounds
    // makes `project` return pixel coordinates already in canvas space.
    let (cw, ch) = canvas_dims((vw / vh) as f64);
    let bounds = Rectangle {
        x: 0.0,
        y: 0.0,
        width: cw as f32,
        height: ch as f32,
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        rasterize_snapshot_parallel(wires, camera, bounds, cw, ch, bg_color, png)
    }
    #[cfg(target_arch = "wasm32")]
    {
        rasterize(wires, cw, ch, bg_color, png, |x, y, z| {
            camera.project(glam::DVec3::new(x, y, z), bounds)
                .map(|s| (s.x.round() as i32, s.y.round() as i32))
        })
    }
}

/// Canvas dimensions for an aspect ratio, longest edge = [`MAX_DIM`].
fn canvas_dims(aspect: f64) -> (u32, u32) {
    if aspect >= 1.0 {
        (MAX_DIM, ((MAX_DIM as f64 / aspect).round() as u32).clamp(16, MAX_DIM))
    } else {
        (((MAX_DIM as f64 * aspect).round() as u32).clamp(16, MAX_DIM), MAX_DIM)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn rasterize_snapshot_parallel(
    wires: &[WireModel],
    camera: &Camera,
    bounds: Rectangle,
    cw: u32,
    ch: u32,
    bg: [f32; 4],
    png: bool,
) -> Option<Preview> {
    use crate::par::prelude::*;

    let total_segments: usize = wires
        .iter()
        .map(|wire| wire.points.len().saturating_sub(1))
        .sum();
    if total_segments < 100_000 {
        return rasterize(wires, cw, ch, bg, png, |x, y, z| {
            camera.project(glam::DVec3::new(x, y, z), bounds)
                .map(|point| (point.x.round() as i32, point.y.round() as i32))
        });
    }

    let task_count = (rayon::current_num_threads().max(1) * 2).min(total_segments);
    let target_work = total_segments.div_ceil(task_count).max(1);
    let mut tasks: Vec<Vec<(usize, usize, usize)>> = Vec::with_capacity(task_count);
    let mut task = Vec::new();
    let mut task_work = 0usize;
    for (wire_index, wire) in wires.iter().enumerate() {
        let segment_count = wire.points.len().saturating_sub(1);
        let mut start = 0usize;
        while start < segment_count {
            let take = (target_work - task_work).min(segment_count - start);
            task.push((wire_index, start, start + take));
            task_work += take;
            start += take;
            if task_work == target_work {
                tasks.push(std::mem::take(&mut task));
                task_work = 0;
            }
        }
    }
    if !task.is_empty() {
        tasks.push(task);
    }

    let layers: Vec<Vec<u32>> = tasks
        .par_iter()
        .map(|task| {
            let mut pixels = vec![u32::MAX; (cw * ch) as usize];
            for &(wire_index, start, end) in task {
                let wire = &wires[wire_index];
                let color = to_rgb(wire.color);
                let packed =
                    color[0] as u32 | (color[1] as u32) << 8 | (color[2] as u32) << 16;
                let mut previous = projected_wire_point(wire, start, camera, bounds);
                for index in start + 1..=end {
                    let current = projected_wire_point(wire, index, camera, bounds);
                    if let (Some(a), Some(b)) = (previous, current) {
                        draw_line_layer(&mut pixels, cw, ch, a, b, packed);
                    }
                    previous = current;
                }
            }
            pixels
        })
        .collect();

    let mut image = RgbImage::from_pixel(cw, ch, Rgb(to_rgb(bg)));
    for layer in layers {
        for (pixel, packed) in image.pixels_mut().zip(layer) {
            if packed != u32::MAX {
                *pixel = Rgb([
                    packed as u8,
                    (packed >> 8) as u8,
                    (packed >> 16) as u8,
                ]);
            }
        }
    }
    encode(image, png)
}

#[cfg(not(target_arch = "wasm32"))]
fn projected_wire_point(
    wire: &WireModel,
    index: usize,
    camera: &Camera,
    bounds: Rectangle,
) -> Option<(i32, i32)> {
    let point = wire.points.get(index)?;
    if !point[0].is_finite() || !point[1].is_finite() {
        return None;
    }
    let (x, y, z) = abs_xyz(wire, index, point);
    camera
        .project(glam::DVec3::new(x, y, z), bounds)
        .map(|screen| (screen.x.round() as i32, screen.y.round() as i32))
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_line_layer(
    pixels: &mut [u32],
    width: u32,
    height: u32,
    (x0, y0): (i32, i32),
    (x1, y1): (i32, i32),
    color: u32,
) {
    let (width, height) = (width as i32, height as i32);
    let Some(((mut x0, mut y0), (x1, y1))) =
        clip_line_to_rect((x0, y0), (x1, y1), width, height)
    else {
        return;
    };
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        if x0 >= 0 && x0 < width && y0 >= 0 && y0 < height {
            pixels[y0 as usize * width as usize + x0 as usize] = color;
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice = 2 * error;
        if twice >= dy {
            error += dy;
            x0 += sx;
        }
        if twice <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

/// Rasterize `wires` onto a `bg`-filled `cw`×`ch` canvas, placing each vertex
/// with `project` (world XYZ → canvas pixel, `None` = not projectable), and
/// encode the result. A `None` from `project` breaks the polyline run, as does a
/// NaN separator, so off-screen / clipped segments simply don't draw.
fn rasterize(
    wires: &[WireModel],
    cw: u32,
    ch: u32,
    bg: [f32; 4],
    png: bool,
    project: impl Fn(f64, f64, f64) -> Option<(i32, i32)>,
) -> Option<Preview> {
    let mut img = RgbImage::from_pixel(cw, ch, Rgb(to_rgb(bg)));
    for w in wires {
        let col = Rgb(to_rgb(w.color));
        let mut prev: Option<(i32, i32)> = None;
        for (i, p) in w.points.iter().enumerate() {
            if !p[0].is_finite() || !p[1].is_finite() {
                prev = None; // NaN separator breaks the run
                continue;
            }
            let (x, y, z) = abs_xyz(w, i, p);
            let cur = project(x, y, z);
            if let (Some(a), Some(b)) = (prev, cur) {
                draw_line(&mut img, a, b, col);
            }
            prev = cur;
        }
    }
    encode(img, png)
}

/// Encode the canvas. PNG for R2013+ targets (few KB); else a BMP → DIB (no
/// 14-byte BITMAPFILEHEADER, which the DWG preview container doesn't carry).
/// The BMP is an 8-bit **RLE8**-compressed DIB — a line drawing on a flat
/// background is a few distinct colours with long single-colour runs, so it
/// collapses from the ~180 KB of a 24-bit DIB to a handful of KB. A view with
/// more than 256 distinct colours (rare) can't be palettised, so it falls back
/// to the 24-bit uncompressed DIB.
fn encode(img: RgbImage, png: bool) -> Option<Preview> {
    if png {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, ImageFormat::Png).ok()?;
        let data = buf.into_inner();
        return (!data.is_empty()).then_some(Preview { format: PreviewFormat::Png, data });
    }
    let data = rle8_dib(&img).or_else(|| bmp24_dib(&img))?;
    Some(Preview { format: PreviewFormat::Bmp, data })
}

/// Build an 8-bit palettised, `BI_RLE8`-compressed DIB. `None` when the image
/// holds more than 256 distinct colours (the caller then uses 24-bit).
fn rle8_dib(img: &RgbImage) -> Option<Vec<u8>> {
    let (w, h) = (img.width(), img.height());
    // Exact palette + per-pixel index (top-to-bottom, left-to-right).
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut lut: std::collections::HashMap<[u8; 3], u8> = std::collections::HashMap::new();
    let mut idx = Vec::with_capacity((w * h) as usize);
    for px in img.pixels() {
        let c = [px.0[0], px.0[1], px.0[2]];
        let i = if let Some(&i) = lut.get(&c) {
            i
        } else {
            if palette.len() >= 256 {
                return None;
            }
            let i = palette.len() as u8;
            palette.push(c);
            lut.insert(c, i);
            i
        };
        idx.push(i);
    }

    // RLE8 body, rows bottom-up (BMP stores the last image row first). Encoded
    // runs only: 2 bytes per single-colour run — ideal for flat-fill previews.
    let mut rle = Vec::new();
    for (n, row) in (0..h).rev().enumerate() {
        let line = &idx[(row * w) as usize..(row * w + w) as usize];
        let mut x = 0usize;
        while x < line.len() {
            let v = line[x];
            let mut run = 1usize;
            while x + run < line.len() && line[x + run] == v && run < 255 {
                run += 1;
            }
            rle.push(run as u8);
            rle.push(v);
            x += run;
        }
        if n + 1 < h as usize {
            rle.extend_from_slice(&[0, 0]); // end of line
        }
    }
    rle.extend_from_slice(&[0, 1]); // end of bitmap

    // BITMAPINFOHEADER (40) + full 256-entry palette (BGRA) + RLE body. The
    // 256-entry palette is required: the reader derives the pixel-data offset as
    // `(1 << bitCount) * 4`, so a short palette would misplace it.
    let mut dib = Vec::with_capacity(40 + 1024 + rle.len());
    dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
    dib.extend_from_slice(&(w as i32).to_le_bytes());
    dib.extend_from_slice(&(h as i32).to_le_bytes()); // + = bottom-up
    dib.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    dib.extend_from_slice(&8u16.to_le_bytes()); // biBitCount
    dib.extend_from_slice(&1u32.to_le_bytes()); // biCompression = BI_RLE8
    dib.extend_from_slice(&(rle.len() as u32).to_le_bytes()); // biSizeImage
    dib.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    dib.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    dib.extend_from_slice(&256u32.to_le_bytes()); // biClrUsed
    dib.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant
    for i in 0..256 {
        let c = palette.get(i).copied().unwrap_or([0, 0, 0]);
        dib.extend_from_slice(&[c[2], c[1], c[0], 0]); // BGRA
    }
    dib.extend_from_slice(&rle);
    Some(dib)
}

/// 24-bit uncompressed DIB (fallback): the `image` BMP minus its file header.
fn bmp24_dib(img: &RgbImage) -> Option<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Bmp).ok()?;
    let bmp = buf.into_inner();
    (bmp.len() > 14).then(|| bmp[14..].to_vec())
}

/// Absolute world XYZ of vertex `i`, reconstructing the double-single residual.
#[inline]
fn abs_xyz(w: &WireModel, i: usize, p: &[f32; 3]) -> (f64, f64, f64) {
    let (lx, ly, lz) = w
        .points_low
        .get(i)
        .map_or((0.0, 0.0, 0.0), |l| (l[0] as f64, l[1] as f64, l[2] as f64));
    (p[0] as f64 + lx, p[1] as f64 + ly, p[2] as f64 + lz)
}

#[inline]
fn to_rgb(c: [f32; 4]) -> [u8; 3] {
    [
        (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

/// Read a DWG's embedded preview and write it as a PNG at `output`, scaled so
/// its longest edge is at most `size`. Returns `false` on any failure (no
/// preview, undecodable, write error) so the OS thumbnailer falls back to a
/// generic icon. Backs the hidden `--dwg-thumbnail` mode the installed
/// freedesktop `.thumbnailer` invokes. Extraction lives in the shared
/// [`dwg_thumbnailer`] core (also used by the Windows/macOS handlers).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn extract_to_png(input: &std::path::Path, output: &std::path::Path, size: u32) -> bool {
    match dwg_thumbnailer::extract(input, size) {
        Some(mut img) => {
            // Bottom-left "DWG" ribbon so the format reads at a glance in the
            // file manager (the Start-page `read_handle` stays unbadged).
            dwg_thumbnailer::badge_dwg(&mut img);
            img.save_with_format(output, ImageFormat::Png).is_ok()
        }
        None => false,
    }
}

/// Read a DWG's embedded preview and decode it to an iced image handle for the
/// Start page's recent-file thumbnails. `None` for DXF/other files, a missing
/// preview, or an undecodable format (WMF).
pub fn read_handle(path: &std::path::Path) -> Option<iced::widget::image::Handle> {
    let img = dwg_thumbnailer::extract(path, MAX_DIM)?;
    let (w, h) = (img.width(), img.height());
    Some(iced::widget::image::Handle::from_rgba(w, h, img.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(pts: &[[f32; 2]], color: [f32; 4]) -> WireModel {
        WireModel {
            points: pts.iter().map(|&[x, y]| [x, y, 0.0]).collect(),
            color,
            ..Default::default()
        }
    }

    /// Prepend a `BITMAPFILEHEADER` so `image` can decode the DIB. Mirrors the
    /// palette-aware offset the shared `dwg_thumbnailer::dib_to_bmp` computes.
    fn dib_to_bmp(dib: &[u8]) -> Vec<u8> {
        let bi_size = u32::from_le_bytes([dib[0], dib[1], dib[2], dib[3]]) as usize;
        let bpp = u16::from_le_bytes([dib[14], dib[15]]) as usize;
        let palette = if (1..=8).contains(&bpp) { (1usize << bpp) * 4 } else { 0 };
        let mut v = Vec::with_capacity(14 + dib.len());
        v.extend_from_slice(b"BM");
        v.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&((14 + bi_size + palette) as u32).to_le_bytes());
        v.extend_from_slice(dib);
        v
    }

    #[test]
    fn canvas_keeps_aspect_with_max_dim_edge() {
        assert_eq!(canvas_dims(1.0), (MAX_DIM, MAX_DIM));
        assert_eq!(canvas_dims(2.0), (MAX_DIM, MAX_DIM / 2)); // wide
        assert_eq!(canvas_dims(0.5), (MAX_DIM / 2, MAX_DIM)); // tall
    }

    #[test]
    fn rasterize_draws_a_valid_non_blank_dib() {
        let bg = [0.1, 0.1, 0.1, 1.0];
        // A closed square (connected polyline) in a distinct colour.
        let sq = wire(
            &[[10.0, 10.0], [90.0, 10.0], [90.0, 90.0], [10.0, 90.0], [10.0, 10.0]],
            [1.0, 0.0, 0.0, 1.0],
        );
        // Trivial projector: world XY straight to pixels (Y flipped), z ignored.
        let p = rasterize(&[sq], MAX_DIM, MAX_DIM, bg, false, |x, y, _| {
            Some((x.round() as i32, (MAX_DIM as f64 - y).round() as i32))
        })
        .expect("some preview");
        assert_eq!(p.format, PreviewFormat::Bmp);
        // DIB starts with a 40-byte BITMAPINFOHEADER.
        assert_eq!(&p.data[0..4], &40u32.to_le_bytes());
        let img = image::load_from_memory(&dib_to_bmp(&p.data)).expect("decodes").to_rgb8();
        assert_eq!((img.width(), img.height()), (MAX_DIM, MAX_DIM));
        let bg_px = to_rgb(bg);
        assert!(img.pixels().any(|px| px.0 != bg_px), "nothing drawn");
        assert!(img.pixels().any(|px| px.0[0] > 128 && px.0[1] < 64), "square not red");
    }

    #[test]
    fn rle8_bmp_is_8bit_compressed_and_round_trips() {
        let bg = [1.0, 1.0, 1.0, 1.0];
        let sq = wire(&[[10.0, 10.0], [90.0, 90.0]], [0.0, 0.0, 0.0, 1.0]);
        let proj = |x: f64, y: f64, _z: f64| Some((x.round() as i32, y.round() as i32));
        let bmp = rasterize(&[sq], MAX_DIM, MAX_DIM, bg, false, proj).unwrap();
        assert_eq!(bmp.format, PreviewFormat::Bmp);
        // 8-bit, BI_RLE8.
        assert_eq!(u16::from_le_bytes([bmp.data[14], bmp.data[15]]), 8, "bitcount");
        assert_eq!(
            u32::from_le_bytes([bmp.data[16], bmp.data[17], bmp.data[18], bmp.data[19]]),
            1,
            "compression = BI_RLE8"
        );
        // Far under a 24-bit DIB of the same canvas (256·256·3 = 196 608).
        assert!(bmp.data.len() < 196_608 / 10, "rle8 {} not << 24-bit", bmp.data.len());
        // Decodes through the exact path the reader uses, line preserved.
        let img = image::load_from_memory(&dib_to_bmp(&bmp.data)).expect("rle8 decodes").to_rgb8();
        assert_eq!((img.width(), img.height()), (MAX_DIM, MAX_DIM));
        assert!(img.pixels().any(|px| px.0 == [0, 0, 0]), "black line missing");
    }

    #[test]
    fn png_preview_decodes() {
        let bg = [1.0, 1.0, 1.0, 1.0];
        let sq = wire(&[[10.0, 10.0], [90.0, 90.0]], [0.0, 0.0, 0.0, 1.0]);
        let p = rasterize(&[sq], MAX_DIM, MAX_DIM, bg, true, |x, y, _| {
            Some((x.round() as i32, y.round() as i32))
        })
        .unwrap();
        assert_eq!(p.format, PreviewFormat::Png);
        let img = image::load_from_memory_with_format(&p.data, ImageFormat::Png).expect("png decodes");
        assert_eq!((img.width(), img.height()), (MAX_DIM, MAX_DIM));
    }
}

/// Bresenham line, clipped to the image bounds.
fn draw_line(img: &mut RgbImage, (x0, y0): (i32, i32), (x1, y1): (i32, i32), col: Rgb<u8>) {
    let (w, h) = (img.width() as i32, img.height() as i32);
    let Some(((mut x0, mut y0), (x1, y1))) =
        clip_line_to_rect((x0, y0), (x1, y1), w, h)
    else {
        return;
    };
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if x0 >= 0 && x0 < w && y0 >= 0 && y0 < h {
            img.put_pixel(x0 as u32, y0 as u32, col);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn clip_line_to_rect(
    (x0, y0): (i32, i32),
    (x1, y1): (i32, i32),
    width: i32,
    height: i32,
) -> Option<((i32, i32), (i32, i32))> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let (x0, y0, x1, y1) = (x0 as f64, y0 as f64, x1 as f64, y1 as f64);
    let (dx, dy) = (x1 - x0, y1 - y0);
    let mut enter = 0.0f64;
    let mut leave = 1.0f64;
    for (p, q) in [
        (-dx, x0),
        (dx, width.saturating_sub(1) as f64 - x0),
        (-dy, y0),
        (dy, height.saturating_sub(1) as f64 - y0),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = q / p;
        if p < 0.0 {
            enter = enter.max(ratio);
        } else {
            leave = leave.min(ratio);
        }
        if enter > leave {
            return None;
        }
    }
    let point = |t: f64| {
        (
            (x0 + t * dx).round().clamp(0.0, width.saturating_sub(1) as f64) as i32,
            (y0 + t * dy).round().clamp(0.0, height.saturating_sub(1) as f64) as i32,
        )
    };
    Some((point(enter), point(leave)))
}
