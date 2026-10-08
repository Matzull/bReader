#!/usr/bin/env python3
"""Python-side bench for breader (PyO3 bindings) and zxing-cpp.

Same matching, normalization, timing and report format as the Rust bench. Images are preloaded
as grayscale numpy arrays; the timed section is the engine call only.

usage: py_bench.py <dataset-dir> [--engine breader|zxingcpp|both] [--effort fast|balanced|exhaustive]
                   [--json out.json] [--no-overrides]
"""
import json
import sys
import time
from pathlib import Path

import numpy as np
from PIL import Image

try:
    import zxingcpp
except ImportError:
    zxingcpp = None
try:
    import breader
except ImportError:
    breader = None

SYMBOLOGIES = ["QR Code", "EAN-13", "EAN-8", "UPC-A", "UPC-E", "Code 128", "Code 39", "ITF", "Data Matrix"]

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
    "DATAMATRIX": "Data Matrix",
}

BR_FORMATS = {
    "qr": "QR Code",
    "ean13": "EAN-13",
    "ean8": "EAN-8",
    "upca": "UPC-A",
    "upce": "UPC-E",
    "code128": "Code 128",
    "code39": "Code 39",
    "itf": "ITF",
    "datamatrix": "Data Matrix",
}


class ZxingEngine:
    name = "zxing-cpp (python)"

    def __init__(self):
        if zxingcpp is None:
            sys.exit("zxingcpp is not installed")
        bf = zxingcpp.BarcodeFormat
        self.formats = {
            bf.QRCode: "QR Code",
            bf.EAN13: "EAN-13",
            bf.EAN8: "EAN-8",
            bf.UPCA: "UPC-A",
            bf.UPCE: "UPC-E",
            bf.Code128: "Code 128",
            bf.Code39: "Code 39",
            bf.Code39Ext: "Code 39",
            bf.Code39Std: "Code 39",
            bf.ITF: "ITF",
            bf.ITF14: "ITF",
            bf.DataMatrix: "Data Matrix",
        }

    def read(self, gray):
        return [
            (self.formats[r.format], bytes(r.bytes)) for r in zxingcpp.read_barcodes(gray) if r.format in self.formats
        ]


class BreaderEngine:
    def __init__(self, effort):
        if breader is None:
            sys.exit("breader is not installed (maturin develop --release in crates/breader-py)")
        self.name = f"breader (python, {effort})"
        self.reader = breader.Reader(effort)

    def read(self, gray):
        return [(BR_FORMATS[s.symbology], s.data) for s in self.reader.scan(gray) if s.symbology in BR_FORMATS]


def map_symbology(name):
    key = "".join(c for c in name if c.isascii() and c.isalnum()).upper()
    return ALIASES.get(key)


def canonical(sym, data):
    try:
        text = data.decode("utf-8").strip().replace("{GS}", "\x1d")
        data = text.removeprefix("\x1d").encode("utf-8")
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


OVERRIDES_PATH = Path(__file__).with_name("gt-overrides.json")


def load_overrides():
    return json.loads(OVERRIDES_PATH.read_text())["datasets"]


def apply_overrides(ann, dir_name, overrides):
    applied = 0
    for entry in overrides.get(dir_name, []):
        for img in ann["images"]:
            if img["file"] != entry["file"]:
                continue
            if "from" not in entry:
                # An entry without "from" adds a label the dataset is missing.
                to = entry["to"]
                if not any(b["format"] == to["format"] and b["text"] == to["text"] for b in img["barcodes"]):
                    img["barcodes"].append({"format": to["format"], "text": to["text"], "points": entry.get("points", [])})
                    applied += 1
                continue
            for b in img["barcodes"]:
                if b["format"] == entry["from"]["format"] and b["text"] == entry["from"]["text"]:
                    b["format"] = entry["to"]["format"]
                    b["text"] = entry["to"]["text"]
                    applied += 1
                    break
    return applied


def centroid(pts):
    if not pts:
        return None
    return (sum(p[0] for p in pts) / len(pts), sum(p[1] for p in pts) / len(pts))


def inside(poly, p):
    if len(poly) < 3:
        return False
    hit = False
    j = len(poly) - 1
    for i in range(len(poly)):
        a, b = poly[i], poly[j]
        if (a[1] > p[1]) != (b[1] > p[1]) and p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]:
            hit = not hit
        j = i
    return hit


def bbox(pts):
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return (min(xs), min(ys), max(xs), max(ys))


def bbox_iou(a, b):
    if not a or not b:
        return 0.0
    a, b = bbox(a), bbox(b)
    w = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    h = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    inter = w * h
    union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter
    return inter / union if union > 0 else 0.0


def quads_overlap(a, b):
    ca, cb = centroid(a), centroid(b)
    return (ca is not None and inside(b, ca)) or (cb is not None and inside(a, cb)) or bbox_iou(a, b) > 0.5


def gt_key(b):
    sym = map_symbology(b["format"])
    if sym is None:
        return (b["format"], b["text"].strip().encode("utf-8"))
    return canonical(sym, b["text"].encode("utf-8"))


def dedup_annotations(ann):
    """Drop GT entries labelling the same symbol twice; prefer UPC-A over EAN-13. Returns removed count."""
    removed = 0
    for img in ann["images"]:
        kept = []
        for b in img["barcodes"]:
            pts = b.get("points") or []
            dup = next(
                (i for i, k in enumerate(kept) if gt_key(k) == gt_key(b) and quads_overlap(k.get("points") or [], pts)),
                None,
            )
            if dup is None:
                kept.append(b)
            else:
                if map_symbology(b["format"]) == "UPC-A":
                    kept[dup] = b
                removed += 1
        img["barcodes"] = kept
    return removed


def find_annotations(root):
    return sorted(p for p in root.rglob("annotations.json") if ".git" not in p.parts)


def dataset_name(root, base):
    rel = base.relative_to(root)
    return str(rel) if str(rel) != "." else root.resolve().name


def run_dataset(base, ann, engine):
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
            found = engine.read(gray)
        except Exception as e:
            print(f"{base / file}: {e}", file=sys.stderr)
            stats.errors += 1
            found = []
        stats.latencies.append((time.perf_counter() - t0) * 1000.0)
        score(found, expected, stats)
    return stats


USAGE = "usage: py_bench.py <dataset-dir> [--engine breader|zxingcpp|both] [--effort E] [--json out.json] [--no-overrides]"


def main(argv):
    args = argv[1:]
    out_json = None
    root = None
    use_overrides = True
    which = "both"
    effort = "balanced"
    while args:
        a = args.pop(0)
        if a == "--json":
            out_json = args.pop(0)
        elif a == "--engine":
            which = args.pop(0)
        elif a == "--effort":
            effort = args.pop(0)
        elif a == "--no-overrides":
            use_overrides = False
        elif root is None:
            root = Path(a)
        else:
            sys.exit(USAGE)
    if root is None or which not in ("breader", "zxingcpp", "both"):
        sys.exit(USAGE)
    engines = []
    if which in ("breader", "both"):
        engines.append(BreaderEngine(effort))
    if which in ("zxingcpp", "both"):
        engines.append(ZxingEngine())
    files = find_annotations(root)
    if not files:
        sys.exit(f"no annotations.json found under {root}")
    totals = {e.name: Stats() for e in engines}
    per_dataset = {e.name: {} for e in engines}
    overrides = load_overrides()
    for f in files:
        ann = json.loads(f.read_text())
        if not str(ann.get("format", "")).startswith("barcode-benchmark/"):
            print(f"skipping {f}: unknown annotation format", file=sys.stderr)
            continue
        ds = dataset_name(root, f.parent)
        if use_overrides:
            n = apply_overrides(ann, f.parent.resolve().name, overrides)
            if n:
                print(f"{ds}: applied {n} GT override(s)", file=sys.stderr)
        print(f"{ds}: removed {dedup_annotations(ann)} duplicate GT entries", file=sys.stderr)
        for e in engines:
            stats = run_dataset(f.parent, ann, e)
            print_table(e.name, ds, stats)
            per_dataset[e.name][ds] = stats_json(stats)
            totals[e.name].merge(stats)
    if len(files) > 1:
        for e in engines:
            print_table(e.name, "ALL DATASETS", totals[e.name])
    if out_json:
        doc = {
            "dataset_root": str(root),
            "engines": [
                {"engine": e.name, "effort": effort if e.name.startswith("breader") else "default",
                 "datasets": per_dataset[e.name], "total": stats_json(totals[e.name])}
                for e in engines
            ],
        }
        Path(out_json).write_text(json.dumps(doc, indent=2))


if __name__ == "__main__":
    main(sys.argv)
