# VisionCache

VisionCache is a Rust MCP server for Windows and macOS desktop agents. It sends a scaled view of the selected window and adds native-resolution crops when changes include small text or red warnings. The same server can click, scroll, type, and follow the window's foreground dialogs.

**42.2% fewer API input tokens in a desktop word-processing benchmark.** Adaptive capture used 2,422 input tokens per completed run, down from 4,188 with full-resolution capture. Both modes completed all three runs. [Measured results](#measured-api-usage)

[Installation guide](INSTALL.md) · [Benchmarks](benchmarks/README.md) · [macOS design](docs/macos-design.md) · [한국어](README.ko.md)

## Install with your agent

Open a local Codex, Claude Code, or Gemini CLI session on the desktop you want to control. Copy this message into the chat.

> Follow https://raw.githubusercontent.com/HosooJeong/VisionCache/main/INSTALL.md to install VisionCache for the agent I'm using on this computer, register its MCP server and desktop skill, and verify the connection.

The agent can handle the terminal steps. The npm release is for Windows x64. On macOS, use the source build. [Installation guide](INSTALL.md)

## Measured API usage

In a Windows desktop word processor, the agent switched a document to landscape, opened print preview, and confirmed the page count and table borders. This live OpenAI Responses API test used `gpt-6.2-sol` and the Python reference implementation of VisionCache. Each trial took four API calls, with zero cached input or cache-write tokens in either mode.

| Capture mode | Success | Input tokens per run | Total tokens per run | API cost per success | Median completion |
| --- | ---: | ---: | ---: | ---: | ---: |
| Full resolution | 3/3 | 4,188 | 4,309 | $0.009586 | 7.56 s |
| Adaptive | 3/3 | 2,422 | 2,543 | $0.006054 | 7.44 s |

Adaptive capture reduced **input tokens by 42.2%**, **total tokens by 41.0%**, and **usage-based API cost by 36.8%**. The token counts come from the API's reported usage. Total tokens include input and output. Costs apply the benchmark rates to that reported usage. [API usage CSV](benchmarks/reference-hwp-2026-11-22.csv) · [Test method](benchmarks/README.md#word-processor-print-preview)

In a separate Windows stock-intake fixture, all four modes succeeded in 8/8 trials each. Adaptive capture reduced usage-based cost per success by **45.5%** from full resolution, with median completion moving from 12.41 s to 11.85 s. [Four-mode results CSV](benchmarks/reference-2026-11-22.csv)

## Platform support

| Component | Status |
| --- | --- |
| Rust runtime | Native Windows capture, adaptive observation, PNG encoding, and input over stdio MCP. |
| Portable Rust core | BGR difference detection, crop selection, PNG encoding, and observation state. |
| macOS 14+ | Native Rust ScreenCaptureKit, Accessibility, and Quartz backend. Cross-target checked on Apple Silicon and Intel. On-device verification is pending. |
| Python reference | Earlier implementation and reproducible benchmark harness. |

## Windows quick start

Build on Windows with the Rust MSVC toolchain. Use an interactive Windows desktop for window capture and input.

```powershell
git clone https://github.com/HosooJeong/VisionCache.git
cd VisionCache
cargo build --locked --release
.\target\release\visioncache-mcp.exe --help
```

Use the executable's absolute path to register a local **stdio** MCP server.

```json
{
  "mcpServers": {
    "visioncache": {
      "command": "C:\\path\\to\\VisionCache\\target\\release\\visioncache-mcp.exe",
      "args": ["--scale", "0.5"]
    }
  }
}
```

The Windows executable needs an interactive session. VisionCache uses no Python runtime or model API credential. The connected client decides where the returned PNGs and metadata go.

## macOS build

On macOS 14 or later with Rust and Xcode Command Line Tools installed:

```sh
MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --locked --release
./target/release/visioncache-mcp --permissions
```

Use the absolute path to the binary in an MCP stdio configuration. Enable Screen Recording and Accessibility for the host as applicable. Apple Silicon and Intel cross-target checks pass. The actual permissions, image geometry, and input need testing on a Mac. [macOS setup and smoke test](docs/macos-design.md)

## Install for Codex

From the built executable, one command installs the local MCP server and an implicitly discoverable desktop skill in the current user's Codex setup:

```powershell
.\target\release\visioncache-mcp.exe --install-codex
```

On macOS, use `./target/release/visioncache-mcp --install-codex`. If Codex is outside PATH, pass its executable with `--codex-cli`. For a different overview size, pass `--scale` to the installer.

Open a new **local** Codex chat. On a request to operate a Windows or macOS desktop application, Codex can select the VisionCache skill and use the registered MCP tools. You can explicitly invoke `$visioncache-desktop` to choose this route. Installation uses Codex's external MCP and skill interfaces. The app's built-in **Settings → Computer use** switch controls the bundled provider separately. [MCP setup](https://learn.chatgpt.com/docs/mcp) · [Skill invocation](https://learn.chatgpt.com/docs/build-skills) · [Built-in Computer Use](https://learn.chatgpt.com/docs/computer-use)

## Use the MCP tools

1. Call `list_windows`, then pass a returned handle to `observe_window`.
2. The response contains JSON metadata followed by PNGs. Image `0` shows the whole client area. Any following image is a native-resolution crop. Use `images[].source_box` for its bounds in capture pixels.
3. Use the newest `observation_id` and image-local coordinates for `click`, `scroll`, or `fill`. Use `type_text` or `press_key` for the current keyboard focus. `map_point` converts the image-local coordinates to the coordinate space in the observation. Windows uses screen pixels and macOS uses screen points.
4. Observe again after each input. The target must be in the foreground, the point must be visible, and the window placement must match the observation.

Keep requesting the same application handle to follow its owned foreground dialog and return after it closes. `requested_window` is the original handle, and `window` describes the actual captured surface. Set `follow_owned=false` to capture exactly the requested window.

| Mode | Images returned |
| --- | --- |
| `full` | The window's native-resolution client capture |
| `resized` | A scaled overview |
| `resized_delta` | A scaled overview and a bounded crop of changed pixels |
| `adaptive_delta` | A scaled overview and a bounded native crop for newly changed small text or red warnings. Owned foreground dialogs are returned at native resolution. |

Crop eligibility depends on pixel height and color. Use `mode="full"` for on-demand detail. Some hardware-accelerated, elevated, and system windows have capture or input restrictions. On macOS, input also requires an accessible foreground window and a reliable hit target.

## Native data path

The Windows adapter captures the selected client area through GDI and writes top-down BGR scanlines into a Rust buffer. The session swaps two reusable buffers between observations and compares the capture bytes directly. The macOS adapter converts a ScreenCaptureKit image into the same BGR format. The shared output stage resizes the overview and converts colors as it encodes PNG. It retains the captured surface and its placement separately from the image pixels.

## Validation

```powershell
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The native app smoke test creates a disposable Windows GUI and uses a separate MCP client to drive the Rust executable. Python is used by this test harness.

```powershell
python -m venv .venv
& .\.venv\Scripts\python.exe -m pip install -e ".[test]"
& .\.venv\Scripts\python.exe -m tests.windows_mcp_smoke --server .\target\release\visioncache-mcp.exe
```

## Related work

DeltaVision and DeltaVision-OS explore changed-region observations for browsers and desktop agents. VisionCache combines selected-window MCP control, owned-dialog tracking, and a separate rule for when to send a native crop. [DeltaVision](https://github.com/ddavidgao/deltavision) · [DeltaVision-OS](https://github.com/ddavidgao/deltavision-os)

## License status

This public repository currently has no license file. [GitHub's guide to repository licensing](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/licensing-a-repository)
