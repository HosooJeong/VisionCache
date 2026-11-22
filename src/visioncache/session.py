"""Window observation and input state shared by desktop integrations."""

from dataclasses import dataclass
import math
from threading import RLock
from typing import Protocol
from uuid import uuid4

from PIL import Image

from .observation import Frame, Mode, observe


class DesktopError(RuntimeError):
    """A window operation cannot use the selected observation."""


@dataclass(frozen=True)
class Window:
    handle: int
    process_id: int
    thread_id: int
    title: str


@dataclass(frozen=True)
class WindowState:
    window: Window
    bounds: tuple[int, int, int, int]

    @property
    def placement(self):
        window = self.window
        return window.handle, window.process_id, window.thread_id, self.bounds


class Desktop(Protocol):
    def windows(self) -> list[Window]: ...
    def capture(self, handle: int, *, follow_owned: bool = False) -> tuple[WindowState, Image.Image]: ...
    def click(self, state: WindowState, x: int, y: int) -> None: ...
    def scroll(self, state: WindowState, x: int, y: int, ticks: int) -> None: ...
    def fill(self, state: WindowState, x: int, y: int, text: str, replace: bool) -> None: ...
    def type_text(self, state: WindowState, text: str) -> None: ...
    def press_key(self, state: WindowState, chord: str) -> None: ...


@dataclass(frozen=True)
class Observation:
    id: str
    state: WindowState
    mode: Mode
    effective_mode: Mode
    frames: tuple[Frame, ...]

    def screen_point(self, image: int, x: int, y: int) -> tuple[int, int]:
        if not 0 <= image < len(self.frames):
            raise DesktopError("Image index is outside this observation.")
        local_x, local_y = self.frames[image].source_point(x, y)
        left, top, right, bottom = self.state.bounds
        _, _, width, height = self.frames[0].box
        return (
            left + local_x * (right - left) // width,
            top + local_y * (bottom - top) // height,
        )


class WindowSession:
    """Keep one capture history and one usable observation per stdio session."""

    def __init__(self, desktop: Desktop, scale: float = 0.5):
        if not math.isfinite(scale) or not 0.1 <= scale < 1.0:
            raise ValueError("Scale must be at least 0.1 and less than 1.")
        self.desktop = desktop
        self.scale = scale
        self._lock = RLock()
        self._latest: Observation | None = None
        self._previous: tuple[WindowState, Image.Image] | None = None

    def windows(self):
        with self._lock:
            return self.desktop.windows()

    def capture(self, handle: int, mode: Mode = "adaptive_delta",
                *, follow_owned: bool = False) -> Observation:
        with self._lock:
            # A failed capture should never leave a stale observation actionable.
            self._latest = None
            state, current = self.desktop.capture(handle, follow_owned=True) if follow_owned else self.desktop.capture(handle)
            previous = None
            if self._previous and self._previous[0].placement == state.placement:
                previous = self._previous[1]
            effective_mode = "full" if mode == "adaptive_delta" and state.window.handle != handle else mode
            frames = tuple(observe(effective_mode, current, previous, self.scale))
            # In full mode callers receive the unscaled image. Keep our cache independent.
            self._previous = (state, current.copy() if effective_mode == "full" else current)
            self._latest = Observation(uuid4().hex, state, mode, effective_mode, frames)
            return self._latest

    def _current(self, observation_id: str) -> Observation:
        if self._latest is None or self._latest.id != observation_id:
            raise DesktopError("Observation is stale. Capture the target window again.")
        return self._latest

    def map_point(self, observation_id: str, image: int, x: int, y: int):
        with self._lock:
            return self._current(observation_id).screen_point(image, x, y)

    def click(self, observation_id: str, image: int, x: int, y: int):
        with self._lock:
            snapshot = self._current(observation_id)
            point = snapshot.screen_point(image, x, y)
            self._latest = None
            self.desktop.click(snapshot.state, *point)

    def scroll(self, observation_id: str, image: int, x: int, y: int, ticks: int):
        with self._lock:
            snapshot = self._current(observation_id)
            point = snapshot.screen_point(image, x, y)
            self._latest = None
            self.desktop.scroll(snapshot.state, *point, ticks)

    def fill(self, observation_id: str, image: int, x: int, y: int, text: str, replace: bool):
        with self._lock:
            snapshot = self._current(observation_id)
            point = snapshot.screen_point(image, x, y)
            self._latest = None
            self.desktop.fill(snapshot.state, *point, text, replace)

    def type_text(self, observation_id: str, text: str):
        with self._lock:
            snapshot = self._current(observation_id)
            self._latest = None
            self.desktop.type_text(snapshot.state, text)

    def press_key(self, observation_id: str, chord: str):
        with self._lock:
            snapshot = self._current(observation_id)
            self._latest = None
            self.desktop.press_key(snapshot.state, chord)
