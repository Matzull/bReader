"""Fast barcode and QR code reader written in Rust."""

from __future__ import annotations

import os
import threading
from typing import Any, List, Optional, Union

from ._breader import Reader, Symbol, __version__

__all__ = ["Reader", "Symbol", "read", "__version__"]

_local = threading.local()


def _default_reader() -> Reader:
    reader = getattr(_local, "reader", None)
    if reader is None:
        reader = _local.reader = Reader()
    return reader


def _looks_like_pil(image: Any) -> bool:
    return hasattr(image, "convert") and hasattr(image, "mode") and hasattr(image, "tobytes")


def read(
    image: Any,
    *,
    format: Optional[str] = None,
    width: Optional[int] = None,
    height: Optional[int] = None,
    stride: Optional[int] = None,
    **options: Any,
) -> List[Symbol]:
    """Read barcodes from a file path, numpy array, PIL image or bytes-like object.

    Keyword arguments other than ``format``, ``width``, ``height`` and ``stride``
    are passed to :class:`Reader`. Without options a per-thread default
    ``Reader`` is reused.
    """
    reader = Reader(**options) if options else _default_reader()
    if isinstance(image, (str, os.PathLike)):
        return reader.scan_file(image)
    if _looks_like_pil(image):
        gray = image.convert("L")
        return reader.scan(gray.tobytes(), format="luma8", width=gray.width, height=gray.height)
    return reader.scan(image, format=format, width=width, height=height, stride=stride)
