"""Local stdio MCP tools for Windows screen observations and input."""

import argparse
import base64
from dataclasses import asdict, dataclass
from io import BytesIO
import json

from mcp.server.fastmcp import FastMCP
from mcp.server.fastmcp.exceptions import ToolError
from mcp.types import ImageContent, TextContent, ToolAnnotations

from .observation import Mode
from .session import DesktopError, WindowSession
from .windows import WindowsDesktop


@dataclass(frozen=True)
class ScreenPoint:
    x: int
    y: int


def create_server(session: WindowSession):
    server = FastMCP(
        "VisionCache",
        log_level="WARNING",
        instructions=(
            "List windows and observe the chosen window before input. "
            "Use the supplied observation_id and image-local pixel coordinates. "
            "Image 0 is the whole client area. Later images are native-resolution crops. "
            "Observe again after each action. Request mode='full' for unreadable text "
            "or when you need a fresh native-resolution view."
        ),
    )
    read_only = ToolAnnotations(readOnlyHint=True, openWorldHint=False)

    @server.tool(annotations=read_only)
    def list_windows() -> list[dict]:
        """List visible, non-minimized Windows apps with their window handles."""
        try:
            return [asdict(window) for window in session.windows()]
        except DesktopError as error:
            raise ToolError(str(error)) from error

    @server.tool(annotations=read_only, structured_output=False)
    def observe_window(window: int, mode: Mode = "adaptive_delta") -> list[TextContent | ImageContent]:
        """Capture a window. Return JSON metadata followed by PNG images in image-index order."""
        try:
            observation = session.capture(window, mode)
        except (DesktopError, ValueError) as error:
            raise ToolError(str(error)) from error
        metadata = {
            "observation_id": observation.id,
            "window": asdict(observation.state.window),
            "window_bounds": observation.state.bounds,
            "mode": mode,
            "images": [
                {
                    "index": index, "size": frame.image.size, "source_box": frame.box,
                    "role": frame.label, "reason": frame.reason,
                }
                for index, frame in enumerate(observation.frames)
            ],
        }
        content: list[TextContent | ImageContent] = [
            TextContent(type="text", text=json.dumps(metadata, ensure_ascii=False))
        ]
        for frame in observation.frames:
            buffer = BytesIO()
            frame.image.save(buffer, format="PNG")
            content.append(ImageContent(
                type="image", mimeType="image/png",
                data=base64.b64encode(buffer.getvalue()).decode("ascii"),
            ))
        return content

    @server.tool(annotations=read_only)
    def map_point(observation_id: str, image: int, x: int, y: int) -> ScreenPoint:
        """Convert image-local pixels to Windows screen pixels for a separate input adapter."""
        try:
            x, y = session.map_point(observation_id, image, x, y)
            return ScreenPoint(x, y)
        except (DesktopError, ValueError) as error:
            raise ToolError(str(error)) from error

    def input_result(action, *args):
        try:
            action(*args)
        except (DesktopError, ValueError) as error:
            raise ToolError(str(error)) from error
        return "Input sent. Observe the window again to verify the result."

    @server.tool()
    def click(observation_id: str, image: int, x: int, y: int) -> str:
        """Left-click a visible point in the observed foreground window."""
        return input_result(session.click, observation_id, image, x, y)

    @server.tool()
    def scroll(observation_id: str, image: int, x: int, y: int, ticks: int) -> str:
        """Scroll at a visible point. Positive ticks move up, negative ticks move down."""
        return input_result(session.scroll, observation_id, image, x, y, ticks)

    @server.tool()
    def fill(observation_id: str, image: int, x: int, y: int, text: str, replace: bool = False) -> str:
        """Click a known text field and type in one action. Set replace to select its current text first."""
        return input_result(session.fill, observation_id, image, x, y, text, replace)

    @server.tool()
    def type_text(observation_id: str, text: str) -> str:
        """Type plain Unicode text into the current focus of the observed foreground window."""
        return input_result(session.type_text, observation_id, text)

    @server.tool()
    def press_key(observation_id: str, chord: str) -> str:
        """Send a key or shortcut such as enter, tab, ctrl+a, or shift+end."""
        return input_result(session.press_key, observation_id, chord)

    return server


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scale", type=float, default=0.5, help="Overview scale. Default: 0.5.")
    args = parser.parse_args()
    try:
        session = WindowSession(WindowsDesktop(), scale=args.scale)
    except (DesktopError, ValueError) as error:
        parser.error(str(error))
    create_server(session).run(transport="stdio")


if __name__ == "__main__":
    main()
