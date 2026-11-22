---
name: visioncache-desktop
description: Use VisionCache MCP for desktop computer use on Windows or macOS, including operating named apps, navigating dialogs, and checking their visible state. Use browser-specific tools for browser tasks and respect explicit tool selections.
---

<!-- Managed by the VisionCache Codex installer. -->

Use the registered `visioncache` MCP server for this workflow. If it is unavailable, report the missing connection and continue through another available, authorized tool only when it fits the task. Respect host app access and approvals.

1. Identify the user-requested app with `list_windows`, then call `observe_window` on its handle. Keep using that handle to follow its owned dialogs.
2. Read the metadata and PNGs together. Image 0 shows the whole window, and later images show native-resolution crops. Send `click`, `scroll`, or `fill` with the latest `observation_id` and coordinates relative to the chosen image. Windows screen pixels and macOS screen points differ, so only use `map_point` when a separate input tool needs screen coordinates.
3. Observe again after input. Request `mode="full"` if the overview is too small to read. If the tool reports stale coordinates, re-observe before deciding whether to retry. If the app cannot be focused or lacks OS permissions, report the exact blocker.
