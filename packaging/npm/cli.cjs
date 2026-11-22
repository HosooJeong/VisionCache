#!/usr/bin/env node
"use strict";

const { spawn } = require("node:child_process");
const path = require("node:path");

if (process.platform !== "win32" || process.arch !== "x64") {
  console.error("This VisionCache package requires Windows x64.");
  process.exitCode = 1;
} else {
  const executable = path.join(__dirname, "native", "visioncache-mcp.exe");
  const child = spawn(executable, process.argv.slice(2), {
    stdio: "inherit",
    windowsHide: true,
  });
  const stop = () => child.kill();
  process.once("SIGINT", stop);
  process.once("SIGTERM", stop);

  child.once("error", (error) => {
    console.error(`Could not start VisionCache: ${error.message}`);
    process.exitCode = 1;
  });
  child.once("close", (code) => {
    process.removeListener("SIGINT", stop);
    process.removeListener("SIGTERM", stop);
    process.exitCode = code ?? 1;
  });
}
