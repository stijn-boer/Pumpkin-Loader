use crate::{
    error::{LoaderError, Result},
    process,
    util::executable_name,
};
use std::{
    env,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug)]
pub struct RustToolchain {
    pub cargo: PathBuf,
    pub rustc: PathBuf,
    pub rustdoc: Option<PathBuf>,
}

pub fn resolve(cwd: &Path) -> Result<RustToolchain> {
    let active = process::output(
        Command::new("rustup")
            .args(["show", "active-toolchain"])
            .current_dir(cwd),
        "resolve active Rust toolchain",
    )?;
    let name = active
        .split_whitespace()
        .next()
        .ok_or(LoaderError::MissingActiveToolchain)?
        .to_owned();
    let sysroot = process::output(
        Command::new("rustup")
            .args(["run", &name, "rustc", "--print", "sysroot"])
            .current_dir(cwd),
        "resolve Rust sysroot",
    )?;
    let bin = PathBuf::from(sysroot.trim()).join("bin");
    let cargo = canonical_tool(&bin.join(executable_name("cargo")), "cargo")?;
    let rustc = canonical_tool(&bin.join(executable_name("rustc")), "rustc")?;
    let rustdoc_path = bin.join(executable_name("rustdoc"));
    let rustdoc = rustdoc_path
        .is_file()
        .then(|| rustdoc_path.canonicalize())
        .transpose()?;
    log::info!("Using Rust toolchain {name}");
    log::debug!("toolchain directory: {}", bin.display());
    Ok(RustToolchain {
        cargo,
        rustc,
        rustdoc,
    })
}

pub fn version(tool: &Path) -> Result<String> {
    process::output(
        Command::new(tool).arg("--version"),
        &format!("query {} version", tool.display()),
    )
}

pub fn path(cargo: &Path) -> Result<OsString> {
    let toolchain_bin = cargo
        .parent()
        .ok_or_else(|| LoaderError::CargoWithoutParent(cargo.to_path_buf()))?;
    let mut paths = vec![toolchain_bin.to_path_buf()];
    if let Some(current) = env::var_os("PATH") {
        for path in env::split_paths(&current) {
            if path.ends_with(Path::new(".cargo/bin")) {
                continue;
            }
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    env::join_paths(paths).map_err(LoaderError::from)
}

fn canonical_tool(path: &Path, name: &'static str) -> Result<PathBuf> {
    if !path.is_file() {
        return Err(LoaderError::MissingTool {
            tool: name,
            path: path.to_path_buf(),
        });
    }
    let path = path.canonicalize()?;
    if path.components().any(|part| part.as_os_str() == ".cargo") {
        return Err(LoaderError::RustupProxy { tool: name, path });
    }
    Ok(path)
}
