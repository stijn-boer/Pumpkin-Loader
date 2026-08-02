use std::{
    collections::BTreeMap,
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

use crate::{
    config::SandboxMode,
    error::{LoaderError, Result},
    layout::Layout,
    util::find_in_path,
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
    if !cfg!(target_os = "linux") {
        if matches!(mode, SandboxMode::Require) {
            return Err(LoaderError::SandboxUnavailable);
        }

        // Bubblewrap is Linux-specific. On Windows and other platforms, `auto`
        // intentionally falls back to an ordinary child process without probing
        // PATH for bwrap or printing an alarming warning.
        return Ok(direct_command(program, args, cwd, envs));
    }

    let bwrap = find_in_path("bwrap");
    let use_bwrap = match (mode, bwrap.is_some()) {
        (SandboxMode::Off, _) => false,
        (SandboxMode::Auto, true) | (SandboxMode::Require, true) => true,
        (SandboxMode::Auto, false) => {
            log::warn!(
                "bubblewrap is unavailable; using process/directory isolation only"
            );
            false
        }
        (SandboxMode::Require, false) => {
            return Err(LoaderError::SandboxUnavailable);
        }
    };

    if !use_bwrap {
        return Ok(direct_command(program, args, cwd, envs));
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
                .arg(&layout.cargo_target);

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

fn direct_command(
    program: &Path,
    args: &[String],
    cwd: &Path,
    envs: &BTreeMap<OsString, OsString>,
) -> Command {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(envs);
    command
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

    // Keep the environment deliberately small, but preserve every platform
    // variable required to spawn programs and let Cargo/rustup locate their
    // installations and temporary directories.
    for key in [
        "PATH",
        "HOME",
        "USER",
        "LOGNAME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "SYSTEMROOT",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LOCALAPPDATA",
        "APPDATA",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
        "PROGRAMW6432",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "CARGO_HOME",
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
        if let Some(home) = user_home() {
            result.insert(
                "RUSTUP_HOME".into(),
                home.join(".rustup").into_os_string(),
            );
        }
    }

    if !result.contains_key(OsStr::new("CARGO_HOME")) {
        if let Some(home) = user_home() {
            result.insert(
                "CARGO_HOME".into(),
                home.join(".cargo").into_os_string(),
            );
        }
    }

    result
}

fn user_home() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
