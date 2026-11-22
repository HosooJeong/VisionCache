"""Compare full and adaptive observations in a disposable Hwp 2020 document."""

import argparse
from collections import Counter
from datetime import datetime
import json
import os
from pathlib import Path
from statistics import median
import time

from PIL import Image

from visioncache.session import DesktopError, WindowSession
from visioncache.windows import WindowsDesktop

from .run import DEFAULT_MODEL, RATES, InvalidAction, call_api, price, read_action, read_usage, write_csv

EDITOR_SIZE = (960, 1040)
DIALOG_SIZE = (581, 642)
DIALOG_TITLE = "편집 용지"
PROMPTS = (
    "Click the Landscape option (가로) in the Edit Paper dialog. Do not confirm yet.",
    "Click the Set button (설정) to apply the selected paper settings.",
    "Open Print Preview (미리 보기) using the small preview button beside Print on the toolbar.",
    "Read the preview. Return orientation (landscape or portrait), total pages from the status "
    "line, and whether all four outer borders of the table are visible (complete or clipped). "
    "Use done with text in the format orientation|page_count|complete_or_clipped. "
    "If an item is illegible, put unknown in its place.",
)
INSTRUCTIONS = (
    "Read only the supplied screenshots of the Korean Hwp 2020 app. "
    "For requested clicks return action=click, the image index, and image-local x,y pixels. "
    "Use empty text for clicks. Image 0 is the entire current surface. "
    "If asked to report, return action=done, image=0, x=0, y=0, and the requested text. "
    "Never print or save anything."
)
# Known clickable controls in the observed Hwp 2020 layout. Reject model clicks
# outside these regions to keep this experiment within the temporary document.
CLICK_BOXES = ((132, 183, 175, 227), (488, 39, 568, 75), (174, 142, 190, 161))


def is_preview(image):
    r, g, b = image.getpixel((10, 34))
    return g > r + 30 and r > b + 30


def wait_for(session, window, title, preview=None):
    deadline = time.perf_counter() + 3
    last_error = None
    previous_view = None
    while time.perf_counter() < deadline:
        try:
            view = session.capture(window, "full", follow_owned=True)
        except DesktopError as error:
            last_error = error
        else:
            image = view.frames[0].image
            if view.state.window.title == title and (preview is None or is_preview(image) == preview):
                expected_size = DIALOG_SIZE if title == DIALOG_TITLE else EDITOR_SIZE
                if image.size != expected_size:
                    raise RuntimeError(f"Expected {expected_size} for {title}, got {image.size}.")
                # Hwp may report the new foreground HWND before its controls paint.
                view_pixels = image.resize((80, 80), Image.Resampling.BILINEAR).tobytes()
                if view_pixels == previous_view:
                    return view
                previous_view = view_pixels
            else:
                previous_view = None
        time.sleep(0.08)
    raise RuntimeError(f"Window did not reach the expected {title} state.") from last_error


def reset_document(session, window, title):
    view = session.capture(window, "full", follow_owned=True)
    if view.state.window.title == DIALOG_TITLE:
        session.press_key(view.id, "escape")
        view = wait_for(session, window, title)
    if view.state.window.title != title or view.frames[0].image.size != EDITOR_SIZE:
        raise RuntimeError("Expected the selected disposable Hwp editor.")
    if is_preview(view.frames[0].image):
        session.click(view.id, 0, 66, 39)
        view = wait_for(session, window, title, preview=False)
    session.press_key(view.id, "f7")
    dialog = wait_for(session, window, DIALOG_TITLE)
    session.click(dialog.id, 0, 79, 204)  # Restore portrait for every trial.
    dialog = wait_for(session, window, DIALOG_TITLE)
    session.click(dialog.id, 0, 529, 56)
    wait_for(session, window, title, preview=False)


def check_preview(image):
    # A known 2x2 fixture has the outer table lines at these four sample points
    # in the 960x1040 landscape preview at page fit.
    border_samples = ((145, 354), (145, 382), (107, 361), (562, 361))
    return is_preview(image) and all(max(image.getpixel(point)) < 112 for point in border_samples)


def run_trial(desktop, handle, title, mode, repetition, key, cache_mode, trace):
    session = WindowSession(desktop)
    probe = WindowSession(desktop)
    reset_document(probe, handle, title)
    start = time.perf_counter()
    initial = wait_for(probe, handle, title, preview=False)
    probe.press_key(initial.id, "f7")

    cost = 0.0
    calls = 0
    tokens = Counter()
    failure = None
    answer = None
    stages = ((DIALOG_TITLE, None), (DIALOG_TITLE, None), (title, False), (title, True))
    for index, (expected_title, expected_preview) in enumerate(stages):
        try:
            wait_for(probe, handle, expected_title, expected_preview)
            state = session.capture(handle, mode, follow_owned=True)
        except RuntimeError as error:
            failure = str(error)
            break
        started = time.perf_counter()
        response = call_api(key, DEFAULT_MODEL, state.frames, PROMPTS[index], "", 45,
                            instructions=INSTRUCTIONS, cache_mode=cache_mode)
        calls += 1
        usage = read_usage(response)
        if usage is None:
            raise RuntimeError("API response did not report usage.")
        request_cost = price(usage, RATES[DEFAULT_MODEL])
        cost += request_cost
        tokens.update(usage)
        action = None
        try:
            action = read_action(response)
        except InvalidAction as error:
            parts = [
                block.get("text", "") for item in response.get("output", [])
                if item.get("type") == "message"
                for block in item.get("content", []) if block.get("type") == "output_text"
            ]
            failure = f"{error} Output: {''.join(parts)[:240]}"
        trace.write(json.dumps({
            "repetition": repetition, "mode": mode, "step": index + 1,
            "surface": "dialog" if expected_title == DIALOG_TITLE else "editor",
            "effective_mode": state.effective_mode,
            "image_sizes": [frame.image.size for frame in state.frames],
            "action": action, "error": failure, "api_seconds": time.perf_counter() - started,
            "usage": usage, "api_cost_usd": request_cost,
        }, ensure_ascii=False) + "\n")
        trace.flush()
        if action is None:
            break
        if index == len(CLICK_BOXES):
            answer = action["text"]
            final = wait_for(probe, handle, title, preview=True)
            if action["action"] != "done" or answer.strip().lower() != "landscape|1|complete":
                failure = "Incorrect preview report"
            elif not check_preview(final.frames[0].image):
                failure = "Preview did not match the fixture layout"
            break

        if action["action"] != "click" or not 0 <= action["image"] < len(state.frames):
            failure = "Expected a click"
            break
        try:
            x, y = state.frames[action["image"]].source_point(action["x"], action["y"])
        except ValueError:
            failure = "Click was outside the supplied image"
            break
        left, top, right, bottom = CLICK_BOXES[index]
        if not (left <= x < right and top <= y < bottom):
            failure = "Click did not target the requested control"
            break
        session.click(state.id, action["image"], action["x"], action["y"])

    return {
        "mode": mode, "repetition": repetition, "success": failure is None,
        "detail": failure, "answer": answer, "api_calls": calls, "api_cost_usd": cost,
        "seconds": time.perf_counter() - start, **tokens,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--window", type=int, required=True, help="Handle of the disposable Hwp test window.")
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--no-cache", action="store_true", help="Disable prompt cache for both modes.")
    args = parser.parse_args()
    cache_mode = "explicit" if args.no_cache else "implicit"
    api_key = os.environ.get("OPENAI_API_KEY", "").strip()
    if not api_key or args.repeats < 1:
        parser.error("Set OPENAI_API_KEY and use a positive repeat count.")
    desktop = WindowsDesktop()
    targets = [window for window in desktop.windows() if window.handle == args.window]
    if len(targets) != 1 or not targets[0].title.startswith("빈 문서") or not targets[0].title.endswith("한글"):
        parser.error("Select the new, unsaved Hwp test document.")
    title = targets[0].title
    output = Path(__file__).resolve().parent / "results" / f"hwp-{datetime.now():%Y%m%d-%H%M%S-%f}"
    output.mkdir(parents=True)
    print(f"Results: {output}", flush=True)
    trials = []
    modes = ["full", "adaptive_delta"]
    with (output / "turns.jsonl").open("w", encoding="utf-8") as trace:
        for repetition in range(1, args.repeats + 1):
            order = modes if repetition % 2 else modes[::-1]
            for mode in order:
                print(f"Running {mode} #{repetition}", flush=True)
                try:
                    trial = run_trial(desktop, args.window, title, mode, repetition, api_key, cache_mode, trace)
                except (DesktopError, RuntimeError) as error:
                    raise SystemExit(f"Benchmark stopped: {error}") from error
                trials.append(trial)
                write_csv(output / "trials.csv", trials)
                print(json.dumps(trial, ensure_ascii=False), flush=True)
    summary = []
    for mode in modes:
        runs = [trial for trial in trials if trial["mode"] == mode]
        completed = [trial["seconds"] for trial in runs if trial["success"]]
        total = sum(trial["api_cost_usd"] for trial in runs)
        summary.append({
            "model": DEFAULT_MODEL, "cache": "off" if args.no_cache else "implicit", "mode": mode, "attempts": len(runs), "successes": len(completed),
            "api_calls": sum(trial["api_calls"] for trial in runs), "api_cost_usd": total,
            "usd_per_success": total / len(completed) if completed else None,
            "median_completion_seconds": median(completed) if completed else None,
        })
    write_csv(output / "summary.csv", summary)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
