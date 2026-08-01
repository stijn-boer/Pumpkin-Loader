use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{
    config::Config,
    error::{LoaderError, Result},
    layout::Layout,
    logging, process,
};

pub fn fetch(config: &Config, layout: &Layout) -> Result<String> {
    let op = logging::Operation::start(format!(
        "Fetching Pumpkin revision {}",
        config.pumpkin.revision
    ));

    if !layout.bare_repo.exists() {
        process::checked(
            Command::new("git")
                .arg("clone")
                .arg("--bare")
                .arg("--filter=blob:none")
                .arg(&config.pumpkin.repository)
                .arg(&layout.bare_repo),
            "clone Pumpkin source cache",
        )?;
    } else {
        process::checked(
            Command::new("git")
                .arg("--git-dir")
                .arg(&layout.bare_repo)
                .arg("remote")
                .arg("set-url")
                .arg("origin")
                .arg(&config.pumpkin.repository),
            "update Pumpkin remote URL",
        )?;
    }

    process::checked(
        Command::new("git")
            .arg("--git-dir")
            .arg(&layout.bare_repo)
            .arg("fetch")
            .arg("--force")
            .arg("--prune")
            .arg("origin")
            .arg(&config.pumpkin.revision),
        "fetch configured Pumpkin revision",
    )?;

    let commit = process::output(
        Command::new("git")
            .arg("--git-dir")
            .arg(&layout.bare_repo)
            .arg("rev-parse")
            .arg("FETCH_HEAD^{commit}"),
        "resolve Pumpkin revision",
    )?;

    op.finish();
    Ok(commit)
}

pub fn resolve_cached_commit(config: &Config, layout: &Layout) -> Result<String> {
    if !layout.bare_repo.exists() {
        return Err(LoaderError::SourceCacheEmpty);
    }

    for candidate in [
        config.pumpkin.revision.clone(),
        format!("refs/remotes/origin/{}", config.pumpkin.revision),
        "FETCH_HEAD".into(),
    ] {
        if let Ok(output) = process::output(
            Command::new("git")
                .arg("--git-dir")
                .arg(&layout.bare_repo)
                .arg("rev-parse")
                .arg(format!("{candidate}^{{commit}}")),
            "resolve cached commit",
        ) {
            return Ok(output);
        }
    }

    Err(LoaderError::RevisionUnavailable {
        revision: config.pumpkin.revision.clone(),
    })
}

/// Returns a stable worktree for a resolved Pumpkin commit.
///
/// The path deliberately depends only on the source revision, not the complete
/// mod/build key. This allows Cargo incremental state to remain reusable when
/// patches or build settings change.
pub fn prepare_worktree(layout: &Layout, commit: &str) -> Result<PathBuf> {
    let worktree = layout.worktrees.join(commit);
    fs::create_dir_all(&layout.worktrees).map_err(|source| {
        LoaderError::io("create worktree directory", &layout.worktrees, source)
    })?;

    if worktree.join(".git").exists() {
        log::info!("Reusing Pumpkin worktree {}", worktree.display());
        reset_worktree(&worktree, commit)?;
    } else {
        if worktree.exists() {
            fs::remove_dir_all(&worktree)
                .map_err(|source| LoaderError::io("remove invalid worktree", &worktree, source))?;
        }

        process::checked(
            Command::new("git")
                .arg("--git-dir")
                .arg(&layout.bare_repo)
                .arg("worktree")
                .arg("add")
                .arg("--detach")
                .arg(&worktree)
                .arg(commit),
            "create persistent Pumpkin worktree",
        )?;
    }

    initialize_submodules(&worktree)?;
    Ok(worktree)
}

fn reset_worktree(worktree: &Path, commit: &str) -> Result<()> {
    process::checked(
        Command::new("git")
            .current_dir(worktree)
            .arg("checkout")
            .arg("--detach")
            .arg("--force")
            .arg(commit),
        "reset Pumpkin worktree revision",
    )?;

    process::checked(
        Command::new("git")
            .current_dir(worktree)
            .arg("reset")
            .arg("--hard")
            .arg(commit),
        "reset Pumpkin worktree contents",
    )?;

    process::checked(
        Command::new("git")
            .current_dir(worktree)
            .arg("clean")
            .arg("-ffdx"),
        "clean Pumpkin worktree",
    )?;

    Ok(())
}

fn initialize_submodules(worktree: &Path) -> Result<()> {
    log::info!("Initializing Pumpkin submodules");

    process::checked(
        Command::new("git")
            .current_dir(worktree)
            .arg("submodule")
            .arg("sync")
            .arg("--recursive"),
        "sync Git submodules",
    )?;

    process::checked(
        Command::new("git")
            .current_dir(worktree)
            .arg("-c")
            .arg("protocol.file.allow=always")
            .arg("submodule")
            .arg("update")
            .arg("--init")
            .arg("--recursive")
            .arg("--force"),
        "initialize Git submodules",
    )?;

    process::checked(
        Command::new("git")
            .current_dir(worktree)
            .arg("submodule")
            .arg("foreach")
            .arg("--recursive")
            .arg("git reset --hard"),
        "reset Git submodules",
    )?;

    process::checked(
        Command::new("git")
            .current_dir(worktree)
            .arg("submodule")
            .arg("foreach")
            .arg("--recursive")
            .arg("git clean -ffdx"),
        "clean Git submodules",
    )?;

    Ok(())
}

pub fn remove_worktree(layout: &Layout, path: &Path) -> Result<()> {
    if layout.bare_repo.exists() {
        let output = Command::new("git")
            .arg("--git-dir")
            .arg(&layout.bare_repo)
            .arg("worktree")
            .arg("remove")
            .arg("--force")
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        if let Err(error) = output {
            log::debug!("could not unregister worktree {}: {error}", path.display());
        }
    }

    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|source| LoaderError::io("remove worktree", path, source))?;
    }

    Ok(())
}

pub fn clean(layout: &Layout) -> Result<()> {
    if layout.worktrees.exists() {
        for entry in fs::read_dir(&layout.worktrees)? {
            remove_worktree(layout, &entry?.path())?;
        }
    }

    if layout.bare_repo.exists() {
        process::checked(
            Command::new("git")
                .arg("--git-dir")
                .arg(&layout.bare_repo)
                .arg("worktree")
                .arg("prune"),
            "prune worktree metadata",
        )?;
    }

    log::info!("Removed persistent worktrees");
    Ok(())
}
