from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple, Union
import os

__version__: str

class Symbol:
    @property
    def symbology(self) -> str:
        """One of "qr", "ean13", "ean8", "upca", "upce", "code128", "code39", "itf", "datamatrix",
        "pdf417", "aztec", "codabar", "micro-qr", "rmqr", "databar", "databar-limited",
        "databar-expanded" or "code93"."""
    @property
    def data(self) -> bytes: ...
    @property
    def text(self) -> Optional[str]: ...
    @property
    def content(self) -> str: ...
    @property
    def gs1(self) -> Optional[List[Tuple[str, str]]]: ...
    @property
    def gs1_hri(self) -> Optional[str]: ...
    @property
    def corners(self) -> List[Tuple[float, float]]: ...
    @property
    def orientation(self) -> float: ...
    @property
    def aim_id(self) -> str: ...
    @property
    def quality(self) -> float: ...
    @property
    def meta(self) -> Dict[str, Any]: ...
    def __eq__(self, other: object) -> bool: ...
    def __hash__(self) -> int: ...

class Reader:
    def __init__(
        self,
        effort: str = "balanced",
        symbologies: Union[str, Iterable[str], None] = None,
        *,
        try_invert: Optional[bool] = None,
        escalation: Optional[str] = None,
        max_symbols: int = 0,
        time_budget_ms: Optional[float] = None,
        upscale_max_side: Optional[int] = None,
        min_line_agreement: int = 2,
        ean_addon: str = "ignore",
        code39_full_ascii: bool = False,
        code39_checksum: bool = False,
        code39_min_length: int = 3,
        itf_lengths: Optional[Sequence[int]] = None,
        itf_min_length: int = 6,
    ) -> None: ...
    @property
    def options(self) -> Dict[str, Any]: ...
    def scan(
        self,
        image: Any,
        *,
        format: Optional[str] = None,
        width: Optional[int] = None,
        height: Optional[int] = None,
        stride: Optional[int] = None,
    ) -> List[Symbol]: ...
    def scan_file(self, path: Union[str, "os.PathLike[str]"]) -> List[Symbol]: ...
