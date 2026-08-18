//! Large-cloud scale check for the bounded LAS/LAZ pipeline.
//!
//! Ignored by default so CI never pays for it. Run on demand:
//!
//! ```text
//! cargo test -p ocs_pointcloud --release --test scale_stress -- --ignored --nocapture
//! ```
//!
//! Generates a multi-million-point LAS and drives the full path — bounded
//! sampling, disk-backed LOD tile build, and parallel leaf reads — asserting
//! that the working set stays bounded and the leaf level holds every source
//! point exactly once. This is the crate-side guarantee behind the app's
//! "millions to billions without crashing" behaviour.

use las::{point::Format, Builder, Color, Point, Writer};
use ocs_pointcloud::{
    build_tiled_cache, inspect, read_tiles_parallel, sample, SampleOptions, TileCacheOptions,
    MAX_TILE_READ_WORKERS,
};
use std::path::PathBuf;
use std::time::Instant;

const POINT_COUNT: u64 = 2_000_000;

fn scratch_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ocs-scale-stress-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn generate(path: &std::path::Path, count: u64) {
    let mut builder = Builder::default();
    builder.point_format = Format::new(3).unwrap();
    let mut writer = Writer::from_path(path, builder.into_header().unwrap()).unwrap();
    for index in 0..count {
        // Small coordinates keep the default LAS scale (0.001) representable
        // in i32; the stress test targets point count, not survey extents.
        let x = (index % 2_000) as f64;
        let y = ((index / 2_000) % 2_000) as f64;
        let z = 100.0 + (index % 97) as f64 * 0.5;
        writer
            .write_point(Point {
                x,
                y,
                z,
                intensity: (index % 65_536) as u16,
                classification: las::point::Classification::new((index % 6) as u8).unwrap(),
                return_number: 1,
                number_of_returns: 1,
                gps_time: Some(50_000.0 + index as f64),
                color: Some(Color::new((index % 65535) as u16, 20, 30)),
                ..Default::default()
            })
            .unwrap();
    }
    writer.close().unwrap();
}

#[test]
#[ignore = "generates a 2M-point LAS; run explicitly with --release"]
fn multi_million_point_pipeline_is_bounded() {
    let dir = scratch_dir();
    let source = dir.join("scale.las");
    let cache = dir.join("scale.las.ocstiles");

    let start = Instant::now();
    generate(&source, POINT_COUNT);
    eprintln!("generated {POINT_COUNT} points in {:?}", start.elapsed());

    let metadata = inspect(&source).expect("inspect");
    assert_eq!(POINT_COUNT, metadata.point_count);

    let start = Instant::now();
    let sampled = sample(
        &source,
        SampleOptions {
            max_points: 500_000,
            chunk_size: 65_536,
        },
    )
    .expect("sample");
    assert!(sampled.points.len() <= 500_000);
    eprintln!(
        "sampled {} display points (stride {}) in {:?}",
        sampled.points.len(),
        sampled.stride,
        start.elapsed()
    );

    let start = Instant::now();
    let manifest = build_tiled_cache(
        &source,
        &cache,
        TileCacheOptions::default(),
        |_| true,
    )
    .expect("build tiled cache");
    eprintln!(
        "built {} tiles through level {} in {:?}",
        manifest.tiles.len(),
        manifest.leaf_level,
        start.elapsed()
    );

    let leaves: Vec<_> = manifest
        .tiles
        .iter()
        .filter(|tile| tile.key.level == manifest.leaf_level)
        .cloned()
        .collect();
    let start = Instant::now();
    let loaded = read_tiles_parallel(&cache, &leaves, MAX_TILE_READ_WORKERS).expect("read leaves");
    let total: usize = loaded.iter().map(|(_, points)| points.len()).sum();
    assert_eq!(POINT_COUNT as usize, total, "leaf level must hold every source point");
    eprintln!(
        "parallel-read {} leaf points across {} tiles in {:?}",
        total,
        loaded.len(),
        start.elapsed()
    );

    // A bounded re-read of a small view never exceeds the target leaf budget.
    let view = manifest.select_tiles(
        metadata.bounds_min,
        metadata.bounds_max,
        100_000,
    );
    let view_points: u64 = view.iter().map(|tile| tile.point_count).sum();
    assert!(view_points <= 100_000, "view selection must respect the budget");

    std::fs::remove_dir_all(&dir).ok();
}
