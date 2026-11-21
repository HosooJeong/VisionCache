"""Window capture and foreground input via Win32 on Windows 10/11."""

from contextlib import contextmanager
import ctypes
from ctypes import wintypes as w
import sys

from PIL import ImageGrab

from .session import DesktopError, Window, WindowState

GA_ROOT = 2
PM_AWARE_V2 = ctypes.c_void_p(-4)
ULONG_PTR = ctypes.c_size_t
INPUT_MOUSE, INPUT_KEYBOARD = 0, 1
KEY_EXTENDED, KEY_UP, KEY_UNICODE = 0x0001, 0x0002, 0x0004
MOUSE_LEFT_DOWN, MOUSE_LEFT_UP, MOUSE_WHEEL = 0x0002, 0x0004, 0x0800
WHEEL_DELTA = 120
KEYS = {
    "backspace": 0x08, "tab": 0x09, "enter": 0x0D, "escape": 0x1B, "space": 0x20,
    "pageup": 0x21, "pagedown": 0x22, "end": 0x23, "home": 0x24,
    "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28, "delete": 0x2E,
}
MODIFIERS = {"shift": 0x10, "ctrl": 0x11, "alt": 0x12}


class MouseInput(ctypes.Structure):
    _fields_ = [
        ("dx", w.LONG), ("dy", w.LONG), ("data", w.DWORD), ("flags", w.DWORD),
        ("time", w.DWORD), ("extra_info", ULONG_PTR),
    ]


class KeyboardInput(ctypes.Structure):
    _fields_ = [
        ("vk", w.WORD), ("scan", w.WORD), ("flags", w.DWORD),
        ("time", w.DWORD), ("extra_info", ULONG_PTR),
    ]


class InputData(ctypes.Union):
    _fields_ = [("mouse", MouseInput), ("keyboard", KeyboardInput)]


class Input(ctypes.Structure):
    _fields_ = [("kind", w.DWORD), ("data", InputData)]


def _key(vk: int, flags: int = 0, scan: int = 0) -> Input:
    return Input(INPUT_KEYBOARD, InputData(keyboard=KeyboardInput(vk, scan, flags)))


def _mouse(flags: int, data: int = 0) -> Input:
    return Input(INPUT_MOUSE, InputData(mouse=MouseInput(0, 0, data, flags)))


def _text_events(text: str) -> list[Input]:
    if not 1 <= len(text) <= 2000 or any(ord(char) < 32 for char in text):
        raise ValueError("Provide 1–2000 characters of plain text. Use press_key for control keys.")
    data = text.encode("utf-16-le")
    codes = [int.from_bytes(data[i:i + 2], "little") for i in range(0, len(data), 2)]
    return [_key(0, flags, code) for code in codes
            for flags in (KEY_UNICODE, KEY_UNICODE | KEY_UP)]


def _point_in_box(x: int, y: int, box: tuple[int, int, int, int]) -> bool:
    left, top, right, bottom = box
    return left <= x < right and top <= y < bottom


class WindowsDesktop:
    def __init__(self):
        if sys.platform != "win32":
            raise DesktopError("The desktop adapter requires an interactive Windows 10/11 session.")
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        self._enum_proc = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)

        signatures = (
            ("EnumWindows", w.BOOL, self._enum_proc, w.LPARAM),
            ("GetAncestor", w.HWND, w.HWND, w.UINT),
            ("GetClientRect", w.BOOL, w.HWND, ctypes.POINTER(w.RECT)),
            ("ClientToScreen", w.BOOL, w.HWND, ctypes.POINTER(w.POINT)),
            ("GetWindowTextLengthW", ctypes.c_int, w.HWND),
            ("GetWindowTextW", ctypes.c_int, w.HWND, w.LPWSTR, ctypes.c_int),
            ("GetWindowThreadProcessId", w.DWORD, w.HWND, ctypes.POINTER(w.DWORD)),
            ("IsWindow", w.BOOL, w.HWND),
            ("IsWindowVisible", w.BOOL, w.HWND),
            ("IsIconic", w.BOOL, w.HWND),
            ("GetForegroundWindow", w.HWND),
            ("WindowFromPoint", w.HWND, w.POINT),
            ("SetCursorPos", w.BOOL, ctypes.c_int, ctypes.c_int),
            ("SetThreadDpiAwarenessContext", ctypes.c_void_p, ctypes.c_void_p),
            ("SendInput", w.UINT, w.UINT, ctypes.POINTER(Input), ctypes.c_int),
        )
        for name, result, *args in signatures:
            function = getattr(user32, name)
            function.argtypes, function.restype = args, result
        self._user32 = user32

    @contextmanager
    def _physical_pixels(self):
        change_context = self._user32.SetThreadDpiAwarenessContext
        previous = change_context(PM_AWARE_V2)
        if not previous:
            raise DesktopError("Could not establish per-monitor Windows DPI coordinates.")
        try:
            yield
        finally:
            change_context(previous)

    def _window(self, handle: int) -> Window:
        api = self._user32
        if not 0 < handle < (1 << (ctypes.sizeof(w.HWND) * 8)):
            raise DesktopError("Invalid Windows handle.")
        if not api.IsWindow(handle) or api.GetAncestor(handle, GA_ROOT) != handle:
            raise DesktopError("The selected top-level window is no longer available.")
        if not api.IsWindowVisible(handle) or api.IsIconic(handle):
            raise DesktopError("Restore the selected window before capturing it.")
        title = ctypes.create_unicode_buffer(api.GetWindowTextLengthW(handle) + 1)
        api.GetWindowTextW(handle, title, len(title))
        process = w.DWORD()
        thread_id = api.GetWindowThreadProcessId(handle, ctypes.byref(process))
        if not thread_id:
            raise DesktopError("The selected window closed.")
        return Window(handle, process.value, thread_id, title.value)

    def windows(self) -> list[Window]:
        windows = []

        @self._enum_proc
        def visit(handle, _parameter):
            try:
                window = self._window(handle)
            except DesktopError:
                return True
            if window.title:
                windows.append(window)
            return True

        if not self._user32.EnumWindows(visit, 0):
            raise DesktopError("Could not enumerate windows in the interactive desktop.")
        return windows

    def _state(self, handle: int) -> WindowState:
        window = self._window(handle)
        api = self._user32
        client = w.RECT()
        origin = w.POINT()
        if not (api.GetClientRect(handle, ctypes.byref(client))
                and api.ClientToScreen(handle, ctypes.byref(origin))):
            raise DesktopError("Could not locate the selected window's client area.")
        if client.right <= 0 or client.bottom <= 0:
            raise DesktopError("The selected window has an empty client area.")
        x, y = origin.x, origin.y
        return WindowState(window, (x, y, x + client.right, y + client.bottom))

    def capture(self, handle: int):
        with self._physical_pixels():
            state = self._state(handle)
            try:
                image = ImageGrab.grab(window=handle)
            except OSError as error:
                raise DesktopError(f"Windows could not capture the selected window: {error}") from error

            if self._state(handle).placement != state.placement:
                raise DesktopError("The window moved or resized during capture. Capture it again.")
            left, top, right, bottom = state.bounds
            # Pillow's HWND capture is the client area. DPI-virtualized windows
            # can produce the same view at a different scale.
            width, height = right - left, bottom - top
            if abs(image.width * height - image.height * width) > max(width, height):
                raise DesktopError("Window capture and client geometry do not agree.")
            if image.getbbox() is None:
                raise DesktopError("Windows returned a blank capture for this app.")
            return state, image

    def _foreground(self, state: WindowState):
        handle = state.window.handle
        if self._state(handle).placement != state.placement:
            raise DesktopError("The window placement changed. Take another observation.")
        if self._user32.GetForegroundWindow() != handle:
            raise DesktopError("Focus the observed window and capture it again before sending input.")

    def _move_cursor(self, state: WindowState, x: int, y: int):
        self._foreground(state)
        api = self._user32
        hit = api.WindowFromPoint(w.POINT(x, y))
        if (not _point_in_box(x, y, state.bounds)
                or not hit or api.GetAncestor(hit, GA_ROOT) != state.window.handle):
            raise DesktopError("The point is not visible in the target window.")
        if not api.SetCursorPos(x, y):
            raise DesktopError("Windows could not move the mouse to the target.")
        self._foreground(state)

    def _send(self, events: list[Input]):
        batch = (Input * len(events))(*events)
        inserted = self._user32.SendInput(len(events), batch, ctypes.sizeof(Input))
        if inserted != len(events):
            raise DesktopError(f"Windows accepted {inserted}/{len(events)} input events. Observe before retrying.")

    def click(self, state: WindowState, x: int, y: int):
        with self._physical_pixels():
            self._move_cursor(state, x, y)
            self._send([_mouse(MOUSE_LEFT_DOWN), _mouse(MOUSE_LEFT_UP)])

    def scroll(self, state: WindowState, x: int, y: int, ticks: int):
        if ticks == 0 or abs(ticks) > 30:
            raise ValueError("Use a nonzero scroll count between -30 and 30.")
        with self._physical_pixels():
            self._move_cursor(state, x, y)
            self._send([_mouse(MOUSE_WHEEL, ticks * WHEEL_DELTA)])

    def fill(self, state: WindowState, x: int, y: int, text: str, replace: bool):
        characters = _text_events(text)
        events = [_mouse(MOUSE_LEFT_DOWN), _mouse(MOUSE_LEFT_UP)]
        if replace:
            ctrl = MODIFIERS["ctrl"]
            events.extend((_key(ctrl), _key(ord("A")), _key(ord("A"), KEY_UP), _key(ctrl, KEY_UP)))
        events.extend(characters)
        with self._physical_pixels():
            self._move_cursor(state, x, y)
            self._send(events)

    def type_text(self, state: WindowState, text: str):
        events = _text_events(text)
        with self._physical_pixels():
            self._foreground(state)
            self._send(events)

    def press_key(self, state: WindowState, chord: str):
        *modifiers, key = chord.lower().strip().split("+")
        if (len(modifiers) != len(set(modifiers))
                or any(modifier not in MODIFIERS for modifier in modifiers)):
            raise ValueError("Use any combination of ctrl, shift and alt without repeats.")
        vk = ord(key.upper()) if len(key) == 1 and key.isascii() and key.isalnum() else KEYS.get(key)
        if vk is None:
            raise ValueError(f"Unsupported key: {key!r}")
        flags = KEY_EXTENDED if 0x21 <= vk <= 0x2E and key in KEYS else 0
        modifier_keys = [MODIFIERS[modifier] for modifier in modifiers]
        events = [_key(code) for code in modifier_keys]
        events.extend((_key(vk, flags), _key(vk, flags | KEY_UP)))
        events.extend(_key(code, KEY_UP) for code in reversed(modifier_keys))
        with self._physical_pixels():
            self._foreground(state)
            self._send(events)
