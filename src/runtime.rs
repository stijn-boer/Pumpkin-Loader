use crate::{
    config::Config,
    error::{LoaderError, Result},
    layout::Layout,
    sandbox::{self, Purpose},
};
use std::{ffi::OsString, path::Path};

pub fn run(config: &Config, layout: &Layout, artifact: &Path, args: &[OsString]) -> Result<()> {
    let mut envs = sandbox::minimal_environment();
    for (key, value) in &config.runtime.environment {
        envs.insert(key.into(), value.into());
    }
    let program = artifact
        .canonicalize()
        .map_err(|source| LoaderError::Canonicalize {
            path: artifact.to_path_buf(),
            source,
        })?;
    let args: Vec<String> = args
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    log::info!("Starting Pumpkin in {}", layout.server_data.display());
    let command = sandbox::command(
        &program,
        &args,
        &layout.server_data,
        &envs,
        config.runtime.sandbox,
        Purpose::Runtime { binary: &program },
        layout,
    )?;
    let status = sandbox::run_inherited(command, "Pumpkin server")?;
    if !status.success() {
        return Err(LoaderError::ServerExited { status });
    }
    Ok(())
}
