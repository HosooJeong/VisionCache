use std::{
    env,
    ffi::OsStr,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use sha2::{Digest, Sha256};

const NAME: &str = "visioncache";
const SKILL_FOLDER: &str = "visioncache-desktop";
const SKILL_MARKER: &str = "<!-- Managed by the VisionCache Codex installer. -->";
const SKILL: &str = include_str!("../integrations/codex/visioncache-desktop/SKILL.md");

pub fn install(codex: Option<&OsStr>, scale: f64) -> Result<()> {
    ensure!(
        (0.1..1.0).contains(&scale),
        "Scale must be at least 0.1 and less than 1."
    );
    ensure!(
        cfg!(any(windows, target_os = "macos")),
        "VisionCache desktop setup requires Windows or macOS."
    );
    let user_home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .context("Could not resolve the user home directory.")?;
    let codex_home = env::var_os("CODEX_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home.join(".codex"));
    let runtime = absolute(codex_home.join("visioncache").join("runtime"))?;
    let skill = user_home
        .join(".agents")
        .join("skills")
        .join(SKILL_FOLDER)
        .join("SKILL.md");
    let codex =
        codex.unwrap_or_else(|| OsStr::new(if cfg!(windows) { "codex.exe" } else { "codex" }));
    let original = env::current_exe().context("Could not locate this VisionCache executable.")?;
    let binary = fs::read(&original).context("Could not read this VisionCache executable.")?;
    let id = Sha256::digest(&binary)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let command_path = runtime
        .join(id)
        .join(format!("visioncache-mcp{}", env::consts::EXE_SUFFIX));
    let args = ["--scale".to_string(), scale.to_string()];

    let servers = list_servers(codex)?;
    let existing = servers.iter().find(|server| server["name"] == NAME);
    if let Some(existing) = existing {
        // A user may have reserved this alias for an unrelated service.
        let current = server_command(existing)
            .context("The Codex name visioncache is already used by a non-stdio server.")?;
        ensure!(
            current.starts_with(&runtime)
                || current.file_name() == original.file_name()
                || current.file_name() == command_path.file_name(),
            "The Codex name visioncache already points to another command."
        );
    }

    let skill_update = check_skill(&skill)?;
    match read_optional(&command_path)? {
        Some(existing_binary) => ensure!(
            existing_binary == binary,
            "The installed VisionCache binary is corrupted."
        ),
        None => write_file(&command_path, &binary, true)?,
    }

    let matches = existing.is_some_and(|server| registration_matches(server, &command_path, &args));
    if !matches {
        let mut add = Command::new(codex);
        add.args(["mcp", "add", NAME, "--"])
            .arg(&command_path)
            .args(&args);
        run_codex(add, "register the VisionCache stdio server")?;
    }
    let current = list_servers(codex)?;
    ensure!(
        current.iter().any(|entry| entry["name"] == NAME && registration_matches(entry, &command_path, &args)),
        "VisionCache was not enabled in the effective Codex MCP configuration."
    );

    if skill_update {
        write_file(&skill, SKILL.as_bytes(), false)?;
    }
    println!("VisionCache MCP: {}", command_path.display());
    println!("Desktop routing skill: {}", skill.display());
    println!("Open a new local Codex chat to use the registered server and skill.");
    Ok(())
}

fn absolute(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()?.join(path))
    }
}

fn run_codex(mut command: Command, operation: &str) -> Result<Vec<u8>> {
    let result = command.output().with_context(|| {
        format!("Could not {operation}. Place the Codex CLI on PATH or set --codex-cli.")
    })?;
    // List/status output can include credentials for other servers. Only the caller
    // examines the fields for VisionCache, and other server settings are not printed.
    ensure!(
        result.status.success(),
        "Codex could not {operation} ({}).",
        result.status
    );
    Ok(result.stdout)
}

fn list_servers(codex: &OsStr) -> Result<Vec<Value>> {
    let mut list = Command::new(codex);
    list.args(["mcp", "list", "--json"]);
    let body = run_codex(list, "read MCP configuration")?;
    serde_json::from_slice(&body).context("Codex returned an unexpected MCP server list.")
}

fn server_command(server: &Value) -> Option<&Path> {
    (server["transport"]["type"] == "stdio")
        .then(|| server["transport"]["command"].as_str().map(Path::new))
        .flatten()
}

fn registration_matches(server: &Value, command: &Path, args: &[String; 2]) -> bool {
    server["enabled"] == true
        && server_command(server) == Some(command)
        && server["transport"]["args"]
            .as_array()
            .is_some_and(|actual| {
                actual.len() == args.len()
                    && actual
                        .iter()
                        .zip(args)
                        .all(|(a, b)| a.as_str() == Some(b.as_str()))
            })
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("Could not read {}", path.display())),
    }
}

fn check_skill(path: &Path) -> Result<bool> {
    match read_optional(path)? {
        None => Ok(true),
        Some(bytes) if bytes == SKILL.as_bytes() => Ok(false),
        Some(bytes) if String::from_utf8_lossy(&bytes).contains(SKILL_MARKER) => Ok(true),
        Some(_) => bail!(
            "The Codex skill at {} is not managed by VisionCache.",
            path.display()
        ),
    }
}

fn write_file(path: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let directory = path.parent().context("Installation path has no parent.")?;
    fs::create_dir_all(directory)?;
    let mut tmp = tempfile::NamedTempFile::new_in(directory)?;
    tmp.write_all(bytes)?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))?;
    }

    // A versioned runtime never replaces an open executable. The managed skill
    // can be updated atomically and Codex can discover it on a later launch.
    let published = if executable {
        tmp.persist_noclobber(path)
    } else {
        tmp.persist(path)
    };
    published.with_context(|| format!("Could not install {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_an_enabled_exact_registration_is_idempotent() {
        let path = Path::new("test/visioncache-mcp");
        let args = ["--scale".to_owned(), "0.5".to_owned()];
        let mut registry = json!({
            "name": NAME,
            "enabled": true,
            "transport": { "type": "stdio", "command": path, "args": &args }
        });
        assert!(registration_matches(&registry, path, &args));
        registry["enabled"] = false.into();
        assert!(!registration_matches(&registry, path, &args));
        registry["enabled"] = true.into();
        registry["transport"]["args"] = json!(["--scale", "0.75"]);
        assert!(!registration_matches(&registry, path, &args));
    }

    #[test]
    fn installer_accepts_its_own_skill_and_protects_user_content() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("visioncache-desktop/SKILL.md");

        assert!(check_skill(&path)?);
        write_file(&path, SKILL.as_bytes(), false)?;
        assert!(!check_skill(&path)?);

        fs::write(&path, "A user-authored skill")?;
        assert!(check_skill(&path).is_err());
        assert_eq!(fs::read_to_string(path)?, "A user-authored skill");
        Ok(())
    }
}
