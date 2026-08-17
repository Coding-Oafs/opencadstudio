//! Real-data smoke test for the production LAS/LAZ pipeline.
//!
//! Ignored by default so CI never needs the fixture. Run it against a folder
//! of real tiles on demand:
//!
//! ```text
//! OCS_LIDAR_SMOKE_DIR="D:\MA_Lidar\..." cargo test -p ocs_pointcloud \
//!     --test real_data_smoke -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Exercises the whole path on production data: header/CRS inspection,
//! bounded sampling, disk-backed LOD tile build + parallel reads, sparse
//! per-source edits, and the merged multi-file export with per-file
//! `point_source_id`. Only the two smallest tiles are chosen so a debug-build
//! run stays reasonable.

use ocs_pointcloud::{
    build_tiled_cache, export_merged_progress, inspect, read_tiles_parallel, sample, EditStore,
    MergeSource, PointPatch, SampleOptions, TileCacheOptions, MAX_TILE_READ_WORKERS,
};
use std::path::{Path, PathBuf};

fn smoke_dir() -> Option<PathBuf> {
    std::env::var_os("OCS_LIDAR_SMOKE_DIR").map(PathBuf::from)
}

fn lidar_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("las") || extension.eq_ignore_ascii_case("laz")
                })
        })
        .collect();
    files.sort_by_key(|path| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(u64::MAX));
    files
}

fn output_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ocs-real-data-smoke-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
#[ignore = "set OCS_LIDAR_SMOKE_DIR to a folder of real LAS/LAZ tiles"]
fn production_folder_pipeline_smoke() {
    let Some(dir) = smoke_dir() else {
        eprintln!("OCS_LIDAR_SMOKE_DIR not set; skipping");
        return;
    };
    let files = lidar_files(&dir);
    assert!(!files.is_empty(), "no LAS/LAZ files under {}", dir.display());
    eprintln!("found {} LAS/LAZ file(s)", files.len());

    // Headers and CRS inspection across every tile.
    let mut total_points = 0_u64;
    for path in &files {
        let metadata = inspect(path).expect("inspect real tile");
        total_points += metadata.point_count;
        eprintln!(
            "{}: {} points, LAS {}.{}, format {}, compressed {}, CRS {} ({} EPSG h/v), VLRs {}",
            path.file_name().unwrap().to_string_lossy(),
            metadata.point_count,
            metadata.version_major,
            metadata.version_minor,
            metadata.point_format,
            metadata.compressed,
            metadata.has_crs,
            metadata
                .crs
                .horizontal_epsg
                .map(|code| code.to_string())
                .unwrap_or_else(|| "?".into()),
            metadata.vlr_count,
        );
    }
    eprintln!("dataset total: {total_points} source points");

    // The two smallest tiles keep the debug-build run fast.
    let smallest = &files[0];
    let second = files.get(1).expect("smoke needs at least two tiles");

    // Bounded sampling of the smallest tile.
    let sampled = sample(
        smallest,
        SampleOptions {
            max_points: 250_000,
            chunk_size: 65_536,
        },
    )
    .expect("sample real tile");
    eprintln!(
        "sampled {}: {} display points (stride {})",
        smallest.display(),
        sampled.points.len(),
        sampled.stride
    );
    assert!(!sampled.points.is_empty());
    assert!(sampled.points.len() <= 250_000);

    // LOD tile cache for the same tile, then a parallel read of every tile.
    let out = output_dir();
    let cache = out.join("smoke.ocstiles");
    let manifest = build_tiled_cache(
        smallest,
        &cache,
        TileCacheOptions {
            target_leaf_points: 50_000,
            read_chunk_size: 100_000,
            max_depth: 6,
        },
        |progress| {
            eprint!(
                "\r  indexed {}/{} points, {} tiles",
                progress.points_read, progress.total_points, progress.tiles_created
            );
            true
        },
    )
    .expect("build tiled cache for real tile");
    eprintln!(
        "\nbuilt {} tiles through level {}",
        manifest.tiles.len(),
        manifest.leaf_level
    );
    assert!(manifest.tiles.len() >= 2);
    let leaves: Vec<_> = manifest
        .tiles
        .iter()
        .filter(|tile| tile.key.level == manifest.leaf_level)
        .cloned()
        .collect();
    // The leaf level must hold every source point exactly once; interior
    // levels are decimated copies that stream for coarse views.
    let leaf_loaded = read_tiles_parallel(&cache, &leaves, MAX_TILE_READ_WORKERS)
        .expect("parallel leaf reads on real cache");
    let leaf_points: usize = leaf_loaded.iter().map(|(_, points)| points.len()).sum();
    eprintln!(
        "parallel leaf read: {leaf_points} points across {} leaf tiles ({} levels total)",
        leaf_loaded.len(),
        manifest.tiles.len()
    );
    let source_count = inspect(smallest).expect("inspect").point_count;
    assert_eq!(source_count as usize, leaf_points);
    let all_levels: u64 = manifest.tiles.iter().map(|tile| tile.point_count).sum();
    let everything = read_tiles_parallel(&cache, &manifest.tiles, MAX_TILE_READ_WORKERS)
        .expect("parallel reads of every level");
    assert_eq!(all_levels as usize, everything.iter().map(|(_, p)| p.len()).sum::<usize>());

    // Phase 2 toolset on real data: noise, ground, contours. The noise
    // radius must match the working set's own spacing — a strided sample is
    // sparse, so derive the radius from the point density instead of a
    // fixed survey constant.
    let bounds = sampled.metadata.clone();
    let area = (bounds.bounds_max[0] - bounds.bounds_min[0])
        * (bounds.bounds_max[1] - bounds.bounds_min[1]);
    let spacing = (area / sampled.points.len().max(1) as f64).sqrt();
    let noise_radius = (spacing * 3.0).max(0.5);
    let noise = ocs_pointcloud::detect_noise(&sampled.points, noise_radius, 3, 7);
    eprintln!(
        "noise detection at radius {noise_radius:.2} (sample spacing {spacing:.2}): {} of {} sampled points flagged",
        noise.len(),
        sampled.points.len(),
    );
    assert!(
        noise.len() * 20 < sampled.points.len(),
        "a real survey tile cannot be more than 5 percent isolated noise"
    );

    let ground_options = ocs_pointcloud::GroundOptions::default();
    let ground = ocs_pointcloud::classify_ground(&sampled.points, &ground_options);
    eprintln!(
        "ground classification: {} of {} sampled points accepted ({}%)",
        ground.len(),
        sampled.points.len(),
        ground.len() * 100 / sampled.points.len().max(1)
    );
    let ground_share = ground.len() as f64 / sampled.points.len() as f64;
    assert!(
        ground_share > 0.10 && ground_share < 0.95,
        "bare-earth share of an urban tile should sit between 10 and 95 percent, got {ground_share:.3}"
    );

    let mut ground_points = sampled.points.clone();
    for point in &mut ground_points {
        point.classification = 1;
    }
    let ground_indexes: std::collections::HashSet<u64> =
        ground.patches.iter().map(|(index, _)| *index).collect();
    for point in &mut ground_points {
        if ground_indexes.contains(&point.source_index) {
            point.classification = 2;
        }
    }
    let tin = ocs_pointcloud::Tin::from_points(&ground_points, Some(2)).expect("ground tin");
    let contours = ocs_pointcloud::generate_contours(&tin, 2.0, 0.0);
    let contour_points: usize = contours.iter().map(|contour| contour.points.len()).sum();
    eprintln!(
        "contours: {} polylines ({} vertices) from {} triangles at 2-unit intervals",
        contours.len(),
        contour_points,
        tin.triangle_count()
    );
    assert!(!contours.is_empty(), "a real tile must produce contours");
    for contour in &contours {
        for point in &contour.points {
            assert!((point[2] - contour.elevation).abs() < 1e-6);
        }
    }

    // Sparse per-source edits and the merged two-file export.
    let mut edits = EditStore::default();
    let classify_count = 5_000.min(sampled.points.len() as u64);
    edits.apply(
        "smoke: noise",
        0..classify_count,
        PointPatch::classification(7),
    );
    let merged = out.join("smoke-merged.laz");
    let stats = export_merged_progress(
        &[
            MergeSource {
                path: smallest.clone(),
                edits: edits.clone(),
            },
            MergeSource {
                path: second.clone(),
                edits: EditStore::default(),
            },
        ],
        &merged,
        |state| {
            eprint!(
                "\r  merged {}/{} points",
                state.points_read, state.total_points
            );
            true
        },
    )
    .expect("merged export of two real tiles");
    eprintln!(
        "\nmerged export: {} points ({} reclassified) -> {} ({} bytes)",
        stats.points_written,
        stats.points_reclassified,
        merged.display(),
        std::fs::metadata(&merged).unwrap().len()
    );
    assert_eq!(total_points_of(smallest) + total_points_of(second), stats.points_written);
    assert_eq!(classify_count, stats.points_reclassified as u64);
    let merged_metadata = inspect(&merged).expect("inspect merged output");
    assert_eq!(stats.points_written, merged_metadata.point_count);

    // Reimport sanity: the merged file samples cleanly with distinct sources.
    let reimported = sample(&merged, SampleOptions {
        max_points: 100_000,
        chunk_size: 65_536,
    }).expect("sample merged output");
    let sources: std::collections::BTreeSet<u16> = reimported
        .points
        .iter()
        .map(|point| point.point_source_id)
        .collect();
    eprintln!(
        "reimported sample sees point_source_ids {sources:?} ({} points)",
        reimported.points.len()
    );
    assert_eq!(sources.len(), 2, "both files must be identifiable in the merge");

    let _ = std::fs::remove_dir_all(&out);
}

fn total_points_of(path: &Path) -> u64 {
    inspect(path).expect("inspect").point_count
}
