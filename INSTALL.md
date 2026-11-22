# VisionCache installation guide

If the user asks you to install VisionCache, run the applicable steps on their local desktop account. Identify the client from the current session or the user's request. Install for that client only unless the user names multiple clients. Keep existing configuration and custom skills. Stop and report any conflicting `visioncache` entry or permission prompt that requires the user's action.

## 1. Get the native executable

### Windows x64

Use Node.js 18 or later. In PowerShell, run:

```powershell
node -p "process.platform + ' ' + process.arch"
npm.cmd install --global visioncache --registry=https://registry.npmjs.org/
$VisionCachePackage = Join-Path (npm.cmd root --global) 'visioncache'
$VisionCacheBinary = Join-Path $VisionCachePackage 'native/visioncache-mcp.exe'
& $VisionCacheBinary --version
```

Expect `win32 x64` and a version beginning with `visioncache-mcp`. Check each result before continuing. Use the absolute native executable path as the stdio command. It works from any project directory and avoids a shell wrapper. The public package can be installed without logging into npm.

### macOS 14 or later

Build on the local Mac with Rust and Xcode Command Line Tools. If there is no VisionCache checkout, clone the official repository first:

```sh
git clone https://github.com/HosooJeong/VisionCache.git
cd VisionCache
```

Run these commands from the checkout:

```sh
MACOSX_DEPLOYMENT_TARGET=14.0 cargo build --locked --release
./target/release/visioncache-mcp --version
./target/release/visioncache-mcp --permissions
```

Use the absolute path to `target/release/visioncache-mcp` in the registration steps. Guide the user through any required Screen Recording or Accessibility prompts. [Build and permissions guide](https://github.com/HosooJeong/VisionCache/blob/main/docs/macos-design.md)

VisionCache needs an interactive Windows or macOS desktop session. If your shell runs in a cloud sandbox, WSL, or on a remote machine, use the user's local session for installation and verification.

## 2. Register with the current client

### Codex

If both the MCP server and skill already work in Codex, reuse them. Otherwise, run the installer from the executable you just verified. On Windows PowerShell:

```powershell
& $VisionCacheBinary --install-codex
codex mcp get visioncache --json
```

On macOS, invoke the built binary with the same flag. The installer registers the stdio server and writes the user skill at `~/.agents/skills/visioncache-desktop/SKILL.md`. Confirm the entry is enabled and its executable exists. Use a new local Codex chat to load the server and skill. [Codex MCP](https://developers.openai.com/codex/mcp) · [Codex skills](https://developers.openai.com/codex/skills)

### Claude Code

Check `claude mcp get visioncache`. If the alias is free, register the executable at user scope. In PowerShell:

```powershell
claude mcp add --transport stdio --scope user visioncache -- "$VisionCacheBinary"
claude mcp get visioncache
```

If the existing entry already launches VisionCache at the right scope, reuse it. Copy the desktop skill to `~/.claude/skills/visioncache-desktop/SKILL.md`, or the matching `skills` directory beneath `CLAUDE_CONFIG_DIR` when set. Confirm it appears as `/visioncache-desktop` in a local session. [Claude Code MCP](https://code.claude.com/docs/en/mcp) · [Claude Code skills](https://code.claude.com/docs/en/skills)

### Gemini CLI

Use a Gemini CLI release with Agent Skills support. Check whether `gemini skills list` or the interactive `/skills list` is available before claiming skill discovery.

Check `gemini mcp list`. If the alias is free, register the executable at user scope. In PowerShell:

```powershell
gemini mcp add visioncache "$VisionCacheBinary" --scope user
gemini mcp list
```

If the existing entry already launches VisionCache at the right scope, reuse it. Copy the desktop skill to `~/.agents/skills/visioncache-desktop/SKILL.md` and check discovery with `gemini skills list` or `/skills list` in the client. If Codex installed the same skill there, reuse it. [Gemini CLI MCP and skills commands](https://geminicli.com/docs/cli/cli-reference/) · [Gemini CLI skill locations](https://geminicli.com/docs/cli/using-agent-skills/)

If Gemini lists the server as disabled in an untrusted folder, verify the connection from a workspace the user already trusts. [Gemini CLI workspace trust](https://geminicli.com/docs/cli/trusted-folders/)

### Copying the desktop skill

For clients where you need to copy the skill, prefer `skill/SKILL.md` from the installed package if present. Otherwise use `integrations/codex/visioncache-desktop/SKILL.md` from the VisionCache checkout or the [official repository](https://github.com/HosooJeong/VisionCache/blob/main/integrations/codex/visioncache-desktop/SKILL.md). Copy the file unchanged, including its front matter. If the destination already has the same file, reuse it. Ask before replacing a different file.

For another MCP client, use its documented local stdio registration and skill discovery paths if available.

## 3. Verify the connection

1. Confirm the client shows the `visioncache` server and discovers `visioncache-desktop`.
2. From the client, list the server's tools and confirm `list_windows` and `observe_window` are present. When the existing session cannot reload tools, use the client's refresh command or ask the user to open a new local session. Report configuration and live tool discovery separately.
3. Tell the user which client is ready. For a first task, they can ask the agent to operate a named desktop app. Use the agent's normal tool approvals.
