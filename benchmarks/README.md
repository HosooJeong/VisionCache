# VisionCache Windows benchmarks

These live API benchmarks use the Python reference implementation of VisionCache. The [main README](../README.md) covers the Rust release and its integration tests.

## Stock intake fixture

The fixture runs the same Windows stock-intake tasks with four capture policies. All model actions are scoped to a disposable test window.

| Mode | Images sent to the model |
| --- | --- |
| `full` | The full test window at its native resolution of 1024×640 |
| `resized` | The full test window scaled to 512×320 by default |
| `resized_delta` | The scaled image plus a native-resolution crop of the changed region |
| `adaptive_delta` | The scaled image plus a native-resolution crop when changes contain small text or red warnings |

Both crop modes send just the scaled window on the first frame, when nothing changes, or when a crop would cover more than 25% of the source. The adaptive policy uses pixel changes and color. It adds a padded native crop when new red text appears or when a changed glyph's stroke height in the scaled image is estimated at six pixels or less. App themes and font sizes can affect detection.

### Run on Windows

Use Python 3.10 or later with Tkinter on an interactive Windows desktop. Leave the test window alone during the run. Close it or press Ctrl+C in the terminal to stop.

```powershell
python -m pip install -e .
python -m benchmarks.run --smoke
```

`--smoke` runs the native Windows capture path and uses scripted input coordinates. It makes no API calls.

For live measurements, set `OPENAI_API_KEY` in the current PowerShell session. The fixture generates synthetic screen content and input values for the API requests.

```powershell
# Run only if the current session needs an API key.
$env:OPENAI_API_KEY = [System.Net.NetworkCredential]::new('', (Read-Host 'OpenAI API key' -AsSecureString)).Password

# Quick run with one trial per scenario and mode.
python -m benchmarks.run --repeats 1

# Default run with three trials per scenario and mode.
python -m benchmarks.run

# Balance the run order across all four modes.
python -m benchmarks.run --repeats 4

# Repeat error recovery and write results to a separate directory.
python -m benchmarks.run --scenario recover-bin --repeats 6 --output .\benchmark-results
```

The default model is `gpt-6.2-sol`. All modes use the same model and request settings, with the current images and previous action on each observation. Change the model with `--model` or the scale with `--scale`. Use `--rates INPUT CACHED WRITE OUTPUT` to override the rates in USD per million tokens. The selected model must support image input, Structured Outputs, low reasoning effort, and the 30-minute prompt cache setting on the Responses API.

### Read the results

Each run saves results to `benchmarks/results/live-…/` or `benchmarks/results/smoke-…/`.

| File | Contents |
| --- | --- |
| `summary.csv` | Success rate, usage-based USD cost, cost per success, and median completion time by mode and scenario |
| `trials.csv` | Outcome, elapsed time, token usage, API calls, and cost per trial |
| `turns.jsonl` | Image dimensions and bounds, crop reason, action and mapped coordinates, API response ID, usage, and API round-trip time per turn |
| `run.json` | Model, rates, seed, repetition count, planned and executed trial counts, and interruption status |

- A trial succeeds when the app saves the correct SKU, quantity, bin, and confirmation code. The agent reads the confirmation code from the screen in both scenarios. In `recover-bin`, it also uses the fallback bin shown by the app. Each repetition shares the same task values across all four modes and rotates their order.
- The cost calculation applies the run's rates to the actual usage returned by the API, with separate rates for uncached input, cache hits, cache writes, and output. Cost per success divides the cost of all attempts by the number of successes. If any request lacks usage, the affected total is blank and `known_api_cost_usd` records the portion with available usage. [API pricing](https://developers.openai.com/api/docs/pricing) · [Prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching)
- The clock starts with the first capture once the window is ready and stops at final validation. This includes image processing, API requests, and GUI input. `median_completion_seconds` uses successful trials and `median_attempt_seconds` uses all trials.
- The stock-intake fixture deliberately uses small text for the confirmation code and the error message. Results reflect this window and these two tasks. [Stock intake results](reference-2026-11-22.csv)

## Word processor print preview

In Hancom Hwp 2020, open a new unsaved document and insert a default 2×2 table. Keep the window in the foreground with a 960×1040 client area and set print preview to fit the page. Use the VisionCache MCP tool `list_windows` to obtain the test window's handle.

```powershell
$windowHandle = [int](Read-Host 'Test document window handle')
python -m benchmarks.run_hwp --window $windowHandle --repeats 3 --no-cache
```

The runner resets the document to portrait before each trial. It opens the Edit Paper dialog and asks the agent to select landscape, apply it, open print preview, and report the page count and whether the full table border is visible. The runner permits clicks in the three control regions for this task. Recheck the regions if the DPI or window layout changes. After a screen transition, the runner waits for two matching observations before calling the model.

Both modes start with the main window handle and follow its foreground dialog. Adaptive capture sends the selected dialog at native resolution. Most of the input savings in this test come from the edit and preview windows. Completion time includes model calls and UI processing. Timing begins after the reset to portrait. By default the runner requests implicit prompt caching. The command above uses explicit cache mode and the reported run recorded zero cache hits and writes.

The reference CSV includes the preliminary trials and the stabilized comparison. Token columns sum the API-reported usage across all attempts in each row. In the stabilized comparison each mode succeeded in 3/3 trials. Input usage fell from 12,564 to 7,266 tokens across the three runs, with 363 output tokens for each mode. [Hwp 2020 results](reference-hwp-2026-11-22.csv)
