#!/usr/bin/env python3
"""zxing-cpp baseline with the same matching, normalization, timing and report format as the Rust bench.

usage: zxingcpp_bench.py <dataset-dir> [--json out.json]
"""
import json
import sys
import time
from pathlib import Path

import numpy as np
import zxingcpp
from PIL import Image

SYMBOLOGIES = ["QR Code", "EAN-13", "EAN-8", "UPC-A", "UPC-E", "Code 128", "Code 39", "ITF"]

ALIASES = {
    "QRCODE": "QR Code",
    "QR": "QR Code",
    "EAN13": "EAN-13",
    "EAN8": "EAN-8",
    "UPCA": "UPC-A",
    "UPCE": "UPC-E",
    "CODE128": "Code 128",
    "CODE39": "Code 39",
    "CODE39EXTENDED": "Code 39",
    "ITF": "ITF",
    "INTERLEAVED2OF5": "ITF",
}

ZX_FORMATS = {
    zxingcpp.BarcodeFormat.QRCode: "QR Code",
    zxingcpp.BarcodeFormat.EAN13: "EAN-13",
    zxingcpp.BarcodeFormat.EAN8: "EAN-8",
    zxingcpp.BarcodeFormat.UPCA: "UPC-A",
    zxingcpp.BarcodeFormat.UPCE: "UPC-E",
    zxingcpp.BarcodeFormat.Code128: "Code 128",
    zxingcpp.BarcodeFormat.Code39: "Code 39",
    zxingcpp.BarcodeFormat.Code39Ext: "Code 39",
    zxingcpp.BarcodeFormat.Code39Std: "Code 39",
    zxingcpp.BarcodeFormat.ITF: "ITF",
    zxingcpp.BarcodeFormat.ITF14: "ITF",
}


def map_symbology(name):
    key = "".join(c for c in name if c.isascii() and c.isalnum()).upper()
    return ALIASES.get(key)


def canonical(sym, data):
    try:
        data = data.decode("utf-8").strip().encode("utf-8")
    except UnicodeDecodeError:
        data = data.strip()
    if sym == "UPC-A" and len(data) == 12 and data.isdigit():
        return ("EAN-13", b"0" + data)
    return (sym, data)


class Stats:
    def __init__(self):
        self.images = 0
        self.errors = 0
        self.unsupported = 0
        self.per = {}
        self.latencies = []

    def row(self, name):
        return self.per.setdefault(name, {"expected": 0, "read": 0, "false_positives": 0})

    def merge(self, other):
        self.images += other.images
        self.errors += other.errors
        self.unsupported += other.unsupported
        self.latencies += other.latencies
        for k, v in other.per.items():
            r = self.row(k)
            for f in r:
                r[f] += v[f]

    def totals(self):
        t = {"expected": 0, "read": 0, "false_positives": 0}
        for v in self.per.values():
            for f in t:
                t[f] += v[f]
        return t


def score(found, expected, stats):
    found_keys = [canonical(s, d) for s, d in found]
    exp_keys = [canonical(s, d) if s else None for s, d in expected]
    used = [False] * len(found)
    for (sym, _), key in zip(expected, exp_keys):
        if sym is None:
            stats.unsupported += 1
            continue
        row = stats.row(sym)
        row["expected"] += 1
        for i, fk in enumerate(found_keys):
            if not used[i] and fk == key:
                used[i] = True
                row["read"] += 1
                break
    known = [k for k in exp_keys if k is not None]
    for i, fk in enumerate(found_keys):
        if used[i] or fk in known:
            continue
        stats.row(found[i][0])["false_positives"] += 1


def percentile(sorted_ms, p):
    if not sorted_ms:
        return 0.0
    return sorted_ms[min(len(sorted_ms) - 1, round((len(sorted_ms) - 1) * p))]


def latency(stats):
    s = sorted(stats.latencies)
    return {"p50": percentile(s, 0.5), "p95": percentile(s, 0.95), "max": s[-1] if s else 0.0}


def rate(row):
    return row["read"] / row["expected"] if row["expected"] else None


def rate_text(row):
    r = rate(row)
    return "-" if r is None else f"{r * 100:.1f}%"


def print_table(engine, dataset, stats):
    print(f"[{engine}] {dataset}  ({stats.images} images)")
    print(f"{'symbology':<10} {'expected':>9} {'read':>6} {'rate':>8} {'FP':>4}")
    for name in SYMBOLOGIES:
        r = stats.per.get(name)
        if r:
            print(f"{name:<10} {r['expected']:>9} {r['read']:>6} {rate_text(r):>8} {r['false_positives']:>4}")
    t = stats.totals()
    print(f"{'TOTAL':<10} {t['expected']:>9} {t['read']:>6} {rate_text(t):>8} {t['false_positives']:>4}")
    lat = latency(stats)
    print(
        f"scan errors {stats.errors}, unsupported (excluded) {stats.unsupported}, "
        f"latency ms p50 {lat['p50']:.2f} p95 {lat['p95']:.2f} max {lat['max']:.2f}\n"
    )


def stats_json(stats):
    def sym(r):
        return {"expected": r["expected"], "read": r["read"], "read_rate": rate(r), "false_positives": r["false_positives"]}

    return {
        "images": stats.images,
        "scan_errors": stats.errors,
        "unsupported": stats.unsupported,
        "symbologies": {n: sym(stats.per[n]) for n in SYMBOLOGIES if n in stats.per},
        "total": sym(stats.totals()),
        "latency_ms": latency(stats),
    }


def find_annotations(root):
    return sorted(p for p in root.rglob("annotations.json") if ".git" not in p.parts)


def dataset_name(root, base):
    rel = base.relative_to(root)
    return str(rel) if str(rel) != "." else root.resolve().name


def run_dataset(base, ann):
    expected_by_file = {}
    for img in ann["images"]:
        expected_by_file.setdefault(img["file"], []).extend(
            (map_symbology(b["format"]), b["text"].encode("utf-8")) for b in img["barcodes"]
        )
    stats = Stats()
    for file in sorted(expected_by_file):
        expected = expected_by_file[file]
        stats.images += 1
        try:
            gray = np.asarray(Image.open(base / file).convert("L"))
        except Exception as e:
            print(f"{base / file}: {e}", file=sys.stderr)
            stats.errors += 1
            score([], expected, stats)
            continue
        t0 = time.perf_counter()
        try:
            results = zxingcpp.read_barcodes(gray)
        except Exception as e:
            print(f"{base / file}: {e}", file=sys.stderr)
            stats.errors += 1
            results = []
        stats.latencies.append((time.perf_counter() - t0) * 1000.0)
        found = []
        for r in results:
            sym = ZX_FORMATS.get(r.format)
            if sym:
                found.append((sym, bytes(r.bytes)))
        score(found, expected, stats)
    return stats


def main(argv):
    args = argv[1:]
    out_json = None
    root = None
    while args:
        a = args.pop(0)
        if a == "--json":
            out_json = args.pop(0)
        elif root is None:
            root = Path(a)
        else:
            sys.exit("usage: zxingcpp_bench.py <dataset-dir> [--json out.json]")
    if root is None:
        sys.exit("usage: zxingcpp_bench.py <dataset-dir> [--json out.json]")
    files = find_annotations(root)
    if not files:
        sys.exit(f"no annotations.json found under {root}")
    total = Stats()
    per_dataset = {}
    name = "zxing-cpp"
    for f in files:
        ann = json.loads(f.read_text())
        if not str(ann.get("format", "")).startswith("barcode-benchmark/"):
            print(f"skipping {f}: unknown annotation format", file=sys.stderr)
            continue
        ds = dataset_name(root, f.parent)
        stats = run_dataset(f.parent, ann)
        print_table(name, ds, stats)
        per_dataset[ds] = stats_json(stats)
        total.merge(stats)
    if len(per_dataset) > 1:
        print_table(name, "ALL DATASETS", total)
    if out_json:
        doc = {
            "dataset_root": str(root),
            "engines": [{"engine": name, "effort": "default", "datasets": per_dataset, "total": stats_json(total)}],
        }
        Path(out_json).write_text(json.dumps(doc, indent=2))


if __name__ == "__main__":
    main(sys.argv)
