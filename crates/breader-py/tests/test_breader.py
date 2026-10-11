import json
import os
import sys
import threading
import time
from pathlib import Path

import numpy as np
import pytest

import breader
from breader import Reader, Symbol

from conftest import render_codabar, render_qr

URL = "https://example.com/breader?x=1"
ROOT = Path(__file__).resolve().parents[3]
SYNTH_DIR = Path(os.environ.get("BREADER_SYNTH_DIR", ROOT / "data" / "synth"))


# ---------- blank images ----------

@pytest.mark.parametrize(
    "fmt,bpp",
    [("luma8", 1), ("rgb8", 3), ("rgba8", 4), ("bgra8", 4), ("nv12", 1), ("nv21", 1), ("i420", 1)],
)
def test_blank_1d_every_format(fmt, bpp):
    w, h = 64, 48
    size = w * h * bpp if bpp > 1 else w * h * 3 // 2
    assert Reader().scan(bytes([255]) * size, format=fmt, width=w, height=h) == []


def test_blank_nd_arrays():
    r = Reader()
    assert r.scan(np.full((40, 50), 255, np.uint8)) == []
    assert r.scan(np.full((40, 50, 3), 255, np.uint8)) == []
    assert r.scan(np.full((40, 50, 4), 255, np.uint8)) == []
    assert r.scan(np.full((40, 50, 4), 255, np.uint8), format="bgra8") == []
    assert r.scan(np.full((60, 50), 255, np.uint8), format="nv12") == []


# ---------- QR end to end ----------

def check_qr(symbols):
    assert len(symbols) == 1
    s = symbols[0]
    assert s.symbology == "qr"
    assert s.text == URL and s.data == URL.encode()
    assert s.content == "url"
    assert s.aim_id.startswith("]Q")
    assert len(s.corners) == 4
    assert s.meta["kind"] == "qr" and s.meta["ec_level"] == "M"
    assert isinstance(s.meta["version"], int) and isinstance(s.meta["eci"], list)
    assert s.meta["mirrored"] is False
    assert 0.0 <= s.quality <= 1.0
    assert s.gs1 is None and s.gs1_hri is None


def test_qr_numpy(qr_image):
    check_qr(Reader().scan(qr_image))


def test_qr_rgb_rgba(qr_image):
    rgb = np.stack([qr_image] * 3, axis=-1)
    rgba = np.concatenate([rgb, np.full_like(qr_image, 255)[..., None]], axis=-1)
    check_qr(Reader().scan(rgb))
    check_qr(Reader().scan(rgba))
    check_qr(Reader().scan(rgba, format="bgra8"))


def test_qr_bytes_and_bytearray(qr_image):
    h, w = qr_image.shape
    raw = qr_image.tobytes()
    check_qr(Reader().scan(raw, width=w, height=h))
    check_qr(Reader().scan(bytearray(raw), width=w, height=h))
    check_qr(Reader().scan(memoryview(raw), width=w, height=h))
    check_qr(Reader().scan(raw, width=w, height=h, format="luma8", stride=w))


def test_qr_1d_stride(qr_image):
    h, w = qr_image.shape
    padded = np.pad(qr_image, ((0, 0), (0, 16)), constant_values=0)
    check_qr(Reader().scan(padded.tobytes(), width=w, height=h, stride=w + 16))


def test_qr_pil(qr_image, tmp_path):
    PIL = pytest.importorskip("PIL.Image")
    img = PIL.fromarray(qr_image)
    check_qr(breader.read(img))
    check_qr(breader.read(img.convert("RGB")))
    check_qr(Reader().scan(np.asarray(img.convert("RGB"))))


def test_yuv_2d(qr_image):
    h, w = qr_image.shape
    h -= h % 2
    yuv = np.concatenate([qr_image[:h], np.full((h // 2, w), 128, np.uint8)])
    check_qr(Reader().scan(yuv, format="nv12"))
    check_qr(Reader().scan(yuv.tobytes(), format="i420", width=w, height=h))


# ---------- strided views ----------

def test_strided_views(qr_image):
    big = np.kron(qr_image, np.ones((2, 2), np.uint8))
    sub = big[::2, ::2]
    assert not sub.flags["C_CONTIGUOUS"]
    check_qr(Reader().scan(sub))
    pad = np.pad(qr_image, ((0, 0), (10, 0)), constant_values=255)
    view = pad[:, 10:]
    assert not view.flags["C_CONTIGUOUS"]
    check_qr(Reader().scan(view))
    check_qr(Reader().scan(np.asfortranarray(qr_image)))
    check_qr(Reader().scan(qr_image[::-1][::-1]))
    rgb = np.stack([qr_image] * 3, axis=-1)
    check_qr(Reader().scan(rgb[:, :, ::-1]))


# ---------- errors ----------

@pytest.mark.parametrize(
    "image,kwargs",
    [
        (np.zeros((10, 10), np.float32), {}),
        (np.zeros((10, 10), np.uint16), {}),
        (np.zeros((10, 10), np.int8), {}),
        (np.zeros((10, 10, 2), np.uint8), {}),
        (np.zeros((10, 10, 3), np.uint8), {"format": "bgr8"}),
        (np.zeros((10, 10, 3), np.uint8), {"format": "rgba8"}),
        (np.zeros((10, 10), np.uint8), {"format": "rgb8"}),
        (np.zeros((10, 10), np.uint8), {"format": "bogus"}),
        (np.zeros((10, 10), np.uint8), {"width": 11}),
        (np.zeros((10, 10), np.uint8), {"stride": 10}),
        (np.zeros((2, 2, 2, 2), np.uint8), {}),
        (np.zeros((0, 10), np.uint8), {}),
        (np.uint8(3), {}),
        (b"\x00" * 100, {}),
        (b"\x00" * 100, {"width": 10}),
        (b"\x00" * 100, {"width": 20, "height": 20}),
        (b"\x00" * 100, {"width": 0, "height": 10}),
        (b"\x00" * 100, {"width": 10, "height": 10, "stride": 5}),
        (b"\x00" * 100, {"width": 10, "height": 10, "format": "rgb8"}),
        (np.zeros((10, 10), np.uint8), {"format": "nv12"}),
        (object(), {}),
        ("text", {}),
    ],
)
def test_bad_input_raises_value_error(image, kwargs):
    with pytest.raises(ValueError):
        Reader().scan(image, **kwargs)


# ---------- options ----------

def test_default_options():
    o = Reader().options
    assert o["effort"] == "balanced" and o["escalation"] is None and o["upscale_max_side"] is None
    assert o["symbologies"] == ["qr", "ean13", "ean8", "upca", "upce", "code128", "code39", "itf", "datamatrix", "pdf417", "aztec", "codabar", "micro-qr"]
    assert o["itf_lengths"] == (14,) and o["min_line_agreement"] == 2
    o["effort"] = "x"
    assert Reader().options["effort"] == "balanced"


def test_options_roundtrip():
    r = Reader(
        "fast", ["QR", "ean-13", "Code_128"], try_invert=True, escalation="stable", max_symbols=3,
        time_budget_ms=250.5, upscale_max_side=300, min_line_agreement=3, ean_addon="optional", code39_full_ascii=True,
        code39_checksum=True, code39_min_length=4, itf_lengths=[6, 8], itf_min_length=4,
    )
    assert r.options == {
        "effort": "fast", "symbologies": ["qr", "ean13", "code128"], "try_invert": True,
        "escalation": "stable", "max_symbols": 3, "time_budget_ms": 250.5, "upscale_max_side": 300,
        "min_line_agreement": 3,
        "ean_addon": "optional", "code39_full_ascii": True, "code39_checksum": True,
        "code39_min_length": 4, "itf_lengths": (6, 8), "itf_min_length": 4,
    }
    assert Reader(symbologies="qr").options["symbologies"] == ["qr"]
    assert Reader(itf_lengths=[]).options["itf_lengths"] == ()


@pytest.mark.parametrize(
    "kwargs",
    [
        {"effort": "max"},
        {"symbologies": ["micropdf417"]},
        {"symbologies": []},
        {"symbologies": 5},
        {"escalation": "never"},
        {"ean_addon": "yes"},
        {"max_symbols": -1},
        {"time_budget_ms": 0},
        {"time_budget_ms": -5.0},
        {"time_budget_ms": float("nan")},
        {"upscale_max_side": -1},
        {"min_line_agreement": 0},
        {"min_line_agreement": 70000},
        {"code39_min_length": 0},
        {"itf_lengths": [7]},
        {"itf_lengths": [0]},
        {"itf_min_length": -2},
    ],
)
def test_bad_options(kwargs):
    with pytest.raises(ValueError):
        Reader(**kwargs)


def test_symbology_filter_and_max_symbols(qr_image):
    assert Reader(symbologies=["ean13"]).scan(qr_image) == []
    assert len(Reader(symbologies=["qr"]).scan(qr_image)) == 1
    assert Reader(max_symbols=1, escalation="found").scan(qr_image)[0].text == URL


# ---------- files and read() ----------

def test_scan_file(qr_image, tmp_path):
    PIL = pytest.importorskip("PIL.Image")
    p = tmp_path / "qr.png"
    PIL.fromarray(qr_image).save(p)
    check_qr(Reader().scan_file(str(p)))
    check_qr(Reader().scan_file(p))
    check_qr(breader.read(p))
    check_qr(breader.read(str(p)))
    with pytest.raises(OSError):
        Reader().scan_file(str(tmp_path / "missing.png"))


def test_read_options_and_default(qr_image):
    check_qr(breader.read(qr_image))
    check_qr(breader.read(qr_image, effort="exhaustive", symbologies=["qr"]))
    assert breader.read(qr_image, symbologies=["ean13"]) == []
    raw = qr_image.tobytes()
    check_qr(breader.read(raw, width=qr_image.shape[1], height=qr_image.shape[0]))
    with pytest.raises(ValueError):
        breader.read(qr_image, effort="nope")


def test_read_default_reader_per_thread(qr_image):
    out = []

    def work():
        out.append(breader.read(qr_image))

    ts = [threading.Thread(target=work) for _ in range(4)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    assert len(out) == 4
    for o in out:
        check_qr(o)


# ---------- Symbol ----------

def test_ean13_linear_meta_and_gs1(ean_image):
    img, code = ean_image
    [s] = Reader().scan(img)
    assert s.symbology == "ean13" and s.text == code
    assert s.meta["kind"] == "linear"
    assert s.meta["checksum_verified"] is True
    assert s.meta["addon"] is None and s.meta["lines_agreeing"] >= 2
    assert s.content == "gs1"
    assert s.gs1 == [("01", "0" + code)]
    assert s.gs1_hri == "(01)0" + code
    assert s.aim_id.startswith("]E")


def test_symbol_eq_hash_repr(qr_image, ean_image):
    a = Reader().scan(qr_image)[0]
    b = Reader("exhaustive").scan(qr_image)[0]
    c = Reader().scan(ean_image[0])[0]
    assert a == b and hash(a) == hash(b)
    assert a != c and a != "x"
    assert len({a, b, c}) == 2
    assert "qr" in repr(a) and URL in repr(a)
    assert isinstance(a, Symbol)


def test_binary_payload_repr():
    img = render_qr(bytes(range(200, 230)).decode("latin-1"), encoding="latin-1", error="l")
    [s] = Reader().scan(img)
    assert s.data == bytes(range(200, 230))
    assert s.content in ("binary", "text")


# ---------- threading ----------

def noisy_image():
    rng = np.random.default_rng(1)
    return (rng.random((900, 900)) * 255).astype(np.uint8)


def test_scan_releases_gil():
    cpus = len(os.sched_getaffinity(0)) if hasattr(os, "sched_getaffinity") else os.cpu_count()
    if cpus < 2:
        pytest.skip("needs at least 2 CPUs")
    img = noisy_image()
    n = min(4, cpus)
    readers = [Reader("exhaustive") for _ in range(n)]
    readers[0].scan(img)

    def timed(fn):
        t = time.perf_counter()
        fn()
        return time.perf_counter() - t

    serial = timed(lambda: [r.scan(img) for r in readers])
    threads = [threading.Thread(target=r.scan, args=(img,)) for r in readers]

    def run():
        [t.start() for t in threads]
        [t.join() for t in threads]

    parallel = timed(run)
    assert parallel < serial * 0.75, (serial, parallel)


def test_shared_reader_from_threads(qr_image):
    r = Reader()
    results = []

    def work():
        for _ in range(5):
            results.append(r.scan(qr_image))

    ts = [threading.Thread(target=work) for _ in range(4)]
    [t.start() for t in ts]
    [t.join() for t in ts]
    assert len(results) == 20
    for res in results:
        check_qr(res)


# ---------- datasets ----------

def test_synth_clean_matches_rust_bench():
    from PIL import Image

    base = SYNTH_DIR / "clean"
    if not (base / "annotations.json").exists():
        pytest.skip("synthetic dataset not generated (set BREADER_SYNTH_DIR)")
    ann = json.loads((base / "annotations.json").read_text())
    r = Reader()
    read = expected = fp = 0
    for img in ann["images"]:
        want = [b["text"] for b in img["barcodes"]]
        expected += len(want)
        found = [s.text for s in r.scan(np.asarray(Image.open(base / img["file"]).convert("L")))]
        for w in want:
            if w in found:
                read += 1
                found.remove(w)
        fp += len(found)
    # Rust bench (cargo run -r -p bench -- data/synth/clean, seed 1): 26/26 (20 images plus 3 with Data Matrix and 3 with PDF417), 0 false positives.
    assert (read, expected, fp) == (26, 26, 0)


def test_datamatrix_meta_on_synth():
    from PIL import Image

    base = SYNTH_DIR / "datamatrix"
    if not (base / "annotations.json").exists():
        pytest.skip("synthetic dataset not generated (set BREADER_SYNTH_DIR)")
    ann = json.loads((base / "annotations.json").read_text())
    r = Reader()
    seen = 0
    for img in ann["images"][:8]:
        want = {b["text"] for b in img["barcodes"]}
        for s in r.scan(np.asarray(Image.open(base / img["file"]).convert("L"))):
            assert s.symbology == "datamatrix"
            assert s.text in want
            assert s.aim_id.startswith("]d")
            m = s.meta
            assert m["kind"] == "datamatrix"
            assert m["rows"] >= 8 and m["cols"] >= 10
            assert isinstance(m["mirrored"], bool) and m["errors_corrected"] >= 0
            assert m["structured_append"] is None
            assert m["reader_init"] is False
            seen += 1
    assert seen >= 6


def test_pdf417_meta_on_synth():
    from PIL import Image

    base = SYNTH_DIR / "clean"
    if not (base / "annotations.json").exists():
        pytest.skip("synthetic dataset not generated (set BREADER_SYNTH_DIR)")
    ann = json.loads((base / "annotations.json").read_text())
    r = Reader()
    seen = 0
    for img in ann["images"]:
        want = {b["text"] for b in img["barcodes"] if b["format"] == "PDF417"}
        if not want:
            continue
        found = r.scan(np.asarray(Image.open(base / img["file"]).convert("L")))
        assert [s.text for s in found] == list(want)
        s = found[0]
        assert s.symbology == "pdf417"
        assert s.aim_id == "]L2"
        m = s.meta
        assert m["kind"] == "pdf417"
        assert m["rows"] >= 3 and 1 <= m["cols"] <= 30 and 0 <= m["ec_level"] <= 8
        assert isinstance(m["compact"], bool) and m["macro"] is None
        assert m["reader_init"] is False
        assert m["errors_corrected"] == 0 and m["erasures"] == 0
        seen += 1
    assert seen == 3


def test_aztec_meta_on_synth():
    from PIL import Image

    base = SYNTH_DIR / "clean"
    if not (base / "annotations.json").exists():
        pytest.skip("synthetic dataset not generated (set BREADER_SYNTH_DIR)")
    ann = json.loads((base / "annotations.json").read_text())
    r = Reader()
    seen = 0
    for img in ann["images"]:
        want = {b["text"] for b in img["barcodes"] if b["format"] == "AZTEC"}
        if not want:
            continue
        found = r.scan(np.asarray(Image.open(base / img["file"]).convert("L")))
        assert [s.text for s in found] == list(want)
        s = found[0]
        assert s.symbology == "aztec"
        assert s.aim_id == "]z0"
        m = s.meta
        assert m["kind"] == "aztec"
        assert isinstance(m["compact"], bool) and m["layers"] >= 1
        assert m["data_codewords"] >= 1 and m["ec_codewords"] >= 3
        assert m["rune"] is None and m["reader_init"] is False
        assert isinstance(m["mirrored"], bool) and m["eci"] == []
        seen += 1
    assert seen >= 3


def test_micro_qr_meta_on_synth():
    from PIL import Image

    base = SYNTH_DIR / "clean"
    if not (base / "annotations.json").exists():
        pytest.skip("synthetic dataset not generated (set BREADER_SYNTH_DIR)")
    ann = json.loads((base / "annotations.json").read_text())
    r = Reader()
    seen = 0
    for img in ann["images"]:
        want = {b["text"] for b in img["barcodes"] if b["format"] == "MICRO_QR_CODE"}
        if not want:
            continue
        found = r.scan(np.asarray(Image.open(base / img["file"]).convert("L")))
        assert [s.text for s in found] == list(want)
        s = found[0]
        assert s.symbology == "micro-qr"
        assert s.aim_id == "]Q1"
        m = s.meta
        assert m["kind"] == "qr" and 1 <= m["version"] <= 4 and 0 <= m["mask"] <= 3
        assert m["ec_level"] in ("DetectionOnly", "L", "M", "Q")
        assert (m["ec_level"] == "DetectionOnly") == (m["version"] == 1)
        assert m["eci"] == [] and isinstance(m["mirrored"], bool)
        seen += 1
    if seen == 0:
        pytest.skip("synthetic dataset has no Micro QR images (regenerate it)")


def test_micro_qr_is_a_known_symbology():
    for name in ("micro-qr", "microqr", "Micro_QR"):
        assert Reader(symbologies=[name]).options["symbologies"] == ["micro-qr"]


def test_aztec_is_a_known_symbology():
    assert Reader(symbologies=["aztec"]).options["symbologies"] == ["aztec"]


def test_pdf417_is_a_known_symbology():
    assert Reader(symbologies=["pdf417"]).options["symbologies"] == ["pdf417"]


def test_codabar_data_excludes_start_stop_and_meta_records_them():
    img = render_codabar("C294/586D")
    [s] = Reader().scan(img)
    assert s.symbology == "codabar" and s.text == "294/586"
    assert s.aim_id == "]F0"
    assert s.meta["kind"] == "linear"
    assert s.meta["codabar_start_stop"] == ("C", "D")
    assert s.meta["checksum_verified"] is False
    assert Reader(symbologies=["code39"]).scan(img) == []
    assert Reader(symbologies=["codabar"]).options["symbologies"] == ["codabar"]


def test_codabar_without_stop_is_not_read():
    full = render_codabar("A123456B")
    assert Reader().scan(full)
    assert Reader().scan(render_codabar("A123456", module=3)) == []
