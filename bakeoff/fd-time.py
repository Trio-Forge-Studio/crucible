"""Time complete, sequential FD builds against two saved release binaries."""

import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess
import time
import shutil


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--copy", required=True, type=Path)
    parser.add_argument("--trunk", required=True, type=Path)
    parser.add_argument("--head", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.copy = args.copy.resolve()
    if not args.copy.is_relative_to(Path(__file__).resolve().parent.parent / "build"):
        raise ValueError("--copy must be this worktree's disposable copy under build")
    rows = []
    baseline = None
    identical = 0
    for pair in range(1, 6):
        row = {"pair": pair}
        for name in ["trunk", "head"]:
            for folder in ["places/warehouse/out", "places/shared/out", "places/warehouse/include", "places/shared/include"]:
                output = args.copy / folder
                if not output.resolve().is_relative_to(args.copy):
                    raise ValueError(f"output escapes the disposable copy: {output}")
                if output.exists():
                    shutil.rmtree(output)
            for folder in ["places/warehouse/include", "places/shared/include"]:
                (args.copy / folder).mkdir(parents=True)
            for place in ["job", "warehouse"]:
                (args.copy / "build" / f"{place}.rbxl").unlink(missing_ok=True)
            binary = getattr(args, name).resolve()
            start = time.perf_counter()
            result = subprocess.run(
                [str(binary), "build"], cwd=args.copy, capture_output=True, text=True
            )
            row[name] = time.perf_counter() - start
            if result.returncode:
                raise RuntimeError(f"{name} pair {pair}: {result.stderr}{result.stdout}")
            if pair == 1:
                manifest = {
                    str(path.relative_to(args.copy)): hashlib.sha256(path.read_bytes()).hexdigest()
                    for folder in ["places/warehouse/out", "places/shared/out"]
                    for path in (args.copy / folder).rglob("*.luau")
                }
                if not manifest:
                    raise RuntimeError("FD wrote no Luau modules to compare")
                if name == "trunk":
                    baseline = manifest
                elif manifest != baseline:
                    raise RuntimeError("FD emitted Luau differs between trunk and head")
                else:
                    identical = len(manifest)
        row["ratio"] = row["head"] / row["trunk"]
        rows.append(row)
        print(json.dumps(row), flush=True)
    trunk = statistics.median(row["trunk"] for row in rows)
    head = statistics.median(row["head"] for row in rows)
    report = {"pairs": rows, "trunk_median": trunk, "head_median": head, "ratio": head / trunk, "fd_luau_identical": identical}
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
