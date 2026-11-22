# VisionCache

VisionCache is a local MCP server for desktop computer use. It gives an agent a low-resolution view of a selected window and adds native-resolution detail when a change includes small text or a red warning. It also maps image coordinates back to the window for mouse and keyboard input.

[한국어](README.ko.md) · [macOS design](docs/macos-design.md) · [Benchmarks](benchmarks/README.md)

| Component | Status |
| --- | --- |
| Windows desktop MCP | Implemented for Windows 10/11. Verified on Windows 10 with an isolated fixture and Hwp 2020. |
| Python image policy | Accepts Pillow images from an existing capture pipeline. |
| macOS desktop MCP | Design proposal for macOS 14 and later. Native capture and input are pending implementation and on-device verification. |

## Quick start

Run this from an interactive Windows desktop with Python 3.10 or later.

```powershell
git clone https://github.com/HosooJeong/VisionCache.git
cd VisionCache
python -m venv .venv
& .\.venv\Scripts\python.exe -m pip install -e ".[mcp]"
& .\.venv\Scripts\visioncache-mcp.exe --help
```

Add VisionCache as a local **stdio** server in your MCP client. Replace the example path with the absolute path to the executable in your virtual environment. For clients that use an `mcpServers` object, the configuration looks like this.

```json
{
  "mcpServers": {
    "visioncache": {
      "command": "C:\\path\\to\\VisionCache\\.venv\\Scripts\\visioncache-mcp.exe",
      "args": ["--scale", "0.5"]
    }
  }
}
```

The server uses no model API key. It returns PNGs and metadata to the connected client, which decides where to send them. Set the agent up to call VisionCache for desktop observation and input.

## Observe and interact

1. Call `list_windows` and select a window handle.
2. Call `observe_window(window=<handle>)`. The response contains JSON metadata followed by PNG images. Image `0` is the full client area, at the requested mode's resolution. Additional PNGs are crops. `images[].source_box` gives each image's bounds in original capture pixels.
3. Use `observation_id` and coordinates relative to the chosen image for `click`, `scroll`, or `fill`. Use `type_text` or `press_key` for the current focus. `map_point` returns Windows screen pixels for an external input adapter.
4. Take a new observation after an input. Input requires the observed window to be in the foreground. Layout changes can invalidate previously observed coordinates.

Keep requesting the original handle when the app opens an owned foreground dialog. VisionCache shows the dialog, then returns to the main window when it closes. The response distinguishes `requested_window` from the captured `window`. Pass `follow_owned=false` to capture the requested window directly.

| Mode | Images returned |
| --- | --- |
| `full` | The current surface at native resolution |
| `resized` | A scaled view of the current surface |
| `resized_delta` | A scaled view plus a bounded crop from the changed area |
| `adaptive_delta` | A scaled view plus a native crop for newly changed small text or red warning text. An owned foreground dialog is captured at native resolution. |

The default uses pixel heuristics for character height and red warning text. Fonts, themes, and display scaling affect detection. Request `mode="full"` for a detailed refresh. GPU-rendered windows, separate system dialogs, and elevated apps can have capture or input restrictions.

## Use the image policy in Python

Install the base package with `python -m pip install -e .` and pass the current and previous captures as Pillow images. Pass `None` as the previous capture on the first observation.

```python
from PIL import Image
from visioncache import observe

with Image.open("current.png") as current, Image.open("previous.png") as previous:
    frames = observe("adaptive_delta", current, previous, scale=0.5)
    for frame in frames:
        print(frame.image.size, frame.box, frame.reason)
```

## Measured results

Both experiments used `gpt-6.2-sol`. Costs use API-reported usage and the Standard rate card. The comparison is scoped to the tasks below.

| Task and mode | Success | USD per success | Median completion |
| --- | ---: | ---: | ---: |
| Windows stock intake and error recovery, full | 8/8 | $0.017472 | 12.41 s |
| Windows stock intake and error recovery, resized + general delta | 8/8 | $0.009810 | 11.69 s |
| Windows stock intake and error recovery, adaptive delta | 8/8 | $0.009526 | 11.85 s |
| Hwp 2020 print-preview verification, full | 3/3 | $0.009586 | 7.56 s |
| Hwp 2020 print-preview verification, adaptive delta | 3/3 | $0.006054 | 7.44 s |

The Hwp comparison used a disposable 2×2 table, waited for the window to settle after each transition, and disabled API prompt caching. Both modes used four model calls per successful trial. Adaptive mode reduced API cost by **36.8%** in that run while preserving the dialog at native resolution.

[Stock intake results](benchmarks/reference-2026-11-22.csv) · [Hwp results, including preliminary runs](benchmarks/reference-hwp-2026-11-22.csv) · [Reproduction](benchmarks/README.md)

## Development

```powershell
& .\.venv\Scripts\python.exe -m pip install -e ".[test]"
& .\.venv\Scripts\python.exe -m pytest -q
& .\.venv\Scripts\python.exe -m tests.windows_mcp_smoke
& .\.venv\Scripts\python.exe -m benchmarks.run --smoke
```

Keep the test window in the foreground during the Windows MCP smoke test. The API benchmarks require credentials. The unit tests and stock-intake smoke test run offline.

## Related work

DeltaVision and DeltaVision-OS explore changed-region observations for browser and desktop agents. VisionCache focuses on selected-window MCP control, including owned dialogs, and applies a separate rule for when a native crop is useful. The benchmarks above use VisionCache's own comparison modes. [DeltaVision](https://github.com/ddavidgao/deltavision) · [DeltaVision-OS](https://github.com/ddavidgao/deltavision-os)

## License status

This public repository currently has no license file. [GitHub's guide to repository licensing](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository)
