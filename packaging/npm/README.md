# VisionCache npm package

VisionCache is a desktop MCP server that sends a scaled view of the selected window and adds native-resolution crops for small text or warnings. This package runs on a local Windows x64 desktop with Node.js 18 or later.

In a live print-preview benchmark with a desktop word processor, adaptive capture used **42.2% fewer API input tokens**, from 4,188 to 2,422 per run. Both modes completed all three trials using the Python reference implementation. [Benchmark results and method](https://github.com/HosooJeong/VisionCache#measured-api-usage)

## Install with your agent

Copy this message into your local Codex, Claude Code, or Gemini CLI session.

> Follow https://raw.githubusercontent.com/HosooJeong/VisionCache/main/INSTALL.md to install VisionCache for the agent I'm using on this computer, register its MCP server and desktop skill, and verify the connection.

The package includes [the installation guide](INSTALL.md) and [the desktop skill](skill/SKILL.md).

## Manual Codex installation

```powershell
npx visioncache --install-codex
```

Open a new local Codex chat to pick up the registered MCP server and desktop skill. The explicit skill name is `$visioncache-desktop`. Windows x64 and Node.js 18 or later are required.

## Build and test a local tarball

From the repository root on Windows with Rust's MSVC toolchain and npm:

```powershell
.\packaging\npm\pack-win-x64.ps1
npm.cmd exec --offline --yes --package .\dist\npm\visioncache-0.2.1.tgz -- visioncache --version
```

The pack command puts the archive under `dist/npm`. The launcher forwards CLI arguments and standard streams to the native executable. Use `--install-codex` instead of `--version` to register VisionCache and its desktop skill in the current user's Codex configuration.

Dependency and Rust standard library notices are included in the `licenses` directory.
