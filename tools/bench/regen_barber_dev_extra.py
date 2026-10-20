#!/usr/bin/env python3
"""Regenerates tools/bench/barber-dev-extra.json, the trap list of the BarBeR development subset.

The list holds every BarBeR image where zxing-cpp or zbar (library defaults) gives a false positive on
the full set, and every image without a supported label. py_bench.py does the scoring (same matching
and ground-truth overrides as the Rust bench); this script only merges its dumps. The output is sorted
and has no timestamps, so rerunning with the same library versions reproduces the file byte for byte.

usage: tools/bench/regen_barber_dev_extra.py [--dataset data/barber] [--out tools/bench/barber-dev-extra.json]
Run it with the repository's .venv Python (zxing-cpp, pyzbar, numpy, pillow); it takes a few minutes.
"""
import argparse
import json
import subprocess
import sys
import tempfile
from importlib.metadata import version
from pathlib import Path

HERE = Path(__file__).resolve().parent
ENGINES = [("zxingcpp", "zxing-cpp"), ("zbar", "zbar")]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dataset", default="data/barber")
    ap.add_argument("--out", default=str(HERE / "barber-dev-extra.json"))
    args = ap.parse_args()

    reasons = {}
    with tempfile.TemporaryDirectory() as tmp:
        no_label = Path(tmp) / "no-label.txt"
        procs = []
        # The engines are independent, so they run side by side (latency is not used here).
        for i, (flag, label) in enumerate(ENGINES):
            fp_path = Path(tmp) / f"fp-{flag}.txt"
            cmd = [sys.executable, str(HERE / "py_bench.py"), args.dataset, "--engine", flag,
                   "--fp-list", str(fp_path)]
            if i == 0:
                cmd += ["--no-label-list", str(no_label)]
            procs.append((subprocess.Popen(cmd, stdout=subprocess.DEVNULL), fp_path, label))
        for proc, fp_path, label in procs:
            if proc.wait() != 0:
                sys.exit(f"py_bench.py failed for {label}")
            for line in fp_path.read_text().splitlines():
                _, file = line.split("\t", 1)
                reasons.setdefault(file, set()).add(f"{label} false positive")
        for file in no_label.read_text().splitlines():
            reasons.setdefault(file, set()).add("no supported label")

    doc = {
        "about": "BarBeR development subset trap images (docs/datasets.md#barber-development-subset); "
                 "regenerate with tools/bench/regen_barber_dev_extra.py, do not edit by hand.",
        "generated_with": {"zxing-cpp": version("zxing-cpp"), "pyzbar": version("pyzbar")},
        "images": [{"file": f, "reason": "; ".join(sorted(r))} for f, r in sorted(reasons.items())],
    }
    Path(args.out).write_text(json.dumps(doc, indent=2) + "\n")
    print(f"{len(doc['images'])} trap images written to {args.out}")


if __name__ == "__main__":
    main()
