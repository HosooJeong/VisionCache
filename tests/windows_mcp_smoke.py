"""Run from the repo root with python -m tests.windows_mcp_smoke."""

import argparse
import asyncio
import base64
from io import BytesIO
import json
import sys
import tkinter as tk

import anyio
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client
from PIL import Image

from benchmarks.fixture import Fixture, make_task


async def check(server=None):
    fixture = Fixture()
    fixture.root.title("VisionCache MCP fixture")
    fixture.root.lift()
    fixture.root.focus_force()
    task = make_task("recover-bin", 731, 1)
    fixture.reset(task)

    async def pump_ui():
        while not fixture.closed:
            fixture.update()
            await asyncio.sleep(0.015)

    if server:
        params = StdioServerParameters(command=server, args=[])
    else:
        params = StdioServerParameters(command=sys.executable, args=["-m", "visioncache.mcp_server"])
    pump = asyncio.create_task(pump_ui())
    try:
        with anyio.fail_after(35):
            async with stdio_client(params) as (reader, writer):
                async with ClientSession(reader, writer) as client:
                    await client.initialize()
                    # Starting a stdio subprocess can shift foreground focus on Windows.
                    # Bring the disposable fixture back to the foreground after startup.
                    fixture.root.lift()
                    fixture.root.focus_force()
                    fixture.update()
                    registered = {tool.name for tool in (await client.list_tools()).tools}
                    assert registered == {
                        "list_windows", "observe_window", "map_point", "click",
                        "scroll", "fill", "type_text", "press_key",
                    }

                    async def call(name, args=None):
                        result = await client.call_tool(name, args or {})
                        assert not result.isError, result.content
                        return result

                    windows = (await call("list_windows")).structuredContent["result"]
                    window = next(item["handle"] for item in windows if item["title"] == fixture.root.title())

                    async def view(mode="adaptive_delta", *, follow_owned=True):
                        result = await call("observe_window", {
                            "window": window, "mode": mode, "follow_owned": follow_owned,
                        })
                        data = json.loads(result.content[0].text)
                        images = [
                            Image.open(BytesIO(base64.b64decode(item.data)))
                            for item in result.content[1:]
                        ]
                        assert [im.size for im in images] == [tuple(frame["size"]) for frame in data["images"]]
                        assert all(image.format == "PNG" for image in images)
                        return data, images

                    async def click(context, x, y):
                        coordinates = {"observation_id": context["observation_id"], "image": 0,
                                       "x": x, "y": y}
                        point = (await call("map_point", coordinates)).structuredContent
                        left, top, _, _ = context["window_bounds"]
                        assert (point["x"], point["y"]) == (left + x * 2, top + y * 2)
                        await call("click", coordinates)
                        replay = await client.call_tool("click", coordinates)
                        assert replay.isError and "stale" in replay.content[0].text
                        await asyncio.sleep(0.08)

                    async def enter_text(x, y, text, replace=False):
                        context, _ = await view()
                        await click(context, x, y)
                        if replace:
                            context, _ = await view()
                            await call("press_key", {"observation_id": context["observation_id"], "chord": "ctrl+a"})
                            await asyncio.sleep(0.08)
                        context, _ = await view()
                        await call("type_text", {"observation_id": context["observation_id"], "text": text})
                        await asyncio.sleep(0.08)

                    full, images = await view("full")
                    assert images[0].size == (1024, 640)
                    assert images[0].getpixel((4, 4)) == (240, 244, 248)
                    assert full["images"][0]["source_box"] == [0, 0, 1024, 640]

                    await enter_text(80, 134, task.sku)
                    await enter_text(80, 175, task.quantity)
                    await enter_text(80, 216, task.preferred_bin)
                    context, _ = await view()
                    await click(context, 77, 257)
                    context, _ = await view()
                    assert context["images"][1]["reason"] == "red_warning_text"

                    await call("fill", {"observation_id": context["observation_id"], "image": 0,
                                        "x": 80, "y": 216, "text": task.active_bin, "replace": True})
                    await asyncio.sleep(0.08)
                    assert fixture.controls["bin"].get() == task.active_bin
                    context, _ = await view()
                    await click(context, 77, 257)
                    context, _ = await view()
                    assert context["images"][1]["reason"] == "small_text"

                    await enter_text(372, 216, task.receipt)
                    context, _ = await view()
                    await click(context, 351, 257)
                    assert fixture.succeeded(), fixture.values()

                    await enter_text(80, 134, "한글 입력", replace=True)
                    assert fixture.controls["sku"].get() == "한글 입력"

                    context, _ = await view()
                    await call("scroll", {"observation_id": context["observation_id"],
                                          "image": 0, "x": 80, "y": 175, "ticks": -1})
                    context, _ = await view()
                    modal = tk.Toplevel(fixture.root)
                    modal.title("VisionCache owned dialog")
                    modal.transient(fixture.root)
                    modal.geometry("360x240+150+160")
                    tk.Label(modal, text="Local owned-dialog fixture").pack()
                    modal.update()
                    modal.focus_force()
                    await asyncio.sleep(0.08)
                    owned, images = await view()
                    assert owned["requested_window"] == window
                    assert owned["window"]["title"] == "VisionCache owned dialog"
                    assert owned["effective_mode"] == "full" and images[0].size == (360, 240)
                    exact, _ = await view(follow_owned=False)
                    assert exact["window"]["handle"] == window
                    modal.destroy()
                    fixture.root.focus_force()
                    context, _ = await view()
                    assert context["window"]["handle"] == window
                    fixture.root.geometry("+100+120")
                    fixture.update()
                    invalid = await client.call_tool("click", {"observation_id": context["observation_id"],
                                                                "image": 0, "x": 80, "y": 175})
                    assert invalid.isError and "placement changed" in invalid.content[0].text
                    print("Windows MCP passed: capture, two crop triggers, input, Unicode, stale window.")
    finally:
        pump.cancel()
        fixture.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", help="Path to the standalone MCP executable.")
    asyncio.run(check(parser.parse_args().server))
