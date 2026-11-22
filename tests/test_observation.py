from PIL import Image, ImageDraw
import pytest

from visioncache import observe
from visioncache.observation import Frame
from visioncache.session import DesktopError, Window, WindowSession, WindowState


def test_adaptive_rejects_graphics_but_general_delta_keeps_them():
    baseline = Image.new("RGB", (600, 400), "white")
    next_frame = baseline.copy()
    ImageDraw.Draw(next_frame).rectangle((220, 160, 300, 210), fill="black")
    assert len(observe("resized_delta", next_frame, baseline)) == 2
    assert len(observe("adaptive_delta", next_frame, baseline)) == 1


@pytest.mark.parametrize(("color", "reason"), [
    ("#216145", "small_text"),
    ("#ae292e", "red_warning_text"),
])
def test_adaptive_keeps_changed_text_once(color, reason):
    baseline = Image.new("RGB", (600, 400), "white")
    next_frame = baseline.copy()
    draw = ImageDraw.Draw(next_frame)
    for x in range(370, 425, 5):
        draw.rectangle((x, 230, x + 2, 238), fill=color)
    overview, crop = observe("adaptive_delta", next_frame, baseline)
    assert overview.image.size == (300, 200)
    assert crop.reason == reason
    assert crop.box[0] <= 370 < 425 <= crop.box[2]
    assert crop.image.getpixel((370 - crop.box[0], 230 - crop.box[1])) == next_frame.getpixel((370, 230))
    assert observe("adaptive_delta", next_frame, next_frame) == [overview]


def test_full_recovery_and_size_change():
    prior = Image.new("RGB", (400, 300))
    current = Image.new("RGBA", (600, 400), "white")
    assert observe("full", current, prior)[0].image.size == (600, 400)
    assert len(observe("adaptive_delta", current, prior)) == 1
    assert observe("resized", Image.new("RGB", (1, 1)))[0].image.size == (1, 1)
    with pytest.raises(ValueError):
        observe("adaptive_delta", current, prior, scale=float("nan"))


def test_image_local_coordinates_and_bounds():
    frame = Frame(Image.new("RGB", (100, 40)), (300, 150, 500, 230), "region")
    assert frame.source_point(50, 20) == (400, 190)
    with pytest.raises(ValueError):
        frame.source_point(100, 40)


class FakeDesktop:
    def __init__(self):
        self.state = WindowState(Window(1, 10, 20, "Window"), (-200, 30, 1000, 830))
        self.image = Image.new("RGB", (600, 400), "white")
        self.actions = []
        self.failed = False

    def capture(self, handle):
        assert handle == self.state.window.handle
        if self.failed:
            raise DesktopError("Capture failed")
        return self.state, self.image.copy()

    def click(self, state, x, y):
        self.actions.append((state, x, y))


def test_session_maps_capture_scale_to_physical_screen_and_consumes_input():
    desktop = FakeDesktop()
    session = WindowSession(desktop)
    observation = session.capture(1)
    assert session.map_point(observation.id, 0, 150, 100) == (400, 430)
    session.click(observation.id, 0, 150, 100)
    assert desktop.actions == [(desktop.state, 400, 430)]
    with pytest.raises(DesktopError, match="stale"):
        session.click(observation.id, 0, 150, 100)


def test_switch_or_move_window_drops_the_diff_and_invalidates_old_ids():
    desktop = FakeDesktop()
    session = WindowSession(desktop)
    initial = session.capture(1, "full")
    initial.frames[0].image.paste("black", (100, 100, 130, 130))
    desktop.image.paste("black", (100, 100, 130, 130))
    changed = session.capture(1, "resized_delta")
    assert len(changed.frames) == 2
    crop = changed.frames[1]
    local_x, local_y = crop.source_point(36, 36)
    assert session.map_point(changed.id, 1, 36, 36) == (-200 + local_x * 2, 30 + local_y * 2)
    desktop.state = WindowState(Window(2, 11, 21, "Other"), desktop.state.bounds)
    desktop.image.paste("black", (150, 150, 180, 180))
    assert len(session.capture(2, "resized_delta").frames) == 1
    desktop.state = WindowState(desktop.state.window, (0, 50, 1200, 850))
    desktop.image.paste("black", (190, 190, 220, 220))
    assert len(session.capture(2, "resized_delta").frames) == 1
    with pytest.raises(DesktopError, match="stale"):
        session.map_point(changed.id, 0, 20, 20)


def test_failed_capture_invalidates_current_observation():
    desktop = FakeDesktop()
    session = WindowSession(desktop)
    observation = session.capture(1)
    desktop.failed = True
    with pytest.raises(DesktopError, match="failed"):
        session.capture(1)
    with pytest.raises(DesktopError, match="stale"):
        session.map_point(observation.id, 0, 0, 0)


@pytest.mark.parametrize(("foreground", "follow_owned", "expected"), [
    (22, True, 22),   # Nested foreground popup of the target.
    (22, False, 10),  # Exact-window capture stays available.
    (23, True, 10),   # Unrelated window from the same process.
    (24, True, 10),   # Other process is excluded even with an owner link.
    (25, True, 10),   # The observed foreground disappeared.
    (None, True, 10),
])
def test_windows_capture_follows_only_owned_foreground(foreground, follow_owned, expected):
    from types import SimpleNamespace
    from visioncache.windows import WindowsDesktop

    backend = object.__new__(WindowsDesktop)
    owners = {21: 10, 22: 21, 24: 10}
    backend._user32 = SimpleNamespace(
        GetForegroundWindow=lambda: foreground,
        GetWindow=lambda window, _relationship: owners.get(window),
    )

    def read_window(handle):
        if handle == 25:
            raise DesktopError("The window closed.")
        return Window(handle, 8 if handle == 24 else 7, 1, f"Window {handle}")

    backend._window = read_window
    assert backend._capture_target(10, follow_owned) == expected


def test_adaptive_session_keeps_owned_dialog_native_and_restores_overview():
    desktop = FakeDesktop()
    base = desktop.state
    dialog = WindowState(Window(2, 10, 20, "Dialog"), (0, 0, 600, 400))
    selected = base

    def capture(handle, *, follow_owned=False):
        assert handle == base.window.handle
        return (selected if follow_owned else base), desktop.image.copy()

    desktop.capture = capture
    session = WindowSession(desktop)
    first = session.capture(1, follow_owned=True)
    assert first.frames[0].image.size == (300, 200)

    selected = dialog
    popup = session.capture(1, follow_owned=True)
    assert popup.effective_mode == "full"
    assert popup.frames[0].image.size == (600, 400)
    assert session.map_point(popup.id, 0, 150, 100) == (150, 100)
    with pytest.raises(DesktopError, match="stale"):
        session.map_point(first.id, 0, 0, 0)

    selected = base
    back = session.capture(1, follow_owned=True)
    assert back.state == base
    assert back.effective_mode == "adaptive_delta"
    assert [frame.image.size for frame in back.frames] == [(300, 200)]
