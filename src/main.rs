mod codex_install;

use anyhow::{Context, Result, bail};
use std::ffi::OsString;

enum Command {
    Run(f64),
    InstallCodex { scale: f64, cli: Option<OsString> },
    Permissions,
    Displayed,
}

fn parse_args() -> Result<Command> {
    let mut args = std::env::args_os().skip(1);
    let mut scale = 0.5;
    let mut install = false;
    let mut cli = None;
    while let Some(argument) = args.next() {
        match argument.to_str().context("Invalid command option.")? {
            "-h" | "--help" => {
                println!(
                    "VisionCache desktop MCP server

Usage:
  visioncache-mcp [--scale FLOAT]
  visioncache-mcp --install-codex [--codex-cli PATH] [--scale FLOAT]

Options:
  --install-codex   Register the MCP server and desktop skill in Codex
  --codex-cli PATH  Use the Codex executable at PATH
  --scale FLOAT     Set the overview scale (0.1 to less than 1.0, default 0.5)
  --permissions     Report macOS desktop permissions and exit
  --version         Print the version
  --help            Print this help"
                );
                return Ok(Command::Displayed);
            }
            "-V" | "--version" => {
                println!("visioncache-mcp {}", env!("CARGO_PKG_VERSION"));
                return Ok(Command::Displayed);
            }
            "--permissions" => return Ok(Command::Permissions),
            "--install-codex" => install = true,
            "--codex-cli" => cli = Some(args.next().context("--codex-cli expects a path.")?),
            "--scale" => {
                scale = args
                    .next()
                    .context("--scale expects a number.")?
                    .to_str()
                    .context("Invalid scale.")?
                    .parse()
                    .context("Invalid scale.")?;
            }
            other => bail!("Unknown argument {other:?}. Use --help."),
        }
    }
    if install {
        Ok(Command::InstallCodex { scale, cli })
    } else {
        anyhow::ensure!(
            cli.is_none(),
            "--codex-cli is only used with --install-codex."
        );
        Ok(Command::Run(scale))
    }
}

#[cfg(any(windows, target_os = "macos"))]
async fn serve(session: visioncache::session::Session) -> Result<()> {
    use rmcp::ServiceExt;
    let server = visioncache::mcp::McpServer::new(session);
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}

#[cfg(windows)]
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    use visioncache::{session::Session, windows::WindowsDesktop};
    match parse_args()? {
        Command::Run(scale) => serve(Session::new(Box::new(WindowsDesktop), scale)?).await,
        Command::InstallCodex { scale, cli } => codex_install::install(cli.as_deref(), scale),
        Command::Permissions => bail!("Permission diagnostics are available on macOS."),
        Command::Displayed => Ok(()),
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<()> {
    use objc2_core_foundation::{CFRunLoop, CFRunLoopRunResult, kCFRunLoopDefaultMode};
    use std::{thread, time::Duration};
    use visioncache::{
        macos::{MacDesktop, permissions},
        session::Session,
    };

    let session = match parse_args()? {
        Command::Run(scale) => Session::new(Box::new(MacDesktop::new()?), scale)?,
        Command::InstallCodex { scale, cli } => {
            return codex_install::install(cli.as_deref(), scale);
        }
        Command::Permissions => {
            MacDesktop::new()?;
            println!("{}", serde_json::to_string_pretty(&permissions())?);
            return Ok(());
        }
        Command::Displayed => return Ok(()),
    };

    // Keep the native run loop alive on the main thread while MCP calls and
    // synchronous capture waits run on Tokio threads.
    let worker = thread::Builder::new()
        .name("visioncache-mcp".into())
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()?
                .block_on(serve(session))
        })?;
    while !worker.is_finished() {
        let result = CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, 0.05, true);
        if result == CFRunLoopRunResult::Finished {
            thread::sleep(Duration::from_millis(8));
        }
    }
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("The MCP worker stopped unexpectedly."))?
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() -> Result<()> {
    match parse_args()? {
        Command::Run(scale) => {
            anyhow::ensure!(
                (0.1..1.0).contains(&scale),
                "Scale must be at least 0.1 and less than 1."
            );
            bail!("Native capture and input require an interactive Windows or macOS desktop.")
        }
        Command::InstallCodex { scale, cli } => codex_install::install(cli.as_deref(), scale),
        Command::Permissions => bail!("Permission diagnostics are available on macOS."),
        Command::Displayed => Ok(()),
    }
}
