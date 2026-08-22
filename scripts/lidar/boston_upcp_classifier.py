#!/usr/bin/env python3
"""Conservative UPCP-style classifier for the Boston USGS LAZ delivery.

The upstream Urban_PointCloud_Processing project is a Dutch registry-fusion
pipeline.  This adapter keeps its ordered-fuser and extra-byte label model, but
uses authoritative Boston building, roadway, and street-tree services in the
source CRS.

By default the source ASPRS ``classification`` dimension is never changed.  A
uint8 extra dimension named ``label`` is added using the UPCP label table:

    0 unknown, 1 road, 9 ground, 10 building, 14 bridge, 30 vegetation, 99 noise

Water (ASPRS 9) and rail (ASPRS 10) remain available in the preserved source
classification because UPCP does not define equivalent labels.  ``--write-asprs``
also writes road/building/vegetation to standard display classes 11/6/5 and
retains the input byte in an extra dimension named ``source_classification``.
"""

from __future__ import annotations

import argparse
import copy
import datetime as dt
import json
import os
from collections import Counter
from pathlib import Path
import sys
import time
from typing import Any, Iterable
from urllib import parse, request

import laspy
import numpy as np
import shapely
from shapely.geometry import shape


UPSTREAM_REPOSITORY = "https://github.com/Coding-Oafs/Urban_PointCloud_Processing"
BUILDING_QUERY_URL = (
    "https://gis.bostonplans.org/hosting/rest/services/"
    "Boston_Buildings/FeatureServer/9/query"
)
ROAD_QUERY_URL = (
    "https://services.arcgis.com/sFnw0xNflSi8J0uh/arcgis/rest/services/"
    "All_Boston_Roads/FeatureServer/0/query"
)
TREE_QUERY_URL = (
    "https://services.arcgis.com/sFnw0xNflSi8J0uh/arcgis/rest/services/"
    "Primary_Street_Trees_Public/FeatureServer/0/query"
)
TREE_WHERE = "TreeThere IN ('Y','Yes') AND (Alive IS NULL OR Alive <> 'N')"
TARGET_WKID = 6492

UPCP_LABELS = {
    0: "Unknown",
    1: "Road",
    9: "Ground",
    10: "Building",
    14: "Bridge",
    30: "Vegetation",
    99: "Noise",
}

UPCP_TO_ASPRS = {1: 11, 9: 2, 10: 6, 14: 17, 30: 5, 99: 18}

# ASPRS input class -> UPCP seed label.  Classes 9 (water) and 10 (rail) are
# deliberately absent because UPCP has no equivalent class.
ASPRS_SEEDS = {2: 9, 17: 14, 18: 99}

# Total fallback paved widths in survey feet, used only where MassDOT's
# SURFACE_WD and NUM_LANES attributes are both absent.
ROAD_CLASS_WIDTHS_FT = {1: 72.0, 2: 60.0, 3: 48.0, 4: 36.0, 5: 24.0, 6: 20.0}


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def log(message: str) -> None:
    print(f"[{dt.datetime.now().strftime('%H:%M:%S')}] {message}", flush=True)


def post_json(url: str, params: dict[str, Any], attempts: int = 5) -> dict[str, Any]:
    payload = parse.urlencode(params).encode("utf-8")
    last_error: Exception | None = None
    for attempt in range(1, attempts + 1):
        try:
            req = request.Request(
                url,
                data=payload,
                method="POST",
                headers={"User-Agent": "OpenCADStudio-UPCP-Adapter/1.0"},
            )
            with request.urlopen(req, timeout=120) as response:
                result = json.load(response)
            if "error" in result:
                raise RuntimeError(json.dumps(result["error"], sort_keys=True))
            return result
        except Exception as exc:  # retry transient ArcGIS/network failures
            last_error = exc
            if attempt == attempts:
                break
            time.sleep(min(2 ** attempt, 15))
    raise RuntimeError(f"ArcGIS query failed after {attempts} attempts: {last_error}")


def batched(values: list[int], size: int) -> Iterable[list[int]]:
    for offset in range(0, len(values), size):
        yield values[offset : offset + size]


def query_features(
    *,
    query_url: str,
    bounds: tuple[float, float, float, float],
    out_fields: str,
    cache_path: Path,
    margin_ft: float,
    where: str = "1=1",
) -> dict[str, Any]:
    if cache_path.exists():
        with cache_path.open("r", encoding="utf-8") as stream:
            cached = json.load(stream)
        log(f"reference cache: {cache_path.name} ({len(cached.get('features', [])):,} features)")
        return cached

    xmin, ymin, xmax, ymax = bounds
    envelope = json.dumps(
        {
            "xmin": xmin - margin_ft,
            "ymin": ymin - margin_ft,
            "xmax": xmax + margin_ft,
            "ymax": ymax + margin_ft,
            "spatialReference": {"wkid": TARGET_WKID},
        },
        separators=(",", ":"),
    )
    spatial = {
        "where": where,
        "geometry": envelope,
        "geometryType": "esriGeometryEnvelope",
        "inSR": str(TARGET_WKID),
        "spatialRel": "esriSpatialRelIntersects",
    }
    ids_result = post_json(
        query_url,
        {
            **spatial,
            "returnIdsOnly": "true",
            "returnGeometry": "false",
            "f": "json",
        },
    )
    object_ids = sorted(int(value) for value in ids_result.get("objectIds", []))
    features: list[dict[str, Any]] = []
    for group in batched(object_ids, 1000):
        result = post_json(
            query_url,
            {
                "where": where,
                "objectIds": ",".join(str(value) for value in group),
                "outFields": out_fields,
                "returnGeometry": "true",
                "outSR": str(TARGET_WKID),
                "f": "geojson",
            },
        )
        features.extend(result.get("features", []))

    collection = {
        "type": "FeatureCollection",
        "name": cache_path.stem,
        "crs": {"type": "name", "properties": {"name": f"EPSG:{TARGET_WKID}"}},
        "features": features,
        "metadata": {
            "query_url": query_url,
            "queried_utc": utc_now(),
            "bounds": [xmin, ymin, xmax, ymax],
            "margin_ft": margin_ft,
            "where": where,
            "object_id_count": len(object_ids),
        },
    }
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = cache_path.with_suffix(cache_path.suffix + ".partial")
    with temporary.open("w", encoding="utf-8") as stream:
        json.dump(collection, stream, separators=(",", ":"))
    os.replace(temporary, cache_path)
    log(f"downloaded {len(features):,} features -> {cache_path.name}")
    return collection


def valid_geometry(feature: dict[str, Any]) -> Any | None:
    raw = feature.get("geometry")
    if not raw:
        return None
    geometry = shape(raw)
    if geometry.is_empty:
        return None
    if not geometry.is_valid:
        geometry = shapely.make_valid(geometry)
    return None if geometry.is_empty else geometry


def building_mask_geometry(collection: dict[str, Any]) -> Any | None:
    geometries = [geometry for feature in collection.get("features", []) if (geometry := valid_geometry(feature))]
    if not geometries:
        return None
    merged = shapely.union_all(geometries)
    shapely.prepare(merged)
    return merged


def positive_number(value: Any) -> float | None:
    try:
        number = float(value)
    except (TypeError, ValueError):
        return None
    return number if np.isfinite(number) and number > 0 else None


def road_half_width_ft(properties: dict[str, Any], extra_ft: float) -> float:
    surface_width = positive_number(properties.get("SURFACE_WD"))
    lanes = positive_number(properties.get("NUM_LANES"))
    road_class = int(positive_number(properties.get("CLASS")) or 5)
    if surface_width is not None:
        total_width = surface_width
    elif lanes is not None:
        total_width = lanes * 12.0
    else:
        total_width = ROAD_CLASS_WIDTHS_FT.get(road_class, 24.0)
    return min(max(total_width / 2.0 + extra_ft, 6.0), 80.0)


def road_mask_geometry(collection: dict[str, Any], extra_ft: float) -> Any | None:
    buffers = []
    for feature in collection.get("features", []):
        geometry = valid_geometry(feature)
        if geometry is None:
            continue
        half_width = road_half_width_ft(feature.get("properties", {}), extra_ft)
        buffers.append(shapely.buffer(geometry, half_width, cap_style="flat", join_style="mitre"))
    if not buffers:
        return None
    merged = shapely.union_all(buffers)
    shapely.prepare(merged)
    return merged


def tree_mask_geometry(collection: dict[str, Any], radius_ft: float) -> Any | None:
    buffers = []
    for feature in collection.get("features", []):
        geometry = valid_geometry(feature)
        if geometry is not None:
            buffers.append(shapely.buffer(geometry, radius_ft, quad_segs=6))
    if not buffers:
        return None
    merged = shapely.union_all(buffers)
    shapely.prepare(merged)
    return merged


def counter_json(counter: Counter[int]) -> dict[str, int]:
    return {str(key): int(counter[key]) for key in sorted(counter)}


def copy_points_with_label(
    points: Any,
    output_header: Any,
    labels: np.ndarray,
    source_classes: np.ndarray,
    write_asprs: bool,
) -> Any:
    output = laspy.ScaleAwarePointRecord.zeros(len(points), header=output_header)
    for dimension in points.point_format.dimension_names:
        output[dimension] = points[dimension]
    output["label"] = labels
    if write_asprs:
        display_classes = source_classes.copy()
        for upcp_label, asprs_class in UPCP_TO_ASPRS.items():
            display_classes[labels == upcp_label] = asprs_class
        output.classification = display_classes
        output["source_classification"] = source_classes
    return output


def write_manifest(path: Path, manifest: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".partial")
    with temporary.open("w", encoding="utf-8") as stream:
        json.dump(manifest, stream, indent=2, sort_keys=True)
        stream.write("\n")
    os.replace(temporary, path)


def classify_tile(
    source: Path,
    output: Path,
    references: Path,
    *,
    chunk_size: int,
    road_extra_ft: float,
    tree_radius_ft: float,
    use_buildings: bool,
    use_roads: bool,
    use_vegetation: bool,
    write_asprs: bool,
    overwrite: bool,
) -> dict[str, Any]:
    started = time.monotonic()
    if output.exists() and not overwrite:
        raise FileExistsError(f"refusing to replace existing output: {output}")
    partial = output.with_suffix(output.suffix + ".partial")
    if partial.exists():
        partial.unlink()

    with laspy.open(source) as reader:
        input_header = reader.header
        if "label" in set(input_header.point_format.extra_dimension_names):
            raise ValueError(f"source already contains a label dimension: {source}")
        bounds = (
            float(input_header.mins[0]),
            float(input_header.mins[1]),
            float(input_header.maxs[0]),
            float(input_header.maxs[1]),
        )
        buildings = (
            query_features(
                query_url=BUILDING_QUERY_URL,
                bounds=bounds,
                out_fields="OBJECTID,GRND_ELEV_2010,ROOF_ELEV_2010,BLDG_HGT_2010",
                cache_path=references / f"{source.stem}.buildings.geojson",
                margin_ft=10.0,
            )
            if use_buildings
            else {"features": []}
        )
        roads = (
            query_features(
                query_url=ROAD_QUERY_URL,
                bounds=bounds,
                out_fields="OBJECTID,CLASS,SURFACE_WD,NUM_LANES,F_CLASS_STR",
                cache_path=references / f"{source.stem}.roads.geojson",
                margin_ft=100.0,
            )
            if use_roads
            else {"features": []}
        )
        trees = (
            query_features(
                query_url=TREE_QUERY_URL,
                bounds=bounds,
                out_fields="FID,Species,Alive,TreeThere",
                cache_path=references / f"{source.stem}.trees.geojson",
                margin_ft=tree_radius_ft,
                where=TREE_WHERE,
            )
            if use_vegetation
            else {"features": []}
        )
        log(f"building union: {len(buildings.get('features', [])):,} polygons")
        building_geometry = building_mask_geometry(buildings)
        log(f"road union: {len(roads.get('features', [])):,} centerlines")
        road_geometry = road_mask_geometry(roads, road_extra_ft)
        log(f"vegetation union: {len(trees.get('features', [])):,} active trees")
        tree_geometry = tree_mask_geometry(trees, tree_radius_ft)

        output_header = copy.deepcopy(input_header)
        output_header.add_extra_dim(
            laspy.ExtraBytesParams(
                name="label",
                type=np.uint8,
                description="UPCP urban class label",
            )
        )
        if write_asprs:
            output_header.add_extra_dim(
                laspy.ExtraBytesParams(
                    name="source_classification",
                    type=np.uint8,
                    description="Original ASPRS class",
                )
            )
        provenance = {
            "schema": "OpenCADStudio.UPCP.Boston.v2",
            "upstream": UPSTREAM_REPOSITORY,
            "source_classification_preserved": not write_asprs,
            "source_classification_dimension": "source_classification" if write_asprs else None,
            "asprs_display_mapping": UPCP_TO_ASPRS if write_asprs else None,
            "label_dimension": "label",
            "labels": UPCP_LABELS,
            "asprs_seeds": ASPRS_SEEDS,
            "target_wkid": TARGET_WKID,
            "building_source": BUILDING_QUERY_URL,
            "road_source": ROAD_QUERY_URL,
            "vegetation_source": TREE_QUERY_URL,
            "road_extra_ft": road_extra_ft,
            "tree_radius_ft": tree_radius_ft,
            "created_utc": utc_now(),
        }
        output_header.vlrs.append(
            laspy.VLR(
                user_id="OpenCADStudio",
                record_id=1001,
                description="UPCP Boston classifier",
                record_data=json.dumps(provenance, separators=(",", ":")).encode("utf-8"),
            )
        )

        original_counts: Counter[int] = Counter()
        label_counts: Counter[int] = Counter()
        output_asprs_counts: Counter[int] = Counter()
        processed = 0
        with laspy.open(partial, mode="w", header=output_header, do_compress=True) as writer:
            for points in reader.chunk_iterator(chunk_size):
                classes = np.asarray(points.classification, dtype=np.uint8)
                labels = np.zeros(len(points), dtype=np.uint8)
                for input_class, target_label in ASPRS_SEEDS.items():
                    labels[classes == input_class] = target_label

                x = np.asarray(points.x)
                y = np.asarray(points.y)
                building_candidates = classes == 1
                if building_geometry is not None and np.any(building_candidates):
                    candidate_indices = np.flatnonzero(building_candidates)
                    inside = shapely.intersects_xy(
                        building_geometry,
                        x[candidate_indices],
                        y[candidate_indices],
                    )
                    labels[candidate_indices[inside]] = 10

                road_candidates = classes == 2
                if road_geometry is not None and np.any(road_candidates):
                    candidate_indices = np.flatnonzero(road_candidates)
                    inside = shapely.intersects_xy(
                        road_geometry,
                        x[candidate_indices],
                        y[candidate_indices],
                    )
                    labels[candidate_indices[inside]] = 1

                vegetation_candidates = (classes == 1) & (labels == 0)
                if tree_geometry is not None and np.any(vegetation_candidates):
                    candidate_indices = np.flatnonzero(vegetation_candidates)
                    inside = shapely.intersects_xy(
                        tree_geometry,
                        x[candidate_indices],
                        y[candidate_indices],
                    )
                    labels[candidate_indices[inside]] = 30

                for value, count in enumerate(np.bincount(classes, minlength=256)):
                    if count:
                        original_counts[value] += int(count)
                for value, count in enumerate(np.bincount(labels, minlength=256)):
                    if count:
                        label_counts[value] += int(count)

                display_classes = classes.copy()
                if write_asprs:
                    for upcp_label, asprs_class in UPCP_TO_ASPRS.items():
                        display_classes[labels == upcp_label] = asprs_class
                for value, count in enumerate(np.bincount(display_classes, minlength=256)):
                    if count:
                        output_asprs_counts[value] += int(count)

                writer.write_points(
                    copy_points_with_label(points, output_header, labels, classes, write_asprs)
                )
                processed += len(points)
                log(f"{source.name}: {processed:,}/{input_header.point_count:,} points")

    # Audit the completed temporary LAZ before publishing it.  If any invariant
    # fails, an existing destination remains untouched and the partial file is
    # visibly incomplete rather than masquerading as a classified delivery.
    with laspy.open(partial) as check:
        output_point_count = int(check.header.point_count)
        extra_dimensions = list(check.header.point_format.extra_dimension_names)
        output_crs = check.header.parse_crs()
        input_crs = input_header.parse_crs()
        if output_point_count != int(input_header.point_count):
            raise RuntimeError(f"point-count mismatch: {output_point_count} != {input_header.point_count}")
        if "label" not in extra_dimensions:
            raise RuntimeError("output label dimension is missing")
        if write_asprs and "source_classification" not in extra_dimensions:
            raise RuntimeError("output source_classification dimension is missing")
        if check.header.point_format.id != input_header.point_format.id:
            raise RuntimeError("output point format changed")
        if not np.array_equal(check.header.scales, input_header.scales):
            raise RuntimeError("output coordinate scales changed")
        if not np.array_equal(check.header.offsets, input_header.offsets):
            raise RuntimeError("output coordinate offsets changed")
        if str(output_crs) != str(input_crs):
            raise RuntimeError("output CRS changed")

    os.replace(partial, output)
    elapsed = time.monotonic() - started
    return {
        "status": "completed",
        "source": str(source),
        "output": str(output),
        "source_bytes": source.stat().st_size,
        "output_bytes": output.stat().st_size,
        "point_count": int(processed),
        "point_format": int(input_header.point_format.id),
        "las_version": str(input_header.version),
        "bounds": list(bounds),
        "original_classification_counts": counter_json(original_counts),
        "upcp_label_counts": counter_json(label_counts),
        "output_asprs_classification_counts": counter_json(output_asprs_counts),
        "building_feature_count": len(buildings.get("features", [])),
        "road_feature_count": len(roads.get("features", [])),
        "tree_feature_count": len(trees.get("features", [])),
        "elapsed_seconds": round(elapsed, 3),
        "completed_utc": utc_now(),
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input_dir", type=Path)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--tile", action="append", help="Process only a filename or stem; repeatable")
    parser.add_argument("--chunk-size", type=int, default=1_000_000)
    parser.add_argument("--road-extra-ft", type=float, default=1.0)
    parser.add_argument("--tree-radius-ft", type=float, default=12.0)
    parser.add_argument("--no-buildings", action="store_true")
    parser.add_argument("--no-roads", action="store_true")
    parser.add_argument("--no-vegetation", action="store_true")
    parser.add_argument("--write-asprs", action="store_true")
    parser.add_argument("--overwrite", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    input_dir = args.input_dir.resolve()
    output_dir = (args.output_dir or input_dir / "classified").resolve()
    if input_dir == output_dir:
        raise ValueError("output directory must be separate from the source directory")
    output_dir.mkdir(parents=True, exist_ok=True)
    references = output_dir / "references"
    references.mkdir(parents=True, exist_ok=True)

    selected = {value.casefold() for value in (args.tile or [])}
    sources = sorted(
        path
        for path in input_dir.iterdir()
        if path.is_file() and path.suffix.casefold() in {".las", ".laz"}
        and (not selected or path.name.casefold() in selected or path.stem.casefold() in selected)
    )
    if not sources:
        raise FileNotFoundError(f"no matching LAS/LAZ files in {input_dir}")

    manifest_path = output_dir / "classification_manifest.json"
    manifest: dict[str, Any] = {
        "schema": "OpenCADStudio.UPCP.Boston.batch.v2",
        "status": "running",
        "started_utc": utc_now(),
        "input_dir": str(input_dir),
        "output_dir": str(output_dir),
        "methodology": {
            "upstream_repository": UPSTREAM_REPOSITORY,
            "source_classification_preserved": not args.write_asprs,
            "source_classification_dimension": "source_classification" if args.write_asprs else None,
            "asprs_display_mapping": UPCP_TO_ASPRS if args.write_asprs else None,
            "label_dimension": "label",
            "labels": UPCP_LABELS,
            "asprs_seeds": ASPRS_SEEDS,
            "building_rule": "ASPRS class 1 inside official Boston building polygons -> UPCP 10",
            "road_rule": "ASPRS class 2 inside width-buffered official Boston road centerlines -> UPCP 1",
            "vegetation_rule": "remaining ASPRS class 1 inside active Boston street-tree buffers -> UPCP 30",
            "water_and_rail": "retained in ASPRS classification; UPCP label remains 0",
            "road_extra_ft": args.road_extra_ft,
            "tree_radius_ft": args.tree_radius_ft,
            "building_fuser": not args.no_buildings,
            "road_fuser": not args.no_roads,
            "vegetation_fuser": not args.no_vegetation,
            "reference_data_warning": "Boston reference layers are current and may differ from the 2013-2014 LiDAR epoch.",
        },
        "tiles": [],
    }
    write_manifest(manifest_path, manifest)
    failures = 0
    for index, source in enumerate(sources, start=1):
        output = output_dir / f"{source.stem}_classified.laz"
        log(f"tile {index}/{len(sources)}: {source.name}")
        try:
            result = classify_tile(
                source,
                output,
                references,
                chunk_size=args.chunk_size,
                road_extra_ft=args.road_extra_ft,
                tree_radius_ft=args.tree_radius_ft,
                use_buildings=not args.no_buildings,
                use_roads=not args.no_roads,
                use_vegetation=not args.no_vegetation,
                write_asprs=args.write_asprs,
                overwrite=args.overwrite,
            )
            log(f"completed {output.name} in {result['elapsed_seconds']:.1f}s")
        except Exception as exc:
            failures += 1
            result = {
                "status": "failed",
                "source": str(source),
                "output": str(output),
                "error": repr(exc),
                "failed_utc": utc_now(),
            }
            log(f"FAILED {source.name}: {exc}")
        manifest["tiles"].append(result)
        write_manifest(manifest_path, manifest)

    manifest["status"] = "completed" if failures == 0 else "completed_with_failures"
    manifest["completed_utc"] = utc_now()
    manifest["failure_count"] = failures
    write_manifest(manifest_path, manifest)
    log(f"batch {manifest['status']}: {len(sources) - failures}/{len(sources)} tiles")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())

