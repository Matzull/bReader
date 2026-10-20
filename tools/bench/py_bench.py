#!/usr/bin/env python3
"""Python-side bench for breader (PyO3 bindings), zxing-cpp and zbar (black box, through pyzbar).

Same matching, normalization, timing and report format as the Rust bench. Images are preloaded
as grayscale numpy arrays; the timed section is the engine call only.

usage: py_bench.py <dataset-dir> [--engine breader|zxingcpp|zbar|both|all] [--effort fast|balanced|exhaustive]
                   [--json out.json] [--no-overrides] [--subset N] [--zbar-all]

`both` is breader plus zxing-cpp; `all` adds zbar. zbar needs the system libzbar and pyzbar; it runs
with the library defaults unless --zbar-all names every symbology explicitly (see ZbarEngine).
"""
import json
import os
import re
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
    from pyzbar import pyzbar as pyzbar_mod
except Exception:  # ImportError, or OSError when libzbar is missing
    pyzbar_mod = None
try:
    import breader
except ImportError:
    breader = None

SYMBOLOGIES = ["QR Code", "EAN-13", "EAN-8", "UPC-A", "UPC-E", "Code 128", "Code 39", "ITF", "Data Matrix", "PDF417", "Aztec", "Codabar", "Micro QR Code", "rMQR Code", "GS1 DataBar", "GS1 DataBar Limited", "GS1 DataBar Expanded", "Code 93"]

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
    "CODE93": "Code 93",
    "ITF": "ITF",
    "INTERLEAVED2OF5": "ITF",
    "CODABAR": "Codabar",
    "DATAMATRIX": "Data Matrix",
    "PDF417": "PDF417",
    "AZTEC": "Aztec",
    "AZTECCODE": "Aztec",
    "MICROQRCODE": "Micro QR Code",
    "MICROQR": "Micro QR Code",
    "RMQRCODE": "rMQR Code",
    "RMQR": "rMQR Code",
    "DATABAR": "GS1 DataBar",
    "GS1DATABAR": "GS1 DataBar",
    "RSS14": "GS1 DataBar",
    "DATABAROMNIDIRECTIONAL": "GS1 DataBar",
    "DATABARTRUNCATED": "GS1 DataBar",
    "DATABARLIMITED": "GS1 DataBar Limited",
    "GS1DATABARLIMITED": "GS1 DataBar Limited",
    "RSSLIMITED": "GS1 DataBar Limited",
    "DATABAREXPANDED": "GS1 DataBar Expanded",
    "GS1DATABAREXPANDED": "GS1 DataBar Expanded",
    "RSSEXPANDED": "GS1 DataBar Expanded",
}

BR_FORMATS = {
    "qr": "QR Code",
    "ean13": "EAN-13",
    "ean8": "EAN-8",
    "upca": "UPC-A",
    "upce": "UPC-E",
    "code128": "Code 128",
    "code39": "Code 39",
    "code93": "Code 93",
    "itf": "ITF",
    "codabar": "Codabar",
    "datamatrix": "Data Matrix",
    "pdf417": "PDF417",
    "aztec": "Aztec",
    "micro-qr": "Micro QR Code",
    "rmqr": "rMQR Code",
    "databar": "GS1 DataBar",
    "databar-limited": "GS1 DataBar Limited",
    "databar-expanded": "GS1 DataBar Expanded",
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
            bf.Code93: "Code 93",
            bf.ITF: "ITF",
            bf.ITF14: "ITF",
            bf.Codabar: "Codabar",
            bf.DataMatrix: "Data Matrix",
            bf.PDF417: "PDF417",
            bf.Aztec: "Aztec",
            bf.MicroQRCode: "Micro QR Code",
            bf.RMQRCode: "rMQR Code",
            bf.DataBarOmni: "GS1 DataBar",
            bf.DataBarStkOmni: "GS1 DataBar",
            bf.DataBarStk: "GS1 DataBar",
            bf.DataBarLtd: "GS1 DataBar Limited",
            bf.DataBarExp: "GS1 DataBar Expanded",
            bf.DataBarExpStk: "GS1 DataBar Expanded",
        }

    def read(self, gray):
        out = []
        for r in zxingcpp.read_barcodes(gray):
            if r.format in self.formats:
                data = bytes(r.bytes)
                text = r.text.encode("utf-8", "replace")
                pos = r.position
                quad = [(p.x, p.y) for p in (pos.top_left, pos.top_right, pos.bottom_right, pos.bottom_left)]
                out.append((self.formats[r.format], data, [text] if text != data else [], quad))
        return out


class ZbarEngine:
    """zbar through pyzbar, used only as a black box (public API, outputs only).

    Normalisations applied so that the comparison is fair (each is also listed in docs/BASELINES.md):
    - symbology names are mapped to breader's; ISBN-13 is an EAN-13; EAN2, EAN5, ISBN-10, COMPOSITE
      and SQCODE have no breader counterpart and their reads are dropped (like unmapped zxing formats);
    - UPC-A / UPC-E arrive as UPCA, UPCE or EAN13 with a leading 0; `canonical` folds all of them to
      EAN-13 on both sides;
    - Interleaved 2 of 5 is reported at any length (zbar's own minimum is 6); no length filter is
      applied, so zbar can read lengths that breader's ITF rejects (D-008);
    - payloads are bytes; invalid UTF-8 is also offered as Shift JIS and Latin-1 text (labels hold UTF-8);
    - Codabar start/stop characters (A-D) are dropped on both sides by `canonical`;
    - DataBar digits come without application identifiers: a bare 14-digit GTIN gets the `01` prefix of
      the labels; parentheses of expanded output are dropped by `canonical`.
    Default settings are the library's; `all_enabled` passes every mapped symbology explicitly instead.
    """
    name = "zbar (pyzbar)"

    def __init__(self, all_enabled=False):
        if pyzbar_mod is None:
            sys.exit("pyzbar or libzbar is not available (pip install pyzbar; apt install libzbar0)")
        self.formats = {
            "EAN13": "EAN-13", "ISBN13": "EAN-13", "EAN8": "EAN-8", "UPCA": "UPC-A", "UPCE": "UPC-E",
            "CODE128": "Code 128", "CODE39": "Code 39", "CODE93": "Code 93", "I25": "ITF",
            "CODABAR": "Codabar", "DATABAR": "GS1 DataBar", "DATABAR_EXP": "GS1 DataBar Expanded",
            "QRCODE": "QR Code", "PDF417": "PDF417",
        }
        self.symbols = None
        if all_enabled:
            self.name = "zbar (pyzbar, all enabled)"
            S = pyzbar_mod.ZBarSymbol
            self.symbols = [getattr(S, n) for n in ("EAN13", "ISBN13", "EAN8", "UPCA", "UPCE", "CODE128", "CODE39",
                                                    "CODE93", "I25", "CODABAR", "DATABAR", "DATABAR_EXP", "QRCODE",
                                                    "PDF417", "SQCODE", "ISBN10", "EAN2", "EAN5", "COMPOSITE")]

    def read(self, gray):
        h, w = gray.shape
        out = []
        for r in pyzbar_mod.decode((np.ascontiguousarray(gray).tobytes(), w, h), symbols=self.symbols):
            fmt = self.formats.get(r.type)
            if fmt is None:
                continue
            data = bytes(r.data)
            alts = []
            try:
                data.decode("utf-8")
            except UnicodeDecodeError:
                for enc in ("shift_jis", "latin-1"):
                    try:
                        alts.append(data.decode(enc).encode("utf-8"))
                    except UnicodeDecodeError:
                        pass
            if fmt == "GS1 DataBar" and len(data) == 14 and data.isdigit():
                data = b"01" + data
            pts = [(p.x, p.y) for p in r.polygon] or [(r.rect.left, r.rect.top)]
            out.append((fmt, data, alts, pts))
        return out


class BreaderEngine:
    def __init__(self, effort):
        if breader is None:
            sys.exit("breader is not installed (maturin develop --release in crates/breader-py)")
        self.name = f"breader (python, {effort})"
        self.reader = breader.Reader(effort)

    def read(self, gray):
        out = []
        for s in self.reader.scan(gray):
            if s.symbology in BR_FORMATS:
                fmt = BR_FORMATS[s.symbology]
                # Same alternative forms as the Rust bench (engine.rs): the charset-decoded text, a Shift JIS
                # reading of undeclared non-UTF-8 bytes, and the Code 39 Full ASCII expansion.
                alts = []
                if s.text is not None:
                    alts.append(s.text.encode("utf-8"))
                try:
                    s.data.decode("utf-8")
                except UnicodeDecodeError:
                    try:
                        alts.append(s.data.decode("shift_jis").encode("utf-8"))
                    except UnicodeDecodeError:
                        pass
                full = code39_full_ascii(s.data) if fmt == "Code 39" else None
                if full is not None:
                    try:
                        alts.append(full.decode("utf-8").encode("utf-8"))
                    except UnicodeDecodeError:
                        pass
                alts = [a for a in alts if a != s.data]
                out.append((fmt, s.data, alts, list(s.corners)))
        return out


def code39_full_ascii(data):
    """Code 39 Full ASCII expansion (ISO/IEC 16388); None when a shift pair is invalid."""
    out = bytearray()
    it = iter(data)
    for c in it:
        if c not in b"$%/+":
            out.append(c)
            continue
        n = next(it, None)
        if n is None or not 65 <= n <= 90:
            return None
        k = n - 65
        if c == ord("$"):
            out.append(k + 1)
        elif c == ord("+"):
            out.append(ord("a") + k)
        elif c == ord("/"):
            if k <= 14:
                out.append(ord("!") + k)
            elif n == ord("Z"):
                out.append(ord(":"))
            else:
                return None
        elif k <= 4:
            out.append(0x1B + k)
        elif k <= 9:
            out.append(ord(";") + k - 5)
        elif k <= 14:
            out.append(ord("[") + k - 10)
        elif k <= 19:
            out.append(ord("{") + k - 15)
        else:
            out.append({20: 0, 21: ord("@"), 22: ord("`")}.get(k, 0x7F))
    return bytes(out)


def map_symbology(name):
    key = "".join(c for c in name if c.isascii() and c.isalnum()).upper()
    return ALIASES.get(key)


def upce_to_upca(e):
    """Expands an 8-digit UPC-E to 12-digit UPC-A; None when the number system is not 0 or 1."""
    if len(e) != 8 or e[:1] not in (b"0", b"1"):
        return None
    d = e[1:7]
    f = d[5:6]
    if f in (b"0", b"1", b"2"):
        mid = d[0:2] + f + b"0000" + d[2:5]
    elif f == b"3":
        mid = d[0:3] + b"00000" + d[3:5]
    elif f == b"4":
        mid = d[0:4] + b"00000" + d[4:5]
    else:
        mid = d[0:5] + b"0000" + f
    return e[:1] + mid + e[7:]


def strip_aztec_structured_append(text):
    """Drops a leading ` ID XY` header (same rule as tools/bench/src/matching.rs)."""
    if not text.startswith(" "):
        return text
    _, sep, tail = text[1:].partition(" ")
    ident = text[1:].split(" ", 1)[0]
    if sep and ident and len(tail) >= 2 and "A" <= tail[0] <= "Z" and "A" <= tail[1] <= "Z" and tail[0] <= tail[1]:
        return tail[2:]
    return text


def to_luma8(pil):
    """Grayscale like the Rust bench (`image` crate `into_luma8`): Rec. 709 weights 2126/7152/722 with
    truncating integer division, not PIL's Rec. 601 `convert("L")`; the two differ by up to a few levels,
    enough to flip marginal reads."""
    if pil.mode == "L":
        return np.asarray(pil)
    if pil.mode == "LA":
        return np.ascontiguousarray(np.asarray(pil)[..., 0])
    rgb = np.asarray(pil.convert("RGB"), dtype=np.uint32)
    luma = (rgb[..., 0] * 2126 + rgb[..., 1] * 7152 + rgb[..., 2] * 722) // 10000
    return luma.astype(np.uint8)


def undo_latin1_utf8(data):
    """Labels only (like matching.rs::canonical_label): UTF-8 that was read as Latin-1 and encoded again
    is repaired; anything else is returned unchanged."""
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return data
    if text.isascii() or any(ord(c) > 0xFF for c in text):
        return data
    try:
        return bytes(ord(c) for c in text).decode("utf-8").encode("utf-8")
    except UnicodeDecodeError:
        return data


def canonical(sym, data):
    try:
        text = data.decode("utf-8")
        if sym == "Aztec":
            text = strip_aztec_structured_append(text)
        text = text.strip().replace("{GS}", "\x1d").replace("\r\n", "\n").replace("\r", "\n")
        data = text.removeprefix("\x1d").encode("utf-8")
    except UnicodeDecodeError:
        data = data.strip()
    if sym == "Codabar":
        if len(data) >= 2 and data[:1].upper() in (b"A", b"B", b"C", b"D") and data[-1:].upper() in (b"A", b"B", b"C", b"D"):
            data = data[1:-1]
        return (sym, data)
    if sym.startswith("GS1 DataBar"):
        return (sym, bytes(b for b in data if b not in b"()\x1d"))
    if sym in ("UPC-A", "UPC-E") and data.isdigit():
        upca = None
        if sym == "UPC-E" and len(data) == 8:
            upca = upce_to_upca(data)
        elif sym == "UPC-A" and len(data) == 12:
            upca = data
        if upca is not None:
            return ("EAN-13", b"0" + upca)
        if len(data) == 13 and data[:1] == b"0":
            return ("EAN-13", data)
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


SHOW_FP = bool(os.environ.get("BENCH_SHOW_FP"))


def overlaps_region(quad, region):
    """A read lies on a GT polygon: a corner or the centre is inside it, the polygon's centre is inside
    the read, or the bounding boxes share a quarter of the smaller one."""
    if not quad or not region:
        return False
    if any(inside(region, p) for p in quad) or inside(region, centroid(quad)) or inside(quad, centroid(region)):
        return True
    a, b = bbox(quad), bbox(region)
    w = max(0.0, min(a[2], b[2]) - max(a[0], b[0]))
    h = max(0.0, min(a[3], b[3]) - max(a[1], b[1]))
    smaller = min((a[2] - a[0]) * (a[3] - a[1]), (b[2] - b[0]) * (b[3] - b[1]))
    return smaller > 0 and w * h >= 0.25 * smaller


def score(found, expected, stats, ignore=()):
    """`found` items are (symbology, data, alts, quad); alts are other forms a label may use. An
    unexpected read on an `ignore` polygon (undecodable symbol of unknown value) is not a false positive."""
    found_keys = [canonical(f[0], f[1]) for f in found]
    alt_keys = [[canonical(f[0], a) for a in f[2]] for f in found]
    exp_keys = [canonical(s, undo_latin1_utf8(d)) if s else None for s, d in expected]
    used = [False] * len(found)
    for (sym, _), key in zip(expected, exp_keys):
        if sym is None:
            stats.unsupported += 1
            continue
        row = stats.row(sym)
        row["expected"] += 1
        for i, fk in enumerate(found_keys):
            if not used[i] and (fk == key or key in alt_keys[i]):
                used[i] = True
                row["read"] += 1
                break
    known = [k for k in exp_keys if k is not None]
    for i, fk in enumerate(found_keys):
        if used[i] or fk in known or any(k in known for k in alt_keys[i]):
            continue
        if any(overlaps_region(found[i][3], r) for r in ignore):
            if SHOW_FP:
                print(f"masked read (undecodable region): {found[i][0]} {fk[1][:80]!r}", file=sys.stderr)
            continue
        if SHOW_FP:
            print(f"false positive: {found[i][0]} {fk[1][:80]!r}", file=sys.stderr)
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
                present = sum(b["format"] == to["format"] and b["text"] == to["text"] for b in img["barcodes"])
                for _ in range(present, entry.get("count", 1)):
                    img["barcodes"].append({"format": to["format"], "text": to["text"], "points": entry.get("points", [])})
                    applied += 1
                continue
            for b in img["barcodes"]:
                if b["format"] == entry["from"]["format"] and b["text"] == entry["from"]["text"]:
                    b["format"] = entry["to"]["format"]
                    b["text"] = entry["to"]["text"]
                    b.pop("bytes_hex", None)
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


def gt_data(b):
    if "bytes_hex" in b:
        try:
            return bytes.fromhex(b["bytes_hex"])
        except ValueError:
            pass
    return b["text"].encode("utf-8")


def gt_key(b):
    sym = map_symbology(b["format"])
    if sym is None:
        return (b["format"], gt_data(b).strip())
    return canonical(sym, gt_data(b))


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


UNDECODABLE = "UNDECODABLE"
BARBER_OVERRIDES_KEY = "barber"


def html_unescape(s):
    """The XML character references the ZVZ-real strings carry."""
    names = {"amp": "&", "lt": "<", "gt": ">", "quot": '"', "apos": "'"}

    def one(m):
        n = m.group(1)
        if n in names:
            return names[n]
        try:
            return chr(int(n[2:], 16) if n[1] in "xX" else int(n[1:]))
        except (ValueError, OverflowError):
            return m.group(0)

    return re.sub(r"&(amp|lt|gt|quot|apos|#[0-9]+|#[xX][0-9a-fA-F]+);", one, s)


def is_unknown_value(s):
    t = s.strip()
    return t == "" or t == "-1" or (t.startswith("^") and t[1:].strip() == "")


def strip_gs1_parentheses(s):
    if not s.startswith("("):
        return s
    out, in_ai = [], False
    for c in s:
        if c == "(" and not in_ai:
            in_ai = True
        elif c == ")" and in_ai:
            in_ai = False
        else:
            out.append(c)
    return "".join(out)


def map_region(ty, string):
    """VIA (Type, String) -> (format, text); the format is UNDECODABLE without a usable value.
    Mirrors benchlib::barber::map_region."""
    text = html_unescape(string)
    if is_unknown_value(text):
        return UNDECODABLE, text
    t = text.strip()
    digits = t.isascii() and t.isdigit()
    n = len(t)
    if ty == "EAN13":
        fmt = {13: "EAN_13", 12: "UPC_A", 8: "EAN_8"}.get(n) if digits else None
    elif ty in ("UPCS", "UPCA"):
        fmt = {12: "UPC_A", 8: "UPC_E"}.get(n) if digits else None
    elif ty == "C39":
        if len(t) >= 2 and t.startswith("*") and t.endswith("*"):
            t = t[1:-1]
        return "CODE_39", t
    elif ty == "UCC128":
        return "CODE_128", strip_gs1_parentheses(text)
    else:
        fmt = {
            "EAN8": "EAN_8", "C128": "CODE_128", "I2O5": "ITF", "QR": "QR_CODE", "DATAMATRIX": "DATAMATRIX",
            "PDF417": "PDF417", "AZTEC": "AZTEC", "2-DIGIT": "EAN_2", "IATA25": "IATA_2_OF_5",
            "INTELLIGENTMAIL": "INTELLIGENT_MAIL", "JAPANPOST": "JAPAN_POST", "KIX": "KIX",
            "POSTNET": "POSTNET", "ROYALMAILCODE": "ROYAL_MAIL",
        }.get(ty)
    if fmt is None:
        return UNDECODABLE, text
    return fmt, text


def cp437_byte(c):
    if c.isascii():
        return ord(c)
    try:
        b = c.encode("cp437")
    except UnicodeEncodeError:
        return None
    return b[0] if len(b) == 1 else None


def legacy_payload(ty, text):
    """ZVZ-real shows PDF417 / Data Matrix / Aztec payload bytes as IBM437 text, and binary payloads as the
    hex of its UTF-16 units (a NUL and a real '^' both show as '^'). Returns bytes, "ambiguous" (a '^' unit)
    or None for plain text. Mirrors benchlib::barber::legacy_payload."""
    if ty not in ("PDF417", "DATAMATRIX", "AZTEC"):
        return None
    t = text.rstrip("\r\n")
    if len(t) >= 8 and len(t) % 4 == 0 and re.fullmatch(r"[0-9A-F]+", t):
        units = [chr(int(t[i:i + 4], 16)) for i in range(0, len(t), 4)]
        latin1 = all(ord(c) <= 0xFF for c in units)
        out = [ord(c) if latin1 else cp437_byte(c) for c in units]
        if None in out:
            return None
        return "ambiguous" if "^" in units else bytes(out)
    # Text with box-drawing or Greek characters is bytes shown as IBM437; accented letters alone are real UTF-8.
    if ty == "PDF417" and any(c == "\u00f7" or "\u0393" <= c <= "\u03c9" or "\u2190" <= c <= "\u25ff" for c in t):
        out = [cp437_byte(c) for c in t]
        return None if None in out else bytes(out)
    return None


def shape_points(shape):
    if shape.get("name") == "polygon":
        return [[x, y] for x, y in zip(shape.get("all_points_x", []), shape.get("all_points_y", []))]
    if shape.get("name") == "ellipse":
        cx, cy, rx, ry = (shape.get(k, 0) for k in ("cx", "cy", "rx", "ry"))
        return [[cx - rx, cy - ry], [cx + rx, cy - ry], [cx + rx, cy + ry], [cx - rx, cy + ry]]
    return []


def find_barber_via(root):
    for d in (root, root / "Annotations" / "VIA", root / "BarBeR - Dataset" / "Annotations" / "VIA"):
        if d.name == "VIA" and (d / "Artelab.json").is_file() and (d.parent.parent / "dataset" / "images").is_dir():
            return d
    return None


def load_barber_sources(via):
    """One (source name, annotation dict) per VIA file; see benchlib::barber::load_sources."""
    images = via.parent.parent / "dataset" / "images"
    owner = {}
    sources = []
    for path in sorted(via.glob("*.json")):
        meta = json.loads(path.read_text())["_via_img_metadata"]
        per_file = {}
        for entry in meta.values():
            barcodes = []
            for r in entry.get("regions", []):
                attrs = r.get("region_attributes", {})
                string = attrs.get("String", "")
                string = string if isinstance(string, str) else json.dumps(string)
                fmt, text = map_region(str(attrs.get("Type", "")).strip(), string)
                b = {"format": fmt, "text": text, "points": shape_points(r.get("shape_attributes", {}))}
                if fmt != UNDECODABLE:
                    payload = legacy_payload(str(attrs.get("Type", "")).strip(), text)
                    if payload == "ambiguous":
                        b["format"] = UNDECODABLE
                    elif payload is not None:
                        b["bytes_hex"] = payload.hex()
                barcodes.append(b)
            per_file.setdefault(entry["filename"], []).append((entry.get("size", 0), barcodes))
        ann = {"format": "barcode-benchmark/via", "images": []}
        for file in sorted(per_file):
            group = per_file[file]
            if len(group) > 1:
                try:
                    w, h = Image.open(images / file).size
                    pixels = w * h
                except Exception:
                    pixels = None
                if any(sz == pixels for sz, _ in group):
                    group = [g for g in group if g[0] == pixels]
            barcodes = [b for _, bs in group for b in bs]
            if file in owner:
                owner[file]["barcodes"].extend(barcodes)
            else:
                img = {"file": file, "barcodes": barcodes}
                owner[file] = img
                ann["images"].append(img)
        sources.append((path.stem, ann))
    return sources


def to_raw_frame(polys, orientation, w, h):
    """VIA polygons are in the EXIF-corrected frame, the engines see raw pixels (EXIF 6, 3 and 8 occur)."""
    maps = {
        6: lambda p: [p[1], h - p[0]],
        3: lambda p: [w - p[0], h - p[1]],
        8: lambda p: [w - p[1], p[0]],
    }
    f = maps.get(orientation)
    return [[f(p) for p in poly] for poly in polys] if f else polys


def in_subset(file, n):
    """Keeps an image when an FNV-1a hash of its name falls in the 1/n share (same as the Rust bench)."""
    if n <= 1:
        return True
    h = 0xCBF29CE484222325
    for b in file.encode("utf-8"):
        h = ((h ^ b) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return (h >> 16) % n == 0


IMAGE_EXTENSIONS = {".jpg", ".jpeg", ".png", ".webp", ".gif", ".bmp", ".tif"}


def is_annotation_file(p):
    """annotations.json anywhere, or any .json inside a directory named `annotation`."""
    return p.name == "annotations.json" or (p.suffix == ".json" and p.parent.name.lower() == "annotation")


def find_annotations(root):
    return sorted(p for p in root.rglob("*.json") if ".git" not in p.parts and is_annotation_file(p))


def dataset_base(ann_path):
    """Directory the image names are relative to (the parent of an `annotation` directory)."""
    d = ann_path.parent
    return d.parent if d.name.lower() == "annotation" else d


def resolve_images(base, files):
    """Maps annotation image names to paths: direct files, else a stem match below `base` (ignoring the
    extension and a trailing `_annotated`). Names without an image are absent from the result."""
    index = None
    out = {}
    for file in files:
        if (base / file).is_file():
            out[file] = base / file
            continue
        if index is None:
            index = {}
            for p in sorted(base.rglob("*")):
                if p.is_file() and p.suffix.lower() in IMAGE_EXTENSIONS:
                    index.setdefault(p.stem, p)
        stem = Path(file).stem
        found = index.get(stem) or index.get(stem.removesuffix("_annotated"))
        if found:
            out[file] = found
    return out


def dataset_name(root, base):
    rel = base.relative_to(root)
    return str(rel) if str(rel) != "." else root.resolve().name


def run_dataset(base, ann, engine, subset=1):
    expected_by_file = {}
    ignore_by_file = {}
    for img in ann["images"]:
        if not in_subset(img["file"], subset):
            continue
        expected_by_file.setdefault(img["file"], []).extend(
            (map_symbology(b["format"]), gt_data(b)) for b in img["barcodes"] if b["format"] != UNDECODABLE
        )
        ignore_by_file.setdefault(img["file"], []).extend(
            b.get("points") or [] for b in img["barcodes"] if b["format"] == UNDECODABLE
        )
    stats = Stats()
    paths = resolve_images(base, expected_by_file)
    for file in sorted(expected_by_file):
        if file not in paths:
            print(f"skipping {file}: labelled but no such image", file=sys.stderr)
            continue
        expected = expected_by_file[file]
        stats.images += 1
        if SHOW_FP:
            print(f"image {paths[file]}", file=sys.stderr)
        try:
            pil = Image.open(paths[file])
            orientation = pil.getexif().get(274, 1)
            gray = to_luma8(pil)
        except Exception as e:
            print(f"{paths[file]}: {e}", file=sys.stderr)
            stats.errors += 1
            score([], expected, stats, ignore_by_file[file])
            continue
        ignore = to_raw_frame(ignore_by_file[file], orientation, gray.shape[1], gray.shape[0])
        t0 = time.perf_counter()
        try:
            found = engine.read(gray)
        except Exception as e:
            print(f"{base / file}: {e}", file=sys.stderr)
            stats.errors += 1
            found = []
        stats.latencies.append((time.perf_counter() - t0) * 1000.0)
        score(found, expected, stats, ignore)
    return stats


USAGE = "usage: py_bench.py <dataset-dir> [--engine breader|zxingcpp|both] [--effort E] [--json out.json] [--no-overrides] [--subset N]"


def main(argv):
    args = argv[1:]
    out_json = None
    root = None
    use_overrides = True
    which = "both"
    effort = "balanced"
    subset = 1
    zbar_all = False
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
        elif a == "--zbar-all":
            zbar_all = True
        elif a == "--subset":
            subset = int(args.pop(0))
        elif root is None:
            root = Path(a)
        else:
            sys.exit(USAGE)
    if root is None or which not in ("breader", "zxingcpp", "zbar", "both", "all"):
        sys.exit(USAGE)
    engines = []
    if which in ("breader", "both", "all"):
        engines.append(BreaderEngine(effort))
    if which in ("zxingcpp", "both", "all"):
        engines.append(ZxingEngine())
    if which in ("zbar", "all"):
        engines.append(ZbarEngine(zbar_all))
    via = find_barber_via(root)
    work = []
    if via is not None:
        images_dir = via.parent.parent / "dataset" / "images"
        work = [(name, ann, images_dir, BARBER_OVERRIDES_KEY) for name, ann in load_barber_sources(via)]
    else:
        files = find_annotations(root)
        if not files:
            sys.exit(f"no annotation files found under {root}")
        for f in files:
            ann = json.loads(f.read_text())
            if not str(ann.get("format", "")).startswith("barcode-benchmark/"):
                print(f"skipping {f}: unknown annotation format", file=sys.stderr)
                continue
            base = dataset_base(f)
            work.append((dataset_name(root, base), ann, base, base.resolve().name))
    totals = {e.name: Stats() for e in engines}
    per_dataset = {e.name: {} for e in engines}
    overrides = load_overrides()
    for ds, ann, base, key in work:
        if use_overrides:
            n = apply_overrides(ann, key, overrides)
            if n:
                print(f"{ds}: applied {n} GT override(s)", file=sys.stderr)
        print(f"{ds}: removed {dedup_annotations(ann)} duplicate GT entries", file=sys.stderr)
        for e in engines:
            stats = run_dataset(base, ann, e, subset)
            print_table(e.name, ds, stats)
            per_dataset[e.name][ds] = stats_json(stats)
            totals[e.name].merge(stats)
    if len(work) > 1:
        for e in engines:
            print_table(e.name, "ALL DATASETS", totals[e.name])
    if out_json:
        doc = {
            "dataset_root": str(root),
            "engines": [
                {"engine": e.name, "effort": effort if e.name.startswith("breader") else ("all-enabled" if "all enabled" in e.name else "default"),
                 "datasets": per_dataset[e.name], "total": stats_json(totals[e.name])}
                for e in engines
            ],
        }
        Path(out_json).write_text(json.dumps(doc, indent=2))


if __name__ == "__main__":
    main(sys.argv)
