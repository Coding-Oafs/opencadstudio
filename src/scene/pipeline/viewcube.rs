// ViewCube wgpu pipeline — OpenCADStudio-style interactive 3D navigation cube.
//
// 26 selectable regions:  6 faces + 12 edges + 8 corners.
// Phong shading + hover highlight passed as uniform.
// Hit-test is 100% CPU — no GPU readback needed.
//
// The ViewCube rotation matrix is derived directly from the camera quaternion
// (cam_rotation: Mat4) everywhere — shader, text labels, hit-test, hover-id.
// This eliminates gimbal lock at top/bottom views and keeps the cube in sync
// with arcball orbit at all angles.

use bytemuck::{Pod, Zeroable};
use glam::camera::rh::proj::directx::orthographic;
use glam::{Mat4, Vec3, Vec4};
use iced::wgpu;
use iced::{Rectangle, Size};
use crate::t;

// ── ViewCube layout ───────────────────────────────────────────────────────
pub const VIEWCUBE_PX: u32 = 84; // 30% smaller cube (was 120)
pub const VIEWCUBE_SCALE: f32 = 0.36;
pub const VIEWCUBE_DRAW_PX: f32 = VIEWCUBE_PX as f32 * VIEWCUBE_SCALE * 2.0;
pub const VIEWCUBE_PAD: f32 = 12.0;
/// The cube centre is inset from the screen corner by this multiple of the
/// cube half-size, leaving room for the compass ring and the nav controls
/// (home / roll / nudge) drawn around it. Tighter than before so the controls
/// hug the cube in the corner instead of floating in a large dead region.
pub const NAV_INSET_F: f32 = 2.0;
/// Side of the whole nav widget (cube + compass ring + controls) in pixels.
pub const VIEWCUBE_REGION_PX: f32 = VIEWCUBE_DRAW_PX * NAV_INSET_F;
/// Z height of the compass ring + cardinals in cube-local space. The cube
/// spans ±1, so −1 parks the ring at the cube's base — it sits *under* the
/// cube in 3D views and reads as a ground compass.
const RING_Z: f32 = -1.0;
/// Horizontal radius of the N/E/S/W cardinal letters — the ring's mid-line, so
/// they sit painted on the ring band.
const R_CARD: f32 = 1.57;

/// Compass cardinal directions, mapped to a side-face snap when clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinal {
    North,
    East,
    South,
    West,
}

/// 90° view nudge directions (tip the cube up/down or spin it left/right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeDir {
    Up,
    Down,
    Left,
    Right,
}

/// Face labels in the current UI language, sourced from `locales/*.yml`
/// (`ViewCube Top` …). The cube text is drawn from a fixed bitmap atlas,
/// so the translated values must stay inside the atlas glyph set — the same
/// vocabulary the `glyph_index`/`latin_rows`/`han_rows` tables below agree on.
fn face_labels() -> [std::borrow::Cow<'static, str>; 6] {
    [
        t!("ViewCube Top"),
        t!("ViewCube Bottom"),
        t!("ViewCube Front"),
        t!("ViewCube Back"),
        t!("ViewCube Right"),
        t!("ViewCube Left"),
    ]
}

const FACE_CENTERS: [[f32; 3]; 6] = [
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
    [0.0, -1.0, 0.0],
    [0.0, 1.0, 0.0],
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
];

pub const FACE_TOP: usize = 0;
pub const FACE_BOTTOM: usize = 1;
pub const FACE_FRONT: usize = 2;
pub const FACE_BACK: usize = 3;
pub const FACE_RIGHT: usize = 4;
pub const FACE_LEFT: usize = 5;
pub const EDGE_TOP_FRONT: usize = 6;
pub const EDGE_TOP_BACK: usize = 7;
pub const EDGE_TOP_RIGHT: usize = 8;
pub const EDGE_TOP_LEFT: usize = 9;
pub const EDGE_BOT_FRONT: usize = 10;
pub const EDGE_BOT_BACK: usize = 11;
pub const EDGE_BOT_RIGHT: usize = 12;
pub const EDGE_BOT_LEFT: usize = 13;
pub const EDGE_FRONT_RIGHT: usize = 14;
pub const EDGE_FRONT_LEFT: usize = 15;
pub const EDGE_BACK_RIGHT: usize = 16;
pub const EDGE_BACK_LEFT: usize = 17;
pub const CORNER_TPF_R: usize = 18;
pub const CORNER_TPF_L: usize = 19;
pub const CORNER_TBK_R: usize = 20;
pub const CORNER_TBK_L: usize = 21;
pub const CORNER_BTF_R: usize = 22;
pub const CORNER_BTF_L: usize = 23;
pub const CORNER_BBK_R: usize = 24;
pub const CORNER_BBK_L: usize = 25;
pub const NUM_REGIONS: usize = 26;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CubeRegion {
    Face(usize),
    Edge(usize),
    Corner(usize),
}

impl CubeRegion {
    pub fn id(self) -> usize {
        match self {
            Self::Face(i) | Self::Edge(i) | Self::Corner(i) => i,
        }
    }

    /// Unit eye-direction vector (from target toward the camera) that
    /// looks straight at this region. Used by `Camera::snap_to_direction`
    /// which derives the full orientation by re-using the current
    /// camera's up vector, projected onto the plane perpendicular to
    /// this direction — so clicking an edge spins the cube around the
    /// edge without rolling the user's "up" sense.
    pub fn snap_direction(self) -> glam::Vec3 {
        let c = region_centroids()[self.id()];
        glam::Vec3::new(c[0], c[1], c[2]).normalize_or(glam::Vec3::Z)
    }

    pub fn opposite(self) -> CubeRegion {
        match self {
            CubeRegion::Face(FACE_TOP) => CubeRegion::Face(FACE_BOTTOM),
            CubeRegion::Face(FACE_BOTTOM) => CubeRegion::Face(FACE_TOP),
            CubeRegion::Face(FACE_FRONT) => CubeRegion::Face(FACE_BACK),
            CubeRegion::Face(FACE_BACK) => CubeRegion::Face(FACE_FRONT),
            CubeRegion::Face(FACE_RIGHT) => CubeRegion::Face(FACE_LEFT),
            CubeRegion::Face(FACE_LEFT) => CubeRegion::Face(FACE_RIGHT),
            other => other,
        }
    }

    pub fn label(self) -> &'static str {
        match self.id() {
            0 => "TOP",
            1 => "BOTTOM",
            2 => "FRONT",
            3 => "BACK",
            4 => "RIGHT",
            5 => "LEFT",
            6 => "Top Front",
            7 => "Top Back",
            8 => "Top Right",
            9 => "Top Left",
            10 => "Bot Front",
            11 => "Bot Back",
            12 => "Bot Right",
            13 => "Bot Left",
            14 => "Front Right",
            15 => "Front Left",
            16 => "Back Right",
            17 => "Back Left",
            18 => "Top Front Right",
            19 => "Top Front Left",
            20 => "Top Back Right",
            21 => "Top Back Left",
            22 => "Bot Front Right",
            23 => "Bot Front Left",
            24 => "Bot Back Right",
            25 => "Bot Back Left",
            _ => "?",
        }
    }
}

// ── Vertex ────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct CubeVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    pub region_f: f32,
}

impl CubeVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
        0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32,
    ];
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct CubeUniforms {
    pub view_proj: [f32; 16],
    pub rotation: [f32; 16],
    pub hover_region: [f32; 4],
}

impl CubeUniforms {
    /// Build uniforms from the camera quaternion-derived rotation matrix.
    /// `cam_rotation` = `Mat4::from_quat(camera.rotation)`.
    pub fn new(
        cam_rotation: Mat4,
        cube_px: u32,
        vp_w: u32,
        vp_h: u32,
        hover: Option<usize>,
    ) -> Self {
        let (hw, hh) = (vp_w as f32 * 0.5, vp_h as f32 * 0.5);
        let cube_half = cube_px as f32 * VIEWCUBE_SCALE;
        // Inset the cube centre to leave room for the ring + controls.
        let inset = cube_half * NAV_INSET_F;
        let cx = hw - inset - VIEWCUBE_PAD;
        let cy = hh - inset - VIEWCUBE_PAD;
        let view_proj = orthographic(-hw, hw, -hh, hh, -2000.0, 2000.0)
            * Mat4::from_translation(Vec3::new(cx, cy, 0.0))
            * Mat4::from_scale(Vec3::splat(cube_px as f32 * VIEWCUBE_SCALE));
        Self {
            view_proj: view_proj.to_cols_array(),
            rotation: cam_rotation.to_cols_array(),
            hover_region: [
                hover.map(|h| h as f32 / 25.0).unwrap_or(-1.0),
                0.0,
                0.0,
                0.0,
            ],
        }
    }
}

// ── Bitmap text ───────────────────────────────────────────────────────────
//
// Atlas grid: 8 × 3 cells of 16×18 px (128×54 texture). Each cell holds one
// glyph, centered. Latin label glyphs are the original 5×7 bitmaps; the six
// Simplified-Chinese face labels are 14×16 bitmaps traced from the system's
// Noto Sans CJK font (see the `han_rows` table below), which keeps the shapes
// correct instead of hand-drawn approximations.

const CELL_W: usize = 16;
const CELL_H: usize = 18;
const ATLAS_COLS: usize = 8;
const ATLAS_ROWS: usize = 3;
// Latin label glyphs: 5×7 bitmaps centered in their cell.
const LATIN_W: usize = 5;
const LATIN_H: usize = 7;
const LATIN_OX: usize = (CELL_W - LATIN_W) / 2;
const LATIN_OY: usize = (CELL_H - LATIN_H) / 2;
// Han label glyphs: 14×16 font bitmaps centered in their cell.
const HAN_W: usize = 14;
const HAN_H: usize = 16;
const HAN_OX: usize = (CELL_W - HAN_W) / 2;
const HAN_OY: usize = (CELL_H - HAN_H) / 2;
const MAX_LABEL_CHARS: usize = 6;
const LABEL_COUNT: usize = 6;
// Face labels + the four compass cardinals (one glyph each).
const MAX_GLYPHS: usize = MAX_LABEL_CHARS * LABEL_COUNT + 4;
const MAX_VERTS: usize = MAX_GLYPHS * 6;

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct TextUniforms {
    screen: [f32; 2],
    _pad: [f32; 2],
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct TextVertex {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

impl TextVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![
        0 => Float32x2, 1 => Float32x2, 2 => Float32x4,
    ];
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

fn glyph_index(c: char) -> Option<usize> {
    match c {
        'A' => Some(0),
        'B' => Some(1),
        'C' => Some(2),
        'E' => Some(3),
        'F' => Some(4),
        'G' => Some(5),
        'H' => Some(6),
        'I' => Some(7),
        'K' => Some(8),
        'L' => Some(9),
        'M' => Some(10),
        'N' => Some(11),
        'O' => Some(12),
        'P' => Some(13),
        'R' => Some(14),
        'T' => Some(15),
        'S' => Some(16),
        'W' => Some(17),
        // Simplified-Chinese face labels (atlas cells 18..24).
        '上' => Some(18),
        '下' => Some(19),
        '前' => Some(20),
        '后' => Some(21),
        '右' => Some(22),
        '左' => Some(23),
        _ => None,
    }
}

/// 5×7 Latin glyph rows (existing shapes, unchanged) for the English labels
/// and compass cardinals. `None` for any other character.
fn latin_rows(c: char) -> Option<[u8; 7]> {
    match c {
        'A' => Some([
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ]),
        'B' => Some([
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ]),
        'C' => Some([
            0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111,
        ]),
        'E' => Some([
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ]),
        'F' => Some([
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ]),
        'G' => Some([
            0b01111, 0b10000, 0b10000, 0b10011, 0b10001, 0b10001, 0b01111,
        ]),
        'H' => Some([
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ]),
        'I' => Some([
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ]),
        'K' => Some([
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ]),
        'L' => Some([
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ]),
        'M' => Some([
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ]),
        'N' => Some([
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ]),
        'O' => Some([
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ]),
        'P' => Some([
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ]),
        'R' => Some([
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ]),
        'T' => Some([
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ]),
        'S' => Some([
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ]),
        'W' => Some([
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ]),
        _ => None,
    }
}

/// 14×16 Han glyph rows for the Simplified-Chinese face labels (上/下/前/后/右/左),
/// traced from Noto Sans CJK Bold (system font) so the stroke shapes are the
/// real printed forms rather than hand-drawn approximations. Each row is a
/// 14-bit mask, bit 13 = leftmost pixel. `None` for any other character.
fn han_rows(c: char) -> Option<[u16; HAN_H]> {
    match c {
        '上' => Some([
            0b00000000000000, 0b00000000000000, 0b00000110000000, 0b00000110000000,
            0b00000110000000, 0b00000110000000, 0b00000111111100, 0b00000111111100,
            0b00000110000000, 0b00000110000000, 0b00000110000000, 0b00000110000000,
            0b11111111111110, 0b11111111111110, 0b00000000000000, 0b00000000000000,
        ]),
        '下' => Some([
            0b00000000000000, 0b11111111111110, 0b11111111111110, 0b00000110000000,
            0b00000110000000, 0b00000111100000, 0b00000111110000, 0b00000110111100,
            0b00000110001100, 0b00000110000000, 0b00000110000000, 0b00000110000000,
            0b00000110000000, 0b00000110000000, 0b00000000000000, 0b00000000000000,
        ]),
        '前' => Some([
            0b00000000000000, 0b00010000010000, 0b00011000011000, 0b11111111111110,
            0b11111111111110, 0b00000000000100, 0b01111100101100, 0b01100100101100,
            0b01111100101100, 0b01100100101100, 0b01111100101100, 0b01100110001100,
            0b01100110011100, 0b01101100011100, 0b00000000000000, 0b00000000000000,
        ]),
        '后' => Some([
            0b00000000000000, 0b00000000111000, 0b00111111111100, 0b01111000000000,
            0b00100000000000, 0b00111111111110, 0b01111111111100, 0b01100000000000,
            0b01101111111100, 0b01101111111100, 0b01101100001100, 0b01101100001100,
            0b11001111111100, 0b11001100001100, 0b00000000000000, 0b00000000000000,
        ]),
        '右' => Some([
            0b00000000000000, 0b00000100000000, 0b00000110000000, 0b00001110000000,
            0b11111111111110, 0b00011100000000, 0b00011000000000, 0b00011000000000,
            0b00111111111100, 0b01111000001100, 0b11111000001100, 0b01011000001100,
            0b00011111111100, 0b00011111111100, 0b00011000001100, 0b00000000000000,
        ]),
        '左' => Some([
            0b00000000000000, 0b00001100000000, 0b00001100000000, 0b01111111111100,
            0b11111111111110, 0b00011000000000, 0b00011000000000, 0b00011111111100,
            0b00111111111100, 0b00110001100000, 0b01100001100000, 0b01100001100000,
            0b11011111111110, 0b00011111111110, 0b00000000000000, 0b00000000000000,
        ]),
        _ => None,
    }
}

fn build_atlas() -> (Vec<u8>, u32, u32) {
    let w = (ATLAS_COLS * CELL_W) as u32;
    let h = (ATLAS_ROWS * CELL_H) as u32;
    let mut data = vec![0u8; (w * h) as usize];
    let glyphs = [
        'A', 'B', 'C', 'E', 'F', 'G', 'H', 'I', 'K', 'L', 'M', 'N', 'O', 'P', 'R', 'T', 'S', 'W',
        '上', '下', '前', '后', '右', '左',
    ];
    for (i, &ch) in glyphs.iter().enumerate() {
        let col = i % ATLAS_COLS;
        let row = i / ATLAS_COLS;
        let x0 = col * CELL_W;
        let y0 = row * CELL_H;
        if let Some(rows) = latin_rows(ch) {
            for y in 0..LATIN_H {
                let bits = rows[y];
                for x in 0..LATIN_W {
                    if (bits >> (LATIN_W - 1 - x)) & 1 == 0 {
                        continue;
                    }
                    data[(y0 + y + LATIN_OY) as usize * w as usize + (x0 + x + LATIN_OX)] = 255;
                }
            }
        } else if let Some(rows) = han_rows(ch) {
            for y in 0..HAN_H {
                let bits = rows[y];
                for x in 0..HAN_W {
                    if (bits >> (HAN_W - 1 - x)) & 1 == 0 {
                        continue;
                    }
                    data[(y0 + y + HAN_OY) as usize * w as usize + (x0 + x + HAN_OX)] = 255;
                }
            }
        }
    }
    (data, w, h)
}

fn glyph_uv(index: usize, atlas_w: f32, atlas_h: f32) -> (f32, f32, f32, f32) {
    let col = index % ATLAS_COLS;
    let row = index / ATLAS_COLS;
    let (ox, oy, gw, gh) = if index < 18 {
        (LATIN_OX, LATIN_OY, LATIN_W, LATIN_H)
    } else {
        (HAN_OX, HAN_OY, HAN_W, HAN_H)
    };
    let x0 = (col * CELL_W + ox) as f32;
    let y0 = (row * CELL_H + oy) as f32;
    (
        x0 / atlas_w,
        y0 / atlas_h,
        (x0 + gw as f32) / atlas_w,
        (y0 + gh as f32) / atlas_h,
    )
}

struct ViewCubeText {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u32,
    vertex_count: u32,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    atlas_w: f32,
    atlas_h: f32,
}

impl ViewCubeText {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let (atlas, w, h) = build_atlas();
        let atlas_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vc.text_atlas"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let bytes_per_row = w;
        let aligned_bpr = ((bytes_per_row + 255) / 256) * 256;
        let atlas_bytes = if aligned_bpr == bytes_per_row {
            atlas
        } else {
            let mut padded = vec![0u8; (aligned_bpr * h) as usize];
            for row in 0..h as usize {
                let src = row * bytes_per_row as usize;
                let dst = row * aligned_bpr as usize;
                padded[dst..dst + bytes_per_row as usize]
                    .copy_from_slice(&atlas[src..src + bytes_per_row as usize]);
            }
            padded
        };
        queue.write_texture(
            atlas_tex.as_image_copy(),
            &atlas_bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(aligned_bpr),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let atlas_view = atlas_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vc.text_sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vc.text_uniform"),
            size: std::mem::size_of::<TextUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vc.text_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vc.text_bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&atlas_view),
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vc.text_layout"),
            bind_group_layouts: &[&bgl].map(Some),
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vc.text_shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "../../shaders/viewcube_text.wgsl"
            ))),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vc.text_pipe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[TextVertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertex_capacity = MAX_VERTS as u32;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vc.text_vb"),
            size: (vertex_capacity as usize * std::mem::size_of::<TextVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            vertex_buffer,
            vertex_capacity,
            vertex_count: 0,
            uniform_buffer,
            bind_group,
            atlas_w: w as f32,
            atlas_h: h as f32,
        }
    }

    /// Update text labels using the quaternion-derived rotation matrix.
    fn update(
        &mut self,
        queue: &wgpu::Queue,
        cam_rotation: Mat4,
        compass_rotation: Mat4,
        vp_w: u32,
        vp_h: u32,
        cube_px: u32,
        text_color: [f32; 4],
    ) {
        let (vw, vh) = (vp_w as f32, vp_h as f32);
        let cube_half = cube_px as f32 * VIEWCUBE_SCALE;
        let inset = cube_half * NAV_INSET_F;
        let (hw, hh) = (vw * 0.5, vh * 0.5);
        let view_proj = orthographic(-hw, hw, -hh, hh, -2000.0, 2000.0)
            * Mat4::from_translation(Vec3::new(
                hw - inset - VIEWCUBE_PAD,
                hh - inset - VIEWCUBE_PAD,
                0.0,
            ))
            * Mat4::from_scale(Vec3::splat(cube_px as f32 * VIEWCUBE_SCALE));

        // Glyph size in cube-local units. Letters are screen-aligned per face
        // (see the axis projection below), so they read upright on every face
        // as the cube rotates instead of turning with the face's local axes.
        const GW: f32 = 0.17; // glyph width
        const GH: f32 = 0.24; // glyph height
        const ADV: f32 = 0.22; // pen advance
        // Local directions that map onto the screen's up / right once rotated
        // by `cam_rotation`. Used to project label axes onto each face plane.
        let inv_rot = cam_rotation.inverse();
        let screen_up_local = inv_rot.transform_vector3(Vec3::Y);
        let screen_right_local = inv_rot.transform_vector3(Vec3::X);
        let mut verts: Vec<TextVertex> = Vec::with_capacity(MAX_VERTS);
        let view_dir = Vec3::Z;
        let labels = face_labels();
        // Local cube point → screen pixel.
        let project = |local: Vec3| -> Option<[f32; 2]> {
            let world = cam_rotation.transform_point3(local);
            let clip = view_proj * Vec4::new(world.x, world.y, world.z, 1.0);
            if clip.w.abs() < 1e-6 {
                return None;
            }
            Some([
                (clip.x / clip.w + 1.0) * 0.5 * vw,
                (1.0 - clip.y / clip.w) * 0.5 * vh,
            ])
        };

        for (fi, &c) in FACE_CENTERS.iter().enumerate() {
            let face_n = Vec3::from(c);
            let world_n = cam_rotation.transform_vector3(face_n).normalize();
            let dot = world_n.dot(view_dir);
            if dot < 0.12 {
                continue;
            }
            let alpha = ((dot - 0.12) / 0.88).clamp(0.0, 1.0);
            let color = [
                text_color[0],
                text_color[1],
                text_color[2],
                text_color[3] * alpha,
            ];
            // Screen-aligned label axes: project the screen's up direction onto
            // the face plane so the text reads upright (never rotated with the
            // face) on every visible face. `u` is then completed in-plane and
            // flipped toward screen-right so letters read left→right.
            let n = face_n;
            let mut v = screen_up_local - n * screen_up_local.dot(n);
            if v.length_squared() < 1e-6 {
                // Face normal is (anti-)parallel to screen-up, so the projected
                // up vanishes; project screen-right instead and rebuild the frame.
                let mut u = screen_right_local - n * screen_right_local.dot(n);
                if u.length_squared() < 1e-6 {
                    u = Vec3::Z - n * Vec3::Z.dot(n);
                }
                u = u.normalize();
                v = n.cross(u);
            } else {
                v = v.normalize();
            }
            let mut u = n.cross(v);
            if u.dot(screen_right_local) < 0.0 {
                u = -u;
            }
            let center = face_n; // unit normal = face surface centre (distance E)

            let label = &labels[fi];
            // Han labels are single characters; render them ~2.6× larger so the
            // 14×16 font bitmap stays ~1:1 and crisp. Multi-letter Latin labels
            // keep the compact size so "BOTTOM" still fits the face.
            let is_han = label.chars().any(|c| ('\u{2E80}'..='\u{9FFF}').contains(&c));
            let (gw, gh, adv) = if is_han {
                (0.45, 0.52, 0.45)
            } else {
                (GW, GH, ADV)
            };
            let total_w = label.chars().count() as f32 * adv;
            let mut pen = -total_w * 0.5;
            for ch in label.chars() {
                let Some(gi) = glyph_index(ch) else {
                    pen += adv;
                    continue;
                };
                let (u0, v0, u1, v1) = glyph_uv(gi, self.atlas_w, self.atlas_h);
                // Glyph quad corners on the face plane, then projected.
                let corner = |lx: f32, ly: f32| center + u * lx + v * ly;
                let tl = project(corner(pen, gh * 0.5));
                let tr = project(corner(pen + gw, gh * 0.5));
                let br = project(corner(pen + gw, -gh * 0.5));
                let bl = project(corner(pen, -gh * 0.5));
                if let (Some(tl), Some(tr), Some(br), Some(bl)) = (tl, tr, br, bl) {
                    let mk = |pos: [f32; 2], uv: [f32; 2]| TextVertex { pos, uv, color };
                    verts.push(mk(tl, [u0, v0]));
                    verts.push(mk(tr, [u1, v0]));
                    verts.push(mk(br, [u1, v1]));
                    verts.push(mk(tl, [u0, v0]));
                    verts.push(mk(br, [u1, v1]));
                    verts.push(mk(bl, [u0, v1]));
                }
                pen += adv;
                if verts.len() >= self.vertex_capacity as usize {
                    break;
                }
            }
            if verts.len() >= self.vertex_capacity as usize {
                break;
            }
        }

        // ── Compass cardinals (N / E / S / W) ──────────────────────────────
        // Painted flat onto the ring band (z = RING_Z) so they foreshorten with
        // the ring and read upright in plan. Local axes u = +X, v = +Y give
        // u × v = +Z → never mirrored when seen from above.
        //
        // The compass is world-fixed: it projects through `compass_rotation`
        // (camera only, no UCS), so N/E/S/W stay aligned to world even when the
        // cube reorients with the active UCS.
        let project_world = |local: Vec3| -> Option<[f32; 2]> {
            let world = compass_rotation.transform_point3(local);
            let clip = view_proj * Vec4::new(world.x, world.y, world.z, 1.0);
            if clip.w.abs() < 1e-6 {
                return None;
            }
            Some([
                (clip.x / clip.w + 1.0) * 0.5 * vw,
                (1.0 - clip.y / clip.w) * 0.5 * vh,
            ])
        };
        const CARD_GW: f32 = 0.16; // glyph size in cube-local units
        const CARD_GH: f32 = 0.22;
        let cardinals = [
            ('N', Vec3::new(0.0, 1.0, 0.0)),
            ('E', Vec3::new(1.0, 0.0, 0.0)),
            ('S', Vec3::new(0.0, -1.0, 0.0)),
            ('W', Vec3::new(-1.0, 0.0, 0.0)),
        ];
        for (ch, dir) in cardinals {
            let Some(gi) = glyph_index(ch) else {
                continue;
            };
            let center = Vec3::new(dir.x * R_CARD, dir.y * R_CARD, RING_Z);
            // Dim a cardinal whose ring point sits behind the cube.
            let alpha = if compass_rotation.transform_point3(center).z >= -0.15 {
                1.0
            } else {
                0.5
            };
            let color = [
                text_color[0],
                text_color[1],
                text_color[2],
                text_color[3] * alpha,
            ];
            let (u0, v0, u1, v1) = glyph_uv(gi, self.atlas_w, self.atlas_h);
            let corner = |lx: f32, ly: f32| center + Vec3::X * lx + Vec3::Y * ly;
            let tl = project_world(corner(-CARD_GW * 0.5, CARD_GH * 0.5));
            let tr = project_world(corner(CARD_GW * 0.5, CARD_GH * 0.5));
            let br = project_world(corner(CARD_GW * 0.5, -CARD_GH * 0.5));
            let bl = project_world(corner(-CARD_GW * 0.5, -CARD_GH * 0.5));
            if let (Some(tl), Some(tr), Some(br), Some(bl)) = (tl, tr, br, bl) {
                let mk = |pos: [f32; 2], uv: [f32; 2]| TextVertex { pos, uv, color };
                verts.push(mk(tl, [u0, v0]));
                verts.push(mk(tr, [u1, v0]));
                verts.push(mk(br, [u1, v1]));
                verts.push(mk(tl, [u0, v0]));
                verts.push(mk(br, [u1, v1]));
                verts.push(mk(bl, [u0, v1]));
            }
            if verts.len() >= self.vertex_capacity as usize {
                break;
            }
        }
        self.vertex_count = verts.len() as u32;
        if self.vertex_count > 0 {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));
        }
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&TextUniforms {
                screen: [vw, vh],
                _pad: [0.0; 2],
            }),
        );
    }

    fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: Rectangle<u32>,
    ) {
        if self.vertex_count == 0 {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vc.text_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            clip.x as f32,
            clip.y as f32,
            clip.width as f32,
            clip.height as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.draw(0..self.vertex_count, 0..1);
    }
}

// ── Geometry ──────────────────────────────────────────────────────────────

const F: f32 = 0.80;
const E: f32 = 1.00;
const C_TOP: [f32; 3] = [0.70, 0.80, 0.94];
const C_BOTTOM: [f32; 3] = [0.32, 0.32, 0.36];
const C_FRONT: [f32; 3] = [0.80, 0.83, 0.90];
const C_BACK: [f32; 3] = [0.46, 0.47, 0.52];
const C_RIGHT: [f32; 3] = [0.62, 0.60, 0.56];
const C_LEFT: [f32; 3] = [0.54, 0.55, 0.64];
const C_EDGE: [f32; 3] = [0.24, 0.25, 0.28];
const C_CORNER: [f32; 3] = [0.16, 0.17, 0.19];

fn push_quad(
    corners: [[f32; 3]; 4],
    rgb: [f32; 3],
    region: usize,
    vs: &mut Vec<CubeVertex>,
    is: &mut Vec<u32>,
) {
    let mut cs = corners;
    let center = {
        let s = Vec3::from(cs[0]) + Vec3::from(cs[1]) + Vec3::from(cs[2]) + Vec3::from(cs[3]);
        (s * 0.25).normalize_or_zero()
    };
    let mut n = (Vec3::from(cs[1]) - Vec3::from(cs[0]))
        .cross(Vec3::from(cs[3]) - Vec3::from(cs[0]))
        .normalize_or_zero();
    if n.dot(center) < 0.0 {
        cs = [cs[0], cs[3], cs[2], cs[1]];
        n = (Vec3::from(cs[1]) - Vec3::from(cs[0]))
            .cross(Vec3::from(cs[3]) - Vec3::from(cs[0]))
            .normalize_or_zero();
    }
    let n = n.to_array();
    let rf = region as f32 / 25.0;
    let base = vs.len() as u32;
    for pos in cs {
        vs.push(CubeVertex {
            pos,
            normal: n,
            color: rgb,
            region_f: rf,
        });
    }
    is.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn push_tri(
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    rgb: [f32; 3],
    region: usize,
    vs: &mut Vec<CubeVertex>,
    is: &mut Vec<u32>,
) {
    let mut b = b;
    let mut c = c;
    let center = {
        let s = Vec3::from(a) + Vec3::from(b) + Vec3::from(c);
        (s / 3.0).normalize_or_zero()
    };
    let mut n = (Vec3::from(b) - Vec3::from(a))
        .cross(Vec3::from(c) - Vec3::from(a))
        .normalize_or_zero();
    if n.dot(center) < 0.0 {
        std::mem::swap(&mut b, &mut c);
        n = (Vec3::from(b) - Vec3::from(a))
            .cross(Vec3::from(c) - Vec3::from(a))
            .normalize_or_zero();
    }
    let n = n.to_array();
    let rf = region as f32 / 25.0;
    let base = vs.len() as u32;
    for pos in [a, b, c] {
        vs.push(CubeVertex {
            pos,
            normal: n,
            color: rgb,
            region_f: rf,
        });
    }
    is.extend_from_slice(&[base, base + 1, base + 2]);
}

pub fn build_geometry() -> (Vec<CubeVertex>, Vec<u32>) {
    let (mut vs, mut is) = (Vec::<CubeVertex>::new(), Vec::<u32>::new());
    push_quad(
        [[-F, -F, E], [F, -F, E], [F, F, E], [-F, F, E]],
        C_TOP,
        FACE_TOP,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, F, -E], [F, F, -E], [F, -F, -E], [-F, -F, -E]],
        C_BOTTOM,
        FACE_BOTTOM,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[F, -E, -F], [-F, -E, -F], [-F, -E, F], [F, -E, F]],
        C_FRONT,
        FACE_FRONT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, E, -F], [F, E, -F], [F, E, F], [-F, E, F]],
        C_BACK,
        FACE_BACK,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[E, F, -F], [E, -F, -F], [E, -F, F], [E, F, F]],
        C_RIGHT,
        FACE_RIGHT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-E, -F, -F], [-E, F, -F], [-E, F, F], [-E, -F, F]],
        C_LEFT,
        FACE_LEFT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[F, -F, E], [-F, -F, E], [-F, -E, F], [F, -E, F]],
        C_EDGE,
        EDGE_TOP_FRONT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, F, E], [F, F, E], [F, E, F], [-F, E, F]],
        C_EDGE,
        EDGE_TOP_BACK,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[F, F, E], [F, -F, E], [E, -F, F], [E, F, F]],
        C_EDGE,
        EDGE_TOP_RIGHT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, -F, E], [-F, F, E], [-E, F, F], [-E, -F, F]],
        C_EDGE,
        EDGE_TOP_LEFT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[F, -F, -E], [-F, -F, -E], [-F, -E, -F], [F, -E, -F]],
        C_EDGE,
        EDGE_BOT_FRONT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, F, -E], [F, F, -E], [F, E, -F], [-F, E, -F]],
        C_EDGE,
        EDGE_BOT_BACK,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[F, F, -E], [F, -F, -E], [E, -F, -F], [E, F, -F]],
        C_EDGE,
        EDGE_BOT_RIGHT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, -F, -E], [-F, F, -E], [-E, F, -F], [-E, -F, -F]],
        C_EDGE,
        EDGE_BOT_LEFT,
        &mut vs,
        &mut is,
    );
    // Side edges: diagonal chamfer strips connecting vertical face pairs.
    // Each strip spans from one face edge to the adjacent face edge — not flat in one plane.
    push_quad(
        [[F, -E, -F], [F, -E, F], [E, -F, F], [E, -F, -F]],
        C_EDGE,
        EDGE_FRONT_RIGHT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, -E, F], [-F, -E, -F], [-E, -F, -F], [-E, -F, F]],
        C_EDGE,
        EDGE_FRONT_LEFT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[F, E, F], [F, E, -F], [E, F, -F], [E, F, F]],
        C_EDGE,
        EDGE_BACK_RIGHT,
        &mut vs,
        &mut is,
    );
    push_quad(
        [[-F, E, F], [-F, E, -F], [-E, F, -F], [-E, F, F]],
        C_EDGE,
        EDGE_BACK_LEFT,
        &mut vs,
        &mut is,
    );
    for &([sx, sy, sz], region) in &[
        ([1.0f32, 1.0, 1.0], CORNER_TBK_R), // sy=+1 → BACK direction
        ([-1.0, 1.0, 1.0], CORNER_TBK_L),
        ([1.0, 1.0, -1.0], CORNER_BBK_R),
        ([-1.0, 1.0, -1.0], CORNER_BBK_L),
        ([1.0, -1.0, 1.0], CORNER_TPF_R), // sy=-1 → FRONT direction
        ([-1.0, -1.0, 1.0], CORNER_TPF_L),
        ([1.0, -1.0, -1.0], CORNER_BTF_R),
        ([-1.0, -1.0, -1.0], CORNER_BTF_L),
    ] {
        push_tri(
            [sx * F, sy * F, sz * E],
            [sx * F, sy * E, sz * F],
            [sx * E, sy * F, sz * F],
            C_CORNER,
            region,
            &mut vs,
            &mut is,
        );
    }
    build_ring(&mut vs, &mut is);
    (vs, is)
}

/// A flat compass ring in the cube's local XY plane (the ground plane),
/// surrounding the cube. Pushed with a sentinel `region_f = -1.0` so the
/// shader never highlights it on hover, and a constant grey colour.
fn build_ring(vs: &mut Vec<CubeVertex>, is: &mut Vec<u32>) {
    const SEG: usize = 64;
    const R0: f32 = 1.40; // inner radius — clear gap to the cube faces
    const R1: f32 = 1.74; // outer radius — wider, thicker band
    const RING_RGB: [f32; 3] = [0.22, 0.23, 0.26];
    for s in 0..SEG {
        let a0 = s as f32 / SEG as f32 * std::f32::consts::TAU;
        let a1 = (s + 1) as f32 / SEG as f32 * std::f32::consts::TAU;
        let (c0, s0) = (a0.cos(), a0.sin());
        let (c1, s1) = (a1.cos(), a1.sin());
        let quad = [
            [c0 * R0, s0 * R0, RING_Z],
            [c1 * R0, s1 * R0, RING_Z],
            [c1 * R1, s1 * R1, RING_Z],
            [c0 * R1, s0 * R1, RING_Z],
        ];
        let base = vs.len() as u32;
        for pos in quad {
            vs.push(CubeVertex {
                pos,
                normal: [0.0, 0.0, 1.0],
                color: RING_RGB,
                region_f: -1.0,
            });
        }
        is.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

pub fn region_centroids() -> [[f32; 3]; NUM_REGIONS] {
    let m = (F + E) * 0.5;
    [
        [0.0, 0.0, E],  // FACE_TOP
        [0.0, 0.0, -E], // FACE_BOTTOM
        [0.0, -E, 0.0], // FACE_FRONT  (geometry Y=-E)
        [0.0, E, 0.0],  // FACE_BACK   (geometry Y=+E)
        [E, 0.0, 0.0],  // FACE_RIGHT
        [-E, 0.0, 0.0], // FACE_LEFT
        [0.0, -m, m],   // EDGE_TOP_FRONT
        [0.0, m, m],    // EDGE_TOP_BACK
        [m, 0.0, m],    // EDGE_TOP_RIGHT
        [-m, 0.0, m],   // EDGE_TOP_LEFT
        [0.0, -m, -m],  // EDGE_BOT_FRONT
        [0.0, m, -m],   // EDGE_BOT_BACK
        [m, 0.0, -m],   // EDGE_BOT_RIGHT
        [-m, 0.0, -m],  // EDGE_BOT_LEFT
        [m, -m, 0.0],   // EDGE_FRONT_RIGHT
        [-m, -m, 0.0],  // EDGE_FRONT_LEFT
        [m, m, 0.0],    // EDGE_BACK_RIGHT
        [-m, m, 0.0],   // EDGE_BACK_LEFT
        [m, -m, m],     // CORNER_TPF_R  (geometry sy=-1 = FRONT)
        [-m, -m, m],    // CORNER_TPF_L
        [m, m, m],      // CORNER_TBK_R  (geometry sy=+1 = BACK)
        [-m, m, m],     // CORNER_TBK_L
        [m, -m, -m],    // CORNER_BTF_R  (geometry sy=-1 = FRONT)
        [-m, -m, -m],   // CORNER_BTF_L
        [m, m, -m],     // CORNER_BBK_R  (geometry sy=+1 = BACK)
        [-m, m, -m],    // CORNER_BBK_L
    ]
}

fn threshold_sq(id: usize, cube_half_px: f32) -> f32 {
    let r = if id < 6 {
        cube_half_px * 0.92
    } else if id < 18 {
        cube_half_px * 0.38
    } else {
        cube_half_px * 0.28
    };
    r * r
}

// ── Pipeline ──────────────────────────────────────────────────────────────

pub struct ViewCubePipeline {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    depth_texture_size: Size<u32>,
    depth_view: wgpu::TextureView,
    pub cube_px: u32,
    text: ViewCubeText,
}

impl ViewCubePipeline {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        use wgpu::util::DeviceExt;
        let (verts, idxs) = build_geometry();
        let cube_px = VIEWCUBE_PX;
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vc.vb"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vc.ib"),
            contents: bytemuck::cast_slice(&idxs),
            usage: wgpu::BufferUsages::INDEX,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vc.ub"),
            size: std::mem::size_of::<CubeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vc.bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vc.bg"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vc.layout"),
            bind_group_layouts: &[&bgl].map(Some),
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vc.shader"),
            source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(include_str!(
                "../../shaders/viewcube.wgsl"
            ))),
        });
        let depth_tex = create_depth_texture(device, Size::new(1, 1));
        let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vc.pipe"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[CubeVertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let text = ViewCubeText::new(device, queue, format);
        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: idxs.len() as u32,
            uniform_buffer,
            uniform_bind_group,
            depth_texture_size: Size::new(1, 1),
            depth_view,
            cube_px,
            text,
        }
    }

    /// Upload using the quaternion rotation matrix.
    /// `cam_rotation` = `camera.view_rotation_mat()` = `Mat4::from_quat(camera.rotation)`.
    pub fn upload(
        &mut self,
        queue: &wgpu::Queue,
        cam_rotation: Mat4,
        compass_rotation: Mat4,
        vp_w: u32,
        vp_h: u32,
        hover: Option<usize>,
        text_color: [f32; 4],
    ) {
        queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::bytes_of(&CubeUniforms::new(
                cam_rotation,
                self.cube_px,
                vp_w,
                vp_h,
                hover,
            )),
        );
        self.text
            .update(
                queue,
                cam_rotation,
                compass_rotation,
                vp_w,
                vp_h,
                self.cube_px,
                text_color,
            );
    }

    pub fn ensure_depth_texture(&mut self, device: &wgpu::Device, size: Size<u32>) {
        if self.depth_texture_size != size {
            let tex = create_depth_texture(device, size);
            self.depth_view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.depth_texture_size = size;
        }
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        clip: Rectangle<u32>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("vc.pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            clip.x as f32,
            clip.y as f32,
            clip.width as f32,
            clip.height as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
        drop(pass);
        self.text.render(encoder, target, clip);
    }
}

impl iced::widget::shader::Pipeline for ViewCubePipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self::new(device, queue, format)
    }
}

fn create_depth_texture(device: &wgpu::Device, size: Size<u32>) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vc.depth_texture"),
        size: wgpu::Extent3d {
            width: size.width.max(1),
            height: size.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24PlusStencil8,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}

// ── Hit test ──────────────────────────────────────────────────────────────
//
// hit_test and hover_id now take cam_rotation: Mat4 (same matrix the shader
// uses) so click regions always match what is drawn — including after free
// arcball orbit from any angle.

/// Returns the ViewCube region under screen position (mx, my), or None.
/// `cam_rotation` must be `camera.view_rotation_mat()`.
pub fn hit_test(
    mx: f32,
    my: f32,
    vp_w: f32,
    vp_h: f32,
    cam_rotation: Mat4,
    cube_px: u32,
) -> Option<CubeRegion> {
    let half = cube_px as f32 * VIEWCUBE_SCALE;
    let inset = half * NAV_INSET_F;
    let cx = vp_w - inset - VIEWCUBE_PAD;
    let cy = inset + VIEWCUBE_PAD;
    if (mx - cx).abs() > half || (my - cy).abs() > half {
        return None;
    }

    let (hw, hh) = (vp_w * 0.5, vp_h * 0.5);
    let vp = orthographic(-hw, hw, -hh, hh, -2000.0, 2000.0)
        * Mat4::from_translation(Vec3::new(
            hw - inset - VIEWCUBE_PAD,
            hh - inset - VIEWCUBE_PAD,
            0.0,
        ))
        * Mat4::from_scale(Vec3::splat(cube_px as f32 * VIEWCUBE_SCALE));

    let view_dir = Vec3::Z;
    let centroids = region_centroids();
    let (mut best, mut best_d) = (None, f32::MAX);

    for (id, &c) in centroids.iter().enumerate() {
        let world = cam_rotation.transform_point3(Vec3::from(c));
        if world.normalize().dot(view_dir) < 0.05 {
            continue;
        }
        let clip = vp * Vec4::new(world.x, world.y, world.z, 1.0);
        if clip.w.abs() < 1e-6 {
            continue;
        }
        let sx = (clip.x / clip.w + 1.0) * 0.5 * vp_w;
        let sy = (1.0 - clip.y / clip.w) * 0.5 * vp_h;
        let d = (sx - mx).powi(2) + (sy - my).powi(2);
        if d < threshold_sq(id, half) && d < best_d {
            best_d = d;
            best = Some(if id < 6 {
                CubeRegion::Face(id)
            } else if id < 18 {
                CubeRegion::Edge(id)
            } else {
                CubeRegion::Corner(id)
            });
        }
    }
    best
}

/// Returns the hovered region id (0-25), or None.
pub fn hover_id(
    mx: f32,
    my: f32,
    vp_w: f32,
    vp_h: f32,
    cam_rotation: Mat4,
    cube_px: u32,
) -> Option<usize> {
    hit_test(mx, my, vp_w, vp_h, cam_rotation, cube_px).map(|r| r.id())
}

impl Cardinal {
    /// The side-elevation face a compass letter snaps to when clicked.
    pub fn face_region(self) -> CubeRegion {
        CubeRegion::Face(match self {
            Cardinal::North => FACE_BACK,
            Cardinal::South => FACE_FRONT,
            Cardinal::East => FACE_RIGHT,
            Cardinal::West => FACE_LEFT,
        })
    }
}

/// Returns the compass cardinal under (mx, my), or None. Projects each of the
/// four ring letters and accepts the nearest within a small pixel radius — the
/// caller tries the cube body first, so this only fires out on the ring.
pub fn hit_test_cardinal(
    mx: f32,
    my: f32,
    vp_w: f32,
    vp_h: f32,
    cam_rotation: Mat4,
    cube_px: u32,
) -> Option<Cardinal> {
    let half = cube_px as f32 * VIEWCUBE_SCALE;
    let inset = half * NAV_INSET_F;
    let cx = vp_w - inset - VIEWCUBE_PAD;
    let cy = inset + VIEWCUBE_PAD;
    if (mx - cx).abs() > inset || (my - cy).abs() > inset {
        return None;
    }

    let (hw, hh) = (vp_w * 0.5, vp_h * 0.5);
    let vp = orthographic(-hw, hw, -hh, hh, -2000.0, 2000.0)
        * Mat4::from_translation(Vec3::new(
            hw - inset - VIEWCUBE_PAD,
            hh - inset - VIEWCUBE_PAD,
            0.0,
        ))
        * Mat4::from_scale(Vec3::splat(cube_px as f32 * VIEWCUBE_SCALE));

    let dirs = [
        (Cardinal::North, Vec3::new(0.0, 1.0, 0.0)),
        (Cardinal::East, Vec3::new(1.0, 0.0, 0.0)),
        (Cardinal::South, Vec3::new(0.0, -1.0, 0.0)),
        (Cardinal::West, Vec3::new(-1.0, 0.0, 0.0)),
    ];
    let thresh = (half * 0.34).powi(2);
    let (mut best, mut best_d) = (None, f32::MAX);
    for (card, dir) in dirs {
        let anchor = Vec3::new(dir.x * R_CARD, dir.y * R_CARD, RING_Z);
        let world = cam_rotation.transform_point3(anchor);
        let clip = vp * Vec4::new(world.x, world.y, world.z, 1.0);
        if clip.w.abs() < 1e-6 {
            continue;
        }
        let sx = (clip.x / clip.w + 1.0) * 0.5 * vp_w;
        let sy = (1.0 - clip.y / clip.w) * 0.5 * vp_h;
        let d = (sx - mx).powi(2) + (sy - my).powi(2);
        if d < thresh && d < best_d {
            best_d = d;
            best = Some(card);
        }
    }
    best
}
