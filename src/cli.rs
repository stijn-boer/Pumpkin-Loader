use clap::{Args, Parser, Subcommand};
use std::{ffi::OsString, path::PathBuf};

pub const DEFAULT_CONFIG: &str = "pumpkin-loader.toml";

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Build Pumpkin with rich source mixins and deploy native Pumpkin plugins"
)]
pub struct Cli {
    #[arg(long, default_value = DEFAULT_CONFIG, global = true)]
    pub config: PathBuf,

    /// Increase diagnostic output. Repeat for more detail.
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Suppress informational messages; command results still go to stdout.
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Action,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    Init {
        #[arg(long)]
        force: bool,
    },
    Fetch,
    Build {
        #[arg(long)]
        force: bool,
    },
    Run {
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    Status,
    Clean,

    /// Create and develop Pumpkin plugin/mixin packages.
    Mod {
        #[command(subcommand)]
        command: ModAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum ModAction {
    /// Create a plugin plus rich-mixin skeleton under mods/<name>.
    Init(ModInitArgs),

    /// Reset the persistent Pumpkin worktree and apply this mod's mixins.
    /// Edit the plugin directly or edit the worktree and run `mod patch`.
    Dev(ModNameArgs),

    /// Save current Pumpkin worktree edits as a rich mixin patch.
    Patch(ModPatchArgs),
}

#[derive(Debug, Args)]
pub struct ModInitArgs {
    pub name: String,

    /// Replace an existing mod directory.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
pub struct ModNameArgs {
    pub name: String,
}

#[derive(Debug, Args)]
pub struct ModPatchArgs {
    pub name: String,

    /// Human-readable patch filename suffix, for example `custom-payload-registry`.
    #[arg(long, default_value = "changes")]
    pub patch_name: String,
}
