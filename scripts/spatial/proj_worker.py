"""Line-delimited JSON worker around the bundled pyproj/PROJ runtime."""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path


def _transform(request: dict) -> dict:
    try:
        import pyproj

        data_dir = Path(request["data_dir"]).resolve()
        if not data_dir.is_dir():
            raise ValueError(f"PROJ data directory is missing: {data_dir}")
        pyproj.datadir.append_data_dir(str(data_dir))
        transformer = pyproj.Transformer.from_pipeline(str(request["pipeline"]))
        points = request.get("points", [])
        if not isinstance(points, list):
            raise ValueError("points must be an array")
        result = []
        for point in points:
            if not isinstance(point, list) or len(point) != 3:
                raise ValueError("every point must contain x, y, and z")
            coordinate = transformer.transform(*map(float, point), errcheck=True)
            xyz = [float(coordinate[0]), float(coordinate[1]), float(coordinate[2])]
            if not all(math.isfinite(value) for value in xyz):
                raise ValueError("PROJ produced a non-finite coordinate")
            result.append(xyz)
        return {"points": result}
    except Exception as error:  # return protocol errors instead of crashing
        return {"error": f"{type(error).__name__}: {error}"}


def _stdio() -> int:
    for line in sys.stdin:
        try:
            response = _transform(json.loads(line))
        except Exception as error:
            response = {"error": f"invalid request: {error}"}
        print(json.dumps(response, separators=(",", ":")), flush=True)
    return 0


def _self_test() -> int:
    try:
        import pyproj

        transformer = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:3857", always_xy=True)
        x, y = transformer.transform(-71.0, 42.0, errcheck=True)
        if not (-8_000_000 < x < -7_000_000 and 5_000_000 < y < 6_000_000):
            raise RuntimeError("unexpected PROJ self-test coordinate")
        print(json.dumps({"ok": True, "proj": pyproj.proj_version_str}))
        return 0
    except Exception as error:
        print(json.dumps({"ok": False, "error": str(error)}))
        return 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--stdio", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return _self_test()
    if args.stdio:
        return _stdio()
    parser.error("choose --stdio or --self-test")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
