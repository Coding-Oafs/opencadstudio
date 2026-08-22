#!/usr/bin/env python3
"""Stream-verify classified Boston LAZ outputs without loading a tile in RAM."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

import laspy
import numpy as np


LABEL_TO_ASPRS = {1: 11, 9: 2, 10: 6, 14: 17, 30: 5, 99: 18}


def add_byte_counts(counter: Counter[int], values: np.ndarray) -> None:
    """Accumulate a byte histogram in NumPy rather than iterating in Python."""
    counts = np.bincount(values, minlength=256)
    counter.update({index: int(count) for index, count in enumerate(counts) if count})


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("classified_dir", type=Path)
    parser.add_argument("--expected-total", type=int)
    args = parser.parse_args()

    manifest_path = args.classified_dir / "classification_manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    total = 0
    classes: Counter[int] = Counter()
    labels: Counter[int] = Counter()
    reports = []

    for tile in manifest["tiles"]:
        if tile.get("status") != "completed":
            raise RuntimeError(f"incomplete manifest tile: {tile}")
        output = Path(tile["output"])
        tile_total = 0
        tile_source: Counter[int] = Counter()
        tile_classes: Counter[int] = Counter()
        tile_labels: Counter[int] = Counter()
        with laspy.open(output) as reader:
            dimensions = set(reader.header.point_format.extra_dimension_names)
            if not {"label", "source_classification"}.issubset(dimensions):
                raise RuntimeError(f"{output.name}: missing audit dimensions: {dimensions}")
            if reader.header.point_count != tile["point_count"]:
                raise RuntimeError(f"{output.name}: header/manifest point-count mismatch")
            for points in reader.chunk_iterator(1_000_000):
                source = np.asarray(points["source_classification"], dtype=np.uint8)
                label = np.asarray(points["label"], dtype=np.uint8)
                classification = np.asarray(points.classification, dtype=np.uint8)
                expected = source.copy()
                for upcp, asprs in LABEL_TO_ASPRS.items():
                    expected[label == upcp] = asprs
                if not np.array_equal(classification, expected):
                    raise RuntimeError(f"{output.name}: ASPRS/UPCP mapping mismatch")
                tile_total += len(points)
                add_byte_counts(tile_source, source)
                add_byte_counts(tile_classes, classification)
                add_byte_counts(tile_labels, label)

        manifest_source = Counter(
            {
                int(key): int(value)
                for key, value in tile["original_classification_counts"].items()
            }
        )
        if tile_source != manifest_source:
            raise RuntimeError(f"{output.name}: preserved source histogram mismatch")
        total += tile_total
        classes.update(tile_classes)
        labels.update(tile_labels)
        reports.append(
            {
                "file": output.name,
                "points": tile_total,
                "vegetation": tile_labels[30],
            }
        )

    if args.expected_total is not None and total != args.expected_total:
        raise RuntimeError(f"expected {args.expected_total:,} points, found {total:,}")
    print(
        json.dumps(
            {
                "status": "verified",
                "tiles": reports,
                "total_points": total,
                "asprs_class_counts": dict(sorted(classes.items())),
                "upcp_label_counts": dict(sorted(labels.items())),
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
