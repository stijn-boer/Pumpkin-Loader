use crate::{
    config::Config,
    error::{LoaderError, Result},
    util::absolutize,
};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Layout {
    pub state: PathBuf,
    pub bare_repo: PathBuf,
    pub worktrees: PathBuf,
    pub builds: PathBuf,
    pub cargo_home: PathBuf,
    pub cargo_target: PathBuf,
    pub server_data: PathBuf,
    pub mods: PathBuf,
}

impl Layout {
    pub fn new(config_path: &Path, config: &Config) -> Result<Self> {
        let base = config_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .or_else(|_| env::current_dir())?;
        let state = absolutize(&base, &config.paths.state);
        Ok(Self {
            bare_repo: state.join("sources/pumpkin.git"),
            worktrees: state.join("worktrees"),
            builds: state.join("builds"),
            cargo_home: state.join("cargo-home"),
            cargo_target: state.join("cargo-target"),
            server_data: config
                .paths
                .server_data
                .as_ref()
                .map_or_else(|| base.clone(), |path| absolutize(&base, &path)),
            mods: base.join("mods"),
            state,
        })
    }

    pub fn create(&self) -> Result<()> {
        for path in [
            &self.state,
            &self.worktrees,
            &self.builds,
            &self.cargo_home,
            &self.cargo_target,
            &self.server_data,
            &self.mods,
        ] {
            fs::create_dir_all(path)
                .map_err(|source| LoaderError::io("create directory", path, source))?;
        }
        Ok(())
    }
}
