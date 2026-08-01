use crate::error::{LoaderError, Result};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ValueEnum, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    #[default]
    Auto,
    Off,
    Require,
}

#[derive(Debug, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct Config {
    pub pumpkin: PumpkinConfig,
    pub paths: PathsConfig,
    pub build: BuildConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct PumpkinConfig {
    pub repository: String,
    pub revision: String,
}
impl Default for PumpkinConfig {
    fn default() -> Self {
        Self {
            repository: "https://github.com/Pumpkin-MC/Pumpkin.git".into(),
            revision: "master".into(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct PathsConfig {
    pub state: PathBuf,
    pub server_data: Option<PathBuf>,
}
impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            state: ".pumpkin-loader".into(),
            server_data: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct BuildConfig {
    pub profile: String,
    pub binary: String,
    pub cargo_features: Vec<String>,
    pub no_default_features: bool,
    pub locked: bool,
    pub jobs: Option<usize>,
    pub rustflags: Option<String>,
    pub environment: BTreeMap<String, String>,
    pub sandbox: SandboxMode,
}
impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            profile: "moddev".into(),
            binary: "pumpkin".into(),
            cargo_features: vec![],
            no_default_features: false,
            locked: true,
            jobs: None,
            rustflags: None,
            environment: BTreeMap::new(),
            sandbox: SandboxMode::Auto,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct RuntimeConfig {
    pub sandbox: SandboxMode,
    pub environment: BTreeMap<String, String>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            sandbox: SandboxMode::Auto,
            environment: BTreeMap::new(),
        }
    }
}

pub fn load(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path).map_err(|source| LoaderError::ReadConfig {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| LoaderError::ParseConfig {
        path: path.to_path_buf(),
        source,
    })
}
