"""Screenshot observations shared by live runs and the local smoke test."""

from dataclasses import dataclass
import math
from typing import Literal

from PIL import Image, ImageChops

MODES = ("full", "resized", "resized_delta", "adaptive_delta")
Mode = Literal["full", "resized", "resized_delta", "adaptive_delta"]
CHANGE_THRESHOLD = 18
CROP_PADDING = 36
MAX_CROP_FRACTION = 0.25
SMALL_TEXT_HEIGHT = 6  # Estimated visible glyph height in overview pixels.


@dataclass(frozen=True)
class Frame:
    image: Image.Image
    box: tuple[int, int, int, int]
    label: str
    reason: str | None = None

    def source_point(self, x: int, y: int) -> tuple[int, int]:
        """Map image-local pixels into the original capture's pixel space."""
        width, height = self.image.size
        if not (0 <= x < width and 0 <= y < height):
            raise ValueError("Point is outside the supplied image.")
        left, top, right, bottom = self.box
        return left + x * (right - left) // width, top + y * (bottom - top) // height


def text_bands(ink):
    """Find bands with several separated strokes in each row, ignoring borders."""
    width, height = ink.size
    pixels = ink.tobytes()
    start = None
    for y in range(height + 1):
        row = pixels[y * width:(y + 1) * width]
        # Include a stroke at the first column in the transition count.
        strokes = (b"\x00" + row).count(b"\x00\xff")
        if strokes >= 3 and row.count(b"\xff") >= 7:
            if start is None:
                start = y
        elif start is not None:
            if y - start >= 5:
                left, _, right, _ = ink.crop((0, start, width, y)).getbbox()
                if right - left >= 24:
                    top, bottom = start, y
                    while top and ink.crop((left, top - 1, right, top)).getbbox():
                        top -= 1
                    while bottom < height and ink.crop((left, bottom, right, bottom + 1)).getbbox():
                        bottom += 1
                    yield left, top, right, bottom
            start = None


def adaptive_region(current, mask, bounds, scale):
    """Use screenshot pixels to select changed small text or red warning text."""
    left, top, right, bottom = bounds
    # Measure full glyph heights even when only part of an existing word changed.
    top, bottom = max(0, top - 12), min(current.height, bottom + 12)
    left, right = max(0, left - 12), min(current.width, right + 12)
    region = left, top, right, bottom
    changed = mask.crop(region)
    patch = current.crop(region)

    red, green, blue = patch.split()
    red_over_green = ImageChops.subtract(red, green).point(lambda n: 255 if n >= 75 else 0)
    red_over_blue = ImageChops.subtract(red, blue).point(lambda n: 255 if n >= 75 else 0)
    # Suppress colored subpixel fringes on ordinary dark text.
    green_near_blue = ImageChops.difference(green, blue).point(lambda n: 255 if n <= 25 else 0)
    red_ink = ImageChops.multiply(ImageChops.multiply(red_over_green, red_over_blue), green_near_blue)
    candidates = list(text_bands(ImageChops.multiply(changed, red_ink)))
    reason = "red_warning_text"
    if not candidates:
        dark = patch.convert("L").point(lambda n: 255 if n < 158 else 0)
        # Ignore saturated blue focus outlines in the capture.
        text_color = ImageChops.subtract(blue, red).point(lambda n: 255 if n < 80 else 0)
        dark = ImageChops.multiply(dark, text_color)
        changed_dark = ImageChops.multiply(changed, dark)
        candidates = [
            box for box in text_bands(dark)
            if (box[3] - box[1]) * scale <= SMALL_TEXT_HEIGHT
            and changed_dark.crop(box).histogram()[255] >= 16
        ]
        reason = "small_text"
    if not candidates:
        return None, None

    return (
        left + min(box[0] for box in candidates),
        top + min(box[1] for box in candidates),
        left + max(box[2] for box in candidates),
        top + max(box[3] for box in candidates),
    ), reason


def observe(mode: Mode, current: Image.Image, previous: Image.Image | None = None,
            scale: float = 0.5) -> list[Frame]:
    """Return an overview and the native-resolution regions chosen by a policy."""
    if mode not in MODES:
        raise ValueError(f"Unknown observation mode: {mode}")
    if not math.isfinite(scale) or not 0.1 <= scale < 1.0:
        raise ValueError("Scale must be at least 0.1 and less than 1.")
    if current.mode != "RGB":
        current = current.convert("RGB")
    width, height = current.size
    screen_box = (0, 0, width, height)
    if mode == "full":
        return [Frame(current, screen_box, "Full view")]

    overview = current.resize(
        (max(1, round(width * scale)), max(1, round(height * scale))), Image.Resampling.LANCZOS
    )
    frames = [Frame(overview, screen_box, "Full view")]
    if mode == "resized" or previous is None or previous.size != current.size:
        return frames

    if previous.mode != "RGB":
        previous = previous.convert("RGB")
    delta = ImageChops.difference(current, previous)
    mask = delta.convert("L").point(lambda value: 255 if value >= CHANGE_THRESHOLD else 0)
    box = mask.getbbox()
    if box is None:
        return frames

    reason = "pixel_change"
    if mode == "adaptive_delta":
        box, reason = adaptive_region(current, mask, box, scale)
        if box is None:
            return frames

    left, top, right, bottom = box
    crop = (
        max(0, left - CROP_PADDING), max(0, top - CROP_PADDING),
        min(width, right + CROP_PADDING), min(height, bottom + CROP_PADDING),
    )
    if (crop[2] - crop[0]) * (crop[3] - crop[1]) <= width * height * MAX_CROP_FRACTION:
        frames.append(Frame(current.crop(crop), crop, "Changed region", reason))
    return frames