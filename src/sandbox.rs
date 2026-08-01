use crate::{
    config::SandboxMode,
    error::{LoaderError, Result},
    layout::Layout,
    util::find_in_path,
};
use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

#[derive(Debug)]
pub enum Purpose<'a> {
    Build,
    Runtime { binary: &'a Path },
}

pub fn command(
    program: &Path,
    args: &[String],
    cwd: &Path,
    envs: &BTreeMap<OsString, OsString>,
    mode: SandboxMode,
    purpose: Purpose<'_>,
    layout: &Layout,
) -> Result<Command> {
    let bwrap = find_in_path("bwrap");
    let use_bwrap = match (mode, bwrap.is_some()) {
        (SandboxMode::Off, _) => false,
        (SandboxMode::Auto, true) | (SandboxMode::Require, true) => cfg!(target_os = "linux"),
        (SandboxMode::Auto, false) => {
            log::warn!("bubblewrap is unavailable; using process/directory isolation only");
            false
        }
        (SandboxMode::Require, false) => return Err(LoaderError::SandboxUnavailable),
    };

    if !use_bwrap {
        let mut command = Command::new(program);
        command.args(args).current_dir(cwd).env_clear().envs(envs);
        return Ok(command);
    }

    log::debug!("starting {:?} sandbox with bubblewrap", purpose);
    let mut command = Command::new("bwrap");
    command
        .args([
            "--die-with-parent",
            "--new-session",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
        ])
        .args(["--ro-bind", "/usr", "/usr"])
        .args(["--ro-bind-try", "/bin", "/bin"])
        .args(["--ro-bind-try", "/lib", "/lib"])
        .args(["--ro-bind-try", "/lib64", "/lib64"])
        .args(["--ro-bind", "/etc", "/etc"])
        .args(["--tmpfs", "/tmp"]);

    match purpose {
        Purpose::Build => {
            command
                .arg("--bind")
                .arg(cwd)
                .arg(cwd)
                .arg("--bind")
                .arg(&layout.cargo_home)
                .arg(&layout.cargo_home)
                .arg("--bind")
                .arg(&layout.cargo_target)
                .arg(&layout.cargo_target)
                .arg("--bind")
                .arg(&layout.plugin_target)
                .arg(&layout.plugin_target);
            if let Some(home) = env::var_os("HOME") {
                let rustup = PathBuf::from(home).join(".rustup");
                if rustup.exists() {
                    command.arg("--ro-bind").arg(&rustup).arg(&rustup);
                }
            }
        }
        Purpose::Runtime { binary } => {
            command
                .args(["--unshare-all", "--share-net"])
                .arg("--ro-bind")
                .arg(binary)
                .arg(binary)
                .arg("--bind")
                .arg(cwd)
                .arg(cwd);
        }
    }
    command
        .arg("--chdir")
        .arg(cwd)
        .arg("--")
        .arg(program)
        .args(args)
        .env_clear()
        .envs(envs);
    Ok(command)
}

pub fn run_inherited(mut command: Command, description: &str) -> Result<ExitStatus> {
    let rendered = crate::process::render(&command);
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|source| LoaderError::CommandSpawn {
            description: description.to_owned(),
            command: rendered,
            source,
        })
}

pub fn minimal_environment() -> BTreeMap<OsString, OsString> {
    let mut result = BTreeMap::new();
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "RUSTUP_HOME",
        "RUSTUP_TOOLCHAIN",
        "TERM",
        "COLORTERM",
        "LANG",
        "LC_ALL",
        "TZ",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "NO_PROXY",
        "http_proxy",
        "https_proxy",
        "no_proxy",
    ] {
        if let Some(value) = env::var_os(key) {
            result.insert(key.into(), value);
        }
    }
    if !result.contains_key(OsStr::new("RUSTUP_HOME")) {
        if let Some(home) = env::var_os("HOME") {
            result.insert(
                "RUSTUP_HOME".into(),
                PathBuf::from(home).join(".rustup").into_os_string(),
            );
        }
    }
    result
}
