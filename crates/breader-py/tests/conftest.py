import numpy as np
import pytest
import segno


def render_qr(text, scale=6, border=4, **kwargs):
    kwargs.setdefault("boost_error", False)
    qr = segno.make(text, **kwargs)
    matrix = np.array([[bool(c) for c in row] for row in qr.matrix], dtype=bool)
    matrix = np.pad(matrix, border, constant_values=False)
    img = np.where(matrix, 0, 255).astype(np.uint8)
    return np.kron(img, np.ones((scale, scale), dtype=np.uint8))


_L = ["0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011", "0110111", "0001011"]
_G = ["0100111", "0110011", "0011011", "0100001", "0011101", "0111001", "0000101", "0010001", "0001001", "0010111"]
_R = ["1110010", "1100110", "1101100", "1000010", "1011100", "1001110", "1010000", "1000100", "1001000", "1110100"]
_PARITY = ["LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL", "LGGLGL"]


def ean13_digits(first12):
    total = sum(int(d) * (3 if i % 2 else 1) for i, d in enumerate(first12))
    return first12 + str((10 - total % 10) % 10)


def render_ean13(first12, module=3, height=80, quiet=12):
    code = ean13_digits(first12)
    bits = "101"
    for i, d in enumerate(code[1:7]):
        bits += (_L if _PARITY[int(code[0])][i] == "L" else _G)[int(d)]
    bits += "01010"
    for d in code[7:]:
        bits += _R[int(d)]
    bits += "101"
    row = np.array([0 if b == "1" else 255 for b in bits], dtype=np.uint8)
    row = np.pad(np.repeat(row, module), quiet * module, constant_values=255)
    img = np.tile(row, (height, 1))
    return np.pad(img, 20, constant_values=255), code


_CODABAR = {
    "0": "nnnnnww", "1": "nnnnwwn", "2": "nnnwnnw", "3": "wwnnnnn", "4": "nnwnnwn",
    "5": "wnnnnwn", "6": "nwnnnnw", "7": "nwnnwnn", "8": "nwwnnnn", "9": "wnnwnnn",
    "-": "nnnwwnn", "$": "nnwwnnn", ":": "wnnnwnw", "/": "wnwnnnw", ".": "wnwnwnn",
    "+": "nnwnwnw", "A": "nnwwnwn", "B": "nnnwnww", "C": "nwnwnnw", "D": "nnnwwwn",
}


def render_codabar(text, module=3, wide=3, height=80, quiet=12):
    """Codabar symbol; `text` includes the start and stop characters."""
    widths = []
    for i, ch in enumerate(text):
        if i:
            widths.append(1)
        widths += [wide if c == "w" else 1 for c in _CODABAR[ch]]
    row = []
    for i, w in enumerate(widths):
        row += [0 if i % 2 == 0 else 255] * (w * module)
    row = np.pad(np.array(row, dtype=np.uint8), quiet * module, constant_values=255)
    img = np.tile(row, (height, 1))
    return np.pad(img, 20, constant_values=255)


@pytest.fixture(scope="session")
def qr_image():
    return render_qr("https://example.com/breader?x=1", error="m")


@pytest.fixture(scope="session")
def ean_image():
    return render_ean13("590123412345")
