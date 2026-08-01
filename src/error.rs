use std::{io, path::PathBuf, process::ExitStatus};

use thiserror::Error;

pub type Result<T> = std::result::Result<T, LoaderError>;

#[derive(Debug, Error)]
pub enum LoaderError {
    #[error("configuration file already exists: {path}; pass --force to replace it")]
    ConfigAlreadyExists { path: PathBuf },

    #[error("failed to read configuration file {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("invalid TOML in {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to serialize configuration: {0}")]
    SerializeConfig(#[from] toml::ser::Error),

    #[error("failed to edit Cargo manifest {path}: {source}")]
    EditCargoManifest {
        path: PathBuf,
        #[source]
        source: toml_edit::TomlError,
    },

    #[error("filesystem operation `{operation}` failed for {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("I/O error: {0}")]
    UnclassifiedIo(#[from] io::Error),

    #[error("failed to start `{description}`\n  command: {command}\n  cause: {source}")]
    CommandSpawn {
        description: String,
        command: String,
        #[source]
        source: io::Error,
    },

    #[error("{description} failed\n  command: {command}\n  status: {status}\n{diagnostic}")]
    CommandFailed {
        description: String,
        command: String,
        status: ExitStatus,
        diagnostic: String,
    },

    #[error("{description} returned non-UTF-8 output: {source}")]
    CommandOutputUtf8 {
        description: String,
        #[source]
        source: std::string::FromUtf8Error,
    },

    #[error("sandbox=require but `bwrap` was not found in PATH")]
    SandboxUnavailable,

    #[error("source cache is empty")]
    SourceCacheEmpty,

    #[error("configured revision `{revision}` is not present in the source cache")]
    RevisionUnavailable { revision: String },

    #[error("rustup returned no active toolchain")]
    MissingActiveToolchain,

    #[error("active toolchain does not contain {tool}: {path}")]
    MissingTool { tool: &'static str, path: PathBuf },

    #[error("resolved {tool} is unexpectedly a rustup proxy: {path}")]
    RustupProxy { tool: &'static str, path: PathBuf },

    #[error("Cargo executable has no parent directory: {0}")]
    CargoWithoutParent(PathBuf),

    #[error("failed to construct sandbox PATH: {0}")]
    InvalidPath(#[from] std::env::JoinPathsError),

    #[error("Cargo completed successfully but expected binary was not produced: {path}")]
    MissingBuildArtifact { path: PathBuf },

    #[error("build artifact path has no parent: {0}")]
    ArtifactWithoutParent(PathBuf),

    #[error("cached manifest build key does not match the requested build")]
    CacheKeyMismatch,

    #[error(
        "cached Pumpkin binary failed integrity verification\n  expected: {expected}\n  actual:   {actual}"
    )]
    IntegrityMismatch { expected: String, actual: String },

    #[error("Pumpkin exited unsuccessfully: {status}")]
    ServerExited { status: ExitStatus },

    #[error("source mod already exists: {path}; pass --force to replace it")]
    ModAlreadyExists { path: PathBuf },

    #[error("invalid source-mod name: {name:?}")]
    InvalidModName { name: String },

    #[error("source mod not found: {name}")]
    ModNotFound { name: String },

    #[error("invalid source-mod TOML in {path}: {source}")]
    ParseModManifest {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("invalid source-mod manifest {path}: {message}")]
    InvalidModManifest { path: PathBuf, message: String },

    #[error(
        "source mod `{mod_id}` targets Pumpkin revision `{declared}`, but the loader is configured for `{configured}` (resolved `{resolved}`)"
    )]
    ModRevisionMismatch {
        mod_id: String,
        declared: String,
        configured: String,
        resolved: String,
    },

    #[error("source-mod patch does not exist: {path}")]
    MissingModPatch { path: PathBuf },

    #[error(
        "development worktree for source mod `{mod_id}` is not prepared; run `pumpkin-loader mod dev {mod_id}` first"
    )]
    ModWorktreeNotPrepared { mod_id: String },

    #[error("no uncommitted Pumpkin source changes found in {worktree}")]
    NoModChanges { worktree: PathBuf },

    #[error("duplicate source-mod id: {id}")]
    DuplicateModId { id: String },

    #[error("source mod `{mod_id}` requires missing dependency `{dependency}`")]
    MissingModDependency { mod_id: String, dependency: String },

    #[error(
        "source mod `{mod_id}` requires `{dependency}` version {required}, but {actual} is installed"
    )]
    ModVersionMismatch {
        mod_id: String,
        dependency: String,
        required: String,
        actual: String,
    },

    #[error("source mods `{first}` and `{second}` conflict")]
    ModConflict { first: String, second: String },

    #[error("source-mod dependency cycle: {mods:?}")]
    ModDependencyCycle { mods: Vec<String> },

    #[error("mixin `{mixin}` references unknown mixin `{reference}`")]
    UnknownMixinReference { mixin: String, reference: String },

    #[error("mixin dependency cycle: {mixins:?}")]
    MixinDependencyCycle { mixins: Vec<String> },

    #[error("mixin `{mixin}` failed validation: {message}")]
    MixinValidation { mixin: String, message: String },

    #[error("Cargo did not report a cdylib/dylib artifact for the plugin")]
    PluginArtifactNotReported,

    #[error("plugin `{mod_id}` build artifact does not exist: {path}")]
    MissingPluginArtifact { mod_id: String, path: PathBuf },

    #[error("failed to walk source-mod files: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("failed to decode build manifest: {0}")]
    DecodeManifest(#[from] serde_json::Error),

    #[error("failed to encode build metadata: {0}")]
    EncodeMetadata(serde_json::Error),

    #[error("failed to resolve path {path}: {source}")]
    Canonicalize {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl LoaderError {
    pub fn io(operation: &'static str, path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }
}
