# macOS desktop adapter

The Rust adapter targets macOS 14 Sonoma or later. Its sources pass cross-target Rust checks for Apple Silicon and Intel. The native build, permissions, display geometry, and desktop interaction still need verification in a signed-in macOS session.

## Build and connect

On macOS with the Rust toolchain and Xcode Command Line Tools, build the same executable used by the Windows entry point.

```sh
MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --locked --release
./target/release/visioncache-mcp --permissions
```

Register the binary as a local stdio MCP server.

```json
{
  "mcpServers": {
    "visioncache": {
      "command": "/absolute/path/to/VisionCache/target/release/visioncache-mcp",
      "args": ["--scale", "0.5"]
    }
  }
}
```

`--permissions` reports `screen_recording`, `accessibility`, and `event_posting`. Grant Screen Recording to the host process recognized by macOS. Grant Accessibility for input. On recent macOS versions the screen permission may appear as Screen & System Audio Recording. Check whether the OS names Terminal or your MCP client when configuring permission and restart that host if requested. Capture can run with screen permission alone. [Screen capture](https://developer.apple.com/documentation/screencapturekit) · [Accessibility trust](https://developer.apple.com/documentation/applicationservices/1460720-axisprocesstrusted) · [Quartz permission APIs](https://developer.apple.com/documentation/coregraphics/core-graphics-functions)

## Capture and coordinates

The adapter enumerates on-screen windows with ScreenCaptureKit and scopes each screenshot to the selected window ID and owning PID. It uses the native one-frame screenshot API with cursor and shadows disabled. It normalizes the image to BGR24 for the shared Rust selection and PNG pipeline. The native Core Foundation run loop stays on the main thread while MCP runs on worker threads. [ScreenCaptureKit window capture](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos) · [Single-frame capture](https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager)

Capture dimensions come from the window filter's content rectangle and pixel scale. The observation reports `coordinate_space="screen_points"` and global point bounds on macOS. Original raster pixels remain the unit of `images[].source_box`. Retaining the two coordinate systems lets the server map image-local coordinates to fractional or negative screen positions on Retina and secondary displays. For instance, raster point `(400, 200)` in a 1600×1200 raster corresponding to an 800×600-point window at `(-800, 50)` maps to `(-600, 150)`. [Pixel scale](https://developer.apple.com/documentation/screencapturekit/sccontentfilter/pointpixelscale) · [Accessibility screen coordinates](https://developer.apple.com/documentation/applicationservices/kaxpositionattribute)

## Dialogs and input

The adapter uses the foreground Accessibility window to identify the active surface. It follows a separate foreground dialog when it has an Accessibility parent link to the requested window and a unique ScreenCaptureKit match. An attached sheet can share the parent's captured surface. Adaptive mode sends that surface at native resolution while the sheet is detected. Keep passing the original handle and request a new observation after each input.

Before posting Quartz events, the server revalidates the focused window and placement. Point input also uses a system-wide Accessibility hit test. When the visible element cannot be associated with the observed window or active sheet, the tool returns an error. `fill(replace=true)` verifies keyboard focus in an accessible text field, then uses Cmd+A and Unicode input. Key chords accept `cmd`, `ctrl`, `option` or `alt`, and `shift`. Letter and digit shortcuts use ANSI key positions. `type_text` sends literal Unicode. [AX hit testing](https://developer.apple.com/documentation/applicationservices/1462077-axuielementcopyelementatposition) · [Quartz events](https://developer.apple.com/documentation/coregraphics/core-graphics-functions)

## First native smoke test

1. In TextEdit, create a disposable document containing synthetic text and keep it in the foreground. Check `--permissions` from the same execution context as the MCP server.
2. Use `list_windows` and `observe_window` on the document. Confirm the PNG dimensions, color orientation, and `screen_points` geometry. Use `map_point` and `click` on a known visible position, then observe again and type.
3. Open the print sheet with `cmd+p`. Re-observe the original handle and verify the active surface is in the result. Dismiss the sheet with `escape`; re-observe the document and verify stale input is rejected.
4. Repeat with an external display at a different scale and a negative display origin. Move or cover the window, switch apps, and test with Accessibility disabled while Screen Recording remains enabled.

These are on-device acceptance checks. The Rust unit tests already exercise the shared Mac point transform with a synthetic Retina raster and fractional, negative display coordinates. Native GUI behavior and the Windows API benchmark are separate results.
