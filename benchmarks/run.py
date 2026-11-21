"""Compare three screenshot policies on an isolated Windows stock-intake fixture."""

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor, TimeoutError as FutureTimeout
import csv
from datetime import datetime
from io import BytesIO
import json
import math
import os
from pathlib import Path
from statistics import median
import sys
import time
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

from .fixture import Fixture, FixtureClosed, make_task
from visioncache.observation import MODES, observe

SCENARIOS = ("stock-entry", "recover-bin")
DEFAULT_MODEL = "gpt-6.2-sol"
# Standard, short-context, 30-minute cache, USD per million tokens.
RATES = {
    "gpt-6.2-sol": (2.0, 0.10, 2.50, 10.0),
    "gpt-6.2-astra": (8.0, 0.40, 10.00, 40.0),
    "gpt-6-astra": (10.0, 1.00, 12.50, 50.0),
}
RATE_KEYS = ("input", "cached_input", "cache_write", "output")
USAGE_KEYS = ("input_tokens", "cached_input_tokens", "cache_write_tokens", "output_tokens")
API_URL = "https://api.openai.com/v1/responses"
PRICING_URL = "https://developers.openai.com/api/docs/pricing"

INSTRUCTIONS = """Operate the stock intake app using only the supplied screenshot images.
Return exactly one action. Use fill on an input field to replace its text. Use click on a button.
For fill or click, choose the image index and coordinates measured in that image's own
pixels, from the top left. Image 0 always shows the whole window. If a region is shown,
you may use its local pixel coordinates too. Use an empty text for a click.
Use done only if the screen shows the task is complete or you cannot proceed.
For done use image=0, x=0, y=0 and empty text. Never guess a receipt code."""
ACTION_FORMAT = {
    "type": "json_schema",
    "name": "ui_action",
    "strict": True,
    "schema": {
        "type": "object",
        "properties": {
            "action": {"type": "string", "enum": ["fill", "click", "done"]},
            "image": {"type": "integer"},
            "x": {"type": "integer"},
            "y": {"type": "integer"},
            "text": {"type": "string"},
        },
        "required": ["action", "image", "x", "y", "text"],
        "additionalProperties": False,
    },
}


class ApiError(Exception):
    pass


class InvalidAction(Exception):
    pass


def encode_frame(frame):
    buffer = BytesIO()
    frame.image.save(buffer, format="PNG")
    data = base64.b64encode(buffer.getvalue()).decode("ascii")
    return {"type": "input_image", "image_url": f"data:image/png;base64,{data}", "detail": "high"}


def call_api(key, model, frames, task, previous_action, timeout):
    content = [{"type": "input_text", "text": f"Task: {task.instruction}\n{previous_action}"}]
    for index, frame in enumerate(frames):
        width, height = frame.image.size
        content.append({
            "type": "input_text",
            "text": f"Image {index}: {frame.label}, {width}x{height} pixels.",
        })
        content.append(encode_frame(frame))

    payload = {
        "model": model,
        "instructions": INSTRUCTIONS,
        "input": [{"role": "user", "content": content}],
        "text": {"format": ACTION_FORMAT},
        "reasoning": {"effort": "low"},
        "service_tier": "default",
        "prompt_cache_options": {"mode": "implicit", "ttl": "30m"},
        "max_output_tokens": 512,
        "store": False,
    }
    request = Request(
        API_URL, data=json.dumps(payload).encode("utf-8"),
        headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json"},
    )
    try:
        with urlopen(request, timeout=timeout) as response:
            return json.load(response)
    except HTTPError as error:
        # Keep keys and provider-returned request data out of logs.
        with error:
            request_id = error.headers.get("x-request-id", "unavailable")
        raise ApiError(f"HTTP {error.code} (request ID: {request_id})") from None
    except (URLError, OSError, ValueError) as error:
        raise ApiError(f"API request failed ({type(error).__name__}).") from None


def read_usage(response):
    raw = response.get("usage")
    if not isinstance(raw, dict):
        return None
    details = raw.get("input_tokens_details") or {}
    usage = {
        "input_tokens": raw.get("input_tokens"),
        "cached_input_tokens": details.get("cached_tokens", 0),
        "cache_write_tokens": details.get("cache_write_tokens", 0),
        "output_tokens": raw.get("output_tokens"),
    }
    if any(type(value) is not int or value < 0 for value in usage.values()):
        return None
    if usage["cached_input_tokens"] + usage["cache_write_tokens"] > usage["input_tokens"]:
        return None
    return usage


def price(usage, rates):
    total, cached, write, output = (usage[name] for name in USAGE_KEYS)
    return ((total - cached - write) * rates[0] + cached * rates[1]
            + write * rates[2] + output * rates[3]) / 1_000_000


def read_action(response):
    if response.get("status") != "completed":
        details = response.get("incomplete_details") or {}
        raise InvalidAction(f"Model response {response.get('status')}: {details.get('reason', '')}")
    parts = [part["text"] for item in response.get("output", [])
             if item.get("type") == "message"
             for part in item.get("content", []) if part.get("type") == "output_text"]
    try:
        action = json.loads("".join(parts))
    except ValueError:
        raise InvalidAction("No valid action JSON was returned.") from None
    if (not isinstance(action, dict) or set(action) != {"action", "image", "x", "y", "text"}
            or action["action"] not in ("click", "fill", "done")
            or any(type(action[name]) is not int for name in ("image", "x", "y"))
            or not isinstance(action["text"], str) or len(action["text"]) > 80):
        raise InvalidAction("Model returned an invalid action.")
    return action


def map_action(action, frames):
    if not 0 <= action["image"] < len(frames):
        raise InvalidAction("Image index was outside the observation.")
    frame = frames[action["image"]]
    width, height = frame.image.size
    if not (0 <= action["x"] < width and 0 <= action["y"] < height):
        raise InvalidAction("Coordinates were outside the selected image.")
    native_x, native_y = frame.source_point(action["x"], action["y"])
    return {
        **action,
        "x": native_x,
        "y": native_y,
    }


def smoke_actions(task):
    yield "fill", 160, 269, task.sku
    yield "fill", 160, 351, task.quantity
    yield "fill", 160, 433, task.preferred_bin
    yield "click", 155, 515, ""
    if task.preferred_bin != task.active_bin:
        yield "fill", 160, 433, task.active_bin
        yield "click", 155, 515, ""
    yield "fill", 745, 433, task.receipt
    yield "click", 702, 515, ""


def scripted_action(script, frames):
    kind, native_x, native_y, value = next(script)
    frame = frames[0]
    width, height = frame.image.size
    return {
        "action": kind, "image": 0, "text": value,
        "x": native_x * width // frame.box[2], "y": native_y * height // frame.box[3],
    }


def wait_for_api(future, fixture):
    while True:
        try:
            return future.result(timeout=0.05)
        except FutureTimeout:
            fixture.update()


def run_trial(fixture, pool, args, rates, api_key, scenario, repetition, mode, turn_log):
    task = make_task(scenario, args.seed, repetition)
    fixture.reset(task)
    script = smoke_actions(task)
    previous = None
    previous_action = "No previous action."
    usage_totals = dict.fromkeys(USAGE_KEYS, 0)
    known_cost = 0.0
    usage_complete = not args.smoke
    calls = 0
    steps = 0
    outcome = "step_limit"
    detail = ""
    start = time.perf_counter()

    for step in range(1, args.max_steps + 1):
        steps = step
        turn = {"scenario": scenario, "repetition": repetition, "mode": mode, "step": step}
        try:
            current = fixture.capture()
            frames = observe(mode, current, previous, args.scale)
            previous = current
            turn["frames"] = [
                {"pixels": frame.image.size, "bounds": frame.box, "label": frame.label, "reason": frame.reason}
                for frame in frames
            ]

            if args.smoke:
                action = scripted_action(script, frames)
            else:
                calls += 1
                api_start = time.perf_counter()
                future = pool.submit(call_api, api_key, args.model, frames,
                                     task, previous_action, args.timeout)
                try:
                    response = wait_for_api(future, fixture)
                except (ApiError, FixtureClosed):
                    usage_complete = False
                    raise
                finally:
                    turn["api_seconds"] = time.perf_counter() - api_start

                usage = read_usage(response)
                turn.update(response_id=response.get("id"), model=response.get("model"), usage=usage)
                if usage is None:
                    usage_complete = False
                else:
                    for key, count in usage.items():
                        usage_totals[key] += count
                    turn["api_cost_usd"] = price(usage, rates)
                    known_cost += turn["api_cost_usd"]
                action = read_action(response)

            turn["action"] = action
            if action["action"] == "done":
                outcome = "model_stopped"
                break
            native = map_action(action, frames)
            turn["native_xy"] = [native["x"], native["y"]]
            outcome_message = fixture.apply(native)
            turn["action_result"] = outcome_message
            previous_action = f"Last action in the preceding observation: {json.dumps(action)}. Result: {outcome_message}"
            if fixture.succeeded():
                outcome = "success"
                break
        except (ApiError, FixtureClosed, InvalidAction, RuntimeError) as error:
            outcome = {
                ApiError: "api_error", FixtureClosed: "cancelled",
                InvalidAction: "invalid_action", RuntimeError: "fixture_error",
            }[type(error)]
            detail = str(error)
            turn["error"] = detail
            break
        finally:
            turn_log.write(json.dumps(turn, ensure_ascii=False) + "\n")
            turn_log.flush()

    return {
        "scenario": scenario, "repetition": repetition, "mode": mode,
        "outcome": outcome, "detail": detail, "seconds": time.perf_counter() - start,
        "steps": steps, "api_calls": calls,
        **(usage_totals if not args.smoke else dict.fromkeys(USAGE_KEYS)),
        "known_api_cost_usd": known_cost if not args.smoke else None,
        "api_cost_usd": known_cost if usage_complete else None,
    }


def summarize(rows, scope, mode):
    successes = [row for row in rows if row["outcome"] == "success"]
    count = len(rows)
    success_count = len(successes)
    cost = (sum(row["api_cost_usd"] for row in rows)
            if count and all(row["api_cost_usd"] is not None for row in rows) else None)
    return {
        "scope": scope, "mode": mode, "attempts": count, "successes": success_count,
        "success_rate": success_count / count if count else None,
        "total_api_cost_usd": cost,
        "usd_per_success": cost / success_count if cost is not None and success_count else None,
        "median_completion_seconds": median(row["seconds"] for row in successes) if successes else None,
        "median_attempt_seconds": median(row["seconds"] for row in rows) if rows else None,
        "api_calls": sum(row["api_calls"] for row in rows),
        "unpriced_api_attempts": sum(row["api_calls"] > 0 and row["api_cost_usd"] is None for row in rows),
    }


def write_csv(path, rows):
    if rows:
        temp = path.with_suffix(".tmp")
        with temp.open("w", encoding="utf-8-sig", newline="") as output:
            writer = csv.DictWriter(output, fieldnames=list(rows[0]))
            writer.writeheader()
            writer.writerows(rows)
        temp.replace(path)


def render_summary(summary):
    def display(value, spec):
        return "--" if value is None else format(value, spec)

    print("\nMode           Success       Total USD   USD/success   Median complete (s)")
    for row in summary:
        if row["scope"] != "all":
            continue
        print(f"{row['mode']:15}{row['successes']:>3}/{row['attempts']:<3}"
              f"{display(row['total_api_cost_usd'], '.6f'):>14}"
              f"{display(row['usd_per_success'], '.6f'):>14}"
              f"{display(row['median_completion_seconds'], '.3f'):>20}")


def options():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--smoke", action="store_true", help="Run the real GUI with scripted actions and no API.")
    parser.add_argument("--repeats", type=int, help="Trials per scenario and mode (live: 3, smoke: 1).")
    parser.add_argument("--scenario", choices=SCENARIOS, action="append", help="Select scenarios, repeatable.")
    parser.add_argument("--seed", type=int, default=731)
    parser.add_argument("--scale", type=float, default=0.5, help="Linear scale of the resized screenshots.")
    parser.add_argument("--max-steps", type=int, default=14)
    parser.add_argument("--timeout", type=float, default=60, help="HTTP connect/read timeout in seconds.")
    parser.add_argument("--model", default=DEFAULT_MODEL, help="Vision model supporting the requested API options.")
    parser.add_argument("--rates", nargs=4, type=float, metavar=("INPUT", "CACHED", "WRITE", "OUTPUT"),
                        help="Override the USD per 1M tokens for the selected Standard model.")
    parser.add_argument("--output", type=Path, default=Path(__file__).resolve().parent / "results")
    args = parser.parse_args()
    if args.repeats is None:
        args.repeats = 1 if args.smoke else 3
    if args.repeats < 1 or args.max_steps < 1:
        parser.error("--repeats and --max-steps must be positive")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be positive and finite")
    if not 0.10 <= args.scale < 1.0:
        parser.error("--scale must be between 0.10 (inclusive) and 1.0 (exclusive)")
    if args.rates is not None and any(not math.isfinite(rate) or rate < 0 for rate in args.rates):
        parser.error("--rates must contain finite nonnegative prices")
    if not args.smoke and args.rates is None and args.model not in RATES:
        parser.error("use --rates for a model without built-in prices")
    if not args.smoke and not os.environ.get("OPENAI_API_KEY", "").strip():
        parser.error("set OPENAI_API_KEY for a live benchmark, or use --smoke")
    return args


def main():
    args = options()
    if sys.platform != "win32":
        raise SystemExit("This benchmark requires an interactive Windows desktop.")

    api_key = os.environ.get("OPENAI_API_KEY", "").strip()
    rates = None if args.smoke else (args.rates if args.rates is not None else RATES[args.model])
    scenarios = tuple(dict.fromkeys(args.scenario or SCENARIOS))
    started = datetime.now().astimezone()
    run_dir = args.output / f"{'smoke' if args.smoke else 'live'}-{started:%Y%m%d-%H%M%S-%f}"
    run_dir.mkdir(parents=True)
    print(f"Writing results to {run_dir.resolve()}", flush=True)
    plan = []
    for offset in range(args.repeats):
        for index, scenario in enumerate(scenarios):
            rotation = (offset + index) % len(MODES)
            order = MODES[rotation:] + MODES[:rotation]
            plan.extend((scenario, offset + 1, mode) for mode in order)
    metadata = {
        "kind": "smoke" if args.smoke else "live", "started_at": started.isoformat(),
        "model": None if args.smoke else args.model, "rates_usd_per_million": dict(zip(RATE_KEYS, rates)) if rates else None,
        "pricing_source": PRICING_URL, "rate_card_checked": "2026-11-22",
        "seed": args.seed, "scale": args.scale, "max_steps": args.max_steps,
        "scenarios": scenarios, "modes": MODES, "repetitions": args.repeats, "planned_attempts": len(plan),
    }
    (run_dir / "run.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")

    fixture = Fixture()
    rows = []
    stop = None
    try:
        with ThreadPoolExecutor(max_workers=1) as pool, (run_dir / "turns.jsonl").open(
            "w", encoding="utf-8"
        ) as turn_log:
            for scenario, repetition, mode in plan:
                print(f"[{len(rows) + 1}/{len(plan)}] {scenario} #{repetition}  {mode}", flush=True)
                row = run_trial(fixture, pool, args, rates, api_key, scenario, repetition, mode, turn_log)
                rows.append(row)
                write_csv(run_dir / "trials.csv", rows)
                print(f"  {row['outcome']}  {row['seconds']:.2f}s", flush=True)
                if row["outcome"] in ("api_error", "cancelled", "fixture_error"):
                    stop = row["outcome"]
                    print(f"  {row['detail']}", file=sys.stderr)
                    break
    except (KeyboardInterrupt, FixtureClosed):
        stop = "cancelled"
    finally:
        fixture.close()
        summary = [
            summarize([row for row in rows if row["mode"] == mode
                       and (scope == "all" or row["scenario"] == scope)], scope, mode)
            for scope in ("all", *scenarios)
            for mode in MODES
        ]
        write_csv(run_dir / "summary.csv", summary)
        metadata.update(completed_attempts=len(rows), stop_reason=stop,
                        finished_at=datetime.now().astimezone().isoformat())
        (run_dir / "run.json").write_text(json.dumps(metadata, indent=2), encoding="utf-8")
        render_summary(summary)

    if stop or (args.smoke and any(row["outcome"] != "success" for row in rows)):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())