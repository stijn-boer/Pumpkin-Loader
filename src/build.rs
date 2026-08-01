use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Table, value};

use crate::{
    config::Config,
    error::{LoaderError, Result},
    layout::Layout,
    modding, process,
    sandbox::{self, Purpose},
    source, toolchain,
    util::{executable_name, hash_file, profile_directory, set_executable},
};

const MODDEV_PROFILE: &str = "moddev";

#[derive(Debug, Serialize, Deserialize)]
struct BuildManifest {
    schema: u32,
    build_key: String,
    repository: String,
    revision: String,
    resolved_commit: String,
    rustc_version: String,
    cargo_version: String,
    profile: String,
    binary: String,
    binary_sha256: String,
}

pub fn ensure(config: &Config, layout: &Layout, force: bool) -> Result<PathBuf> {
    let commit = source::fetch(config, layout)?;
    let resolved_mods = modding::resolve_all(config, layout, &commit)?;
    let build_key = calculate_key(config, layout, &commit, &resolved_mods)?;
    let build_dir = layout.builds.join(&build_key);
    let artifact = build_dir.join(&config.build.binary);
    let manifest_path = build_dir.join("manifest.json");

    if !force && artifact.is_file() && manifest_path.is_file() {
        verify_cached(&artifact, &manifest_path, &build_key)?;
        log::info!("Using cached build {}", short_key(&build_key));
        modding::build_and_deploy_plugins(config, layout, &resolved_mods)?;
        return Ok(artifact);
    }

    let worktree = modding::prepare_worktree(config, layout, &commit, &resolved_mods)?;
    inject_requested_profiles(config, &worktree)?;

    compile(config, layout, &worktree, &artifact, &build_key, &commit)?;
    modding::build_and_deploy_plugins(config, layout, &resolved_mods)?;
    Ok(artifact)
}

fn inject_requested_profiles(config: &Config, worktree: &Path) -> Result<()> {
    if config.build.profile != MODDEV_PROFILE {
        return Ok(());
    }

    let manifest_path = worktree.join("Cargo.toml");
    let source = fs::read_to_string(&manifest_path).map_err(|error| {
        LoaderError::io("read Pumpkin workspace manifest", &manifest_path, error)
    })?;
    let mut document =
        source
            .parse::<DocumentMut>()
            .map_err(|source| LoaderError::EditCargoManifest {
                path: manifest_path.clone(),
                source,
            })?;

    if document.get("profile").is_none() {
        document.insert("profile", Item::Table(Table::new()));
    }
    let profiles = document
        .get_mut("profile")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| LoaderError::InvalidModManifest {
            path: manifest_path.clone(),
            message: "Cargo `profile` must be a table".into(),
        })?;
    if profiles.get(MODDEV_PROFILE).is_none() {
        profiles.insert(MODDEV_PROFILE, Item::Table(Table::new()));
    }
    let profile = profiles
        .get_mut(MODDEV_PROFILE)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| LoaderError::InvalidModManifest {
            path: manifest_path.clone(),
            message: format!("Cargo profile `{MODDEV_PROFILE}` must be a table"),
        })?;
    profile.insert("inherits", value("dev"));
    profile.insert("opt-level", value(0));
    profile.insert("debug", value(0));
    profile.insert("incremental", value(true));
    profile.insert("codegen-units", value(256));
    profile.insert("panic", value("abort"));

    fs::write(&manifest_path, document.to_string())
        .map_err(|error| LoaderError::io("inject moddev Cargo profile", &manifest_path, error))?;

    log::info!("Injected Cargo profile `moddev` (opt-level 0, no debuginfo, incremental)");
    Ok(())
}

fn compile(
    config: &Config,
    layout: &Layout,
    worktree: &Path,
    artifact: &Path,
    build_key: &str,
    commit: &str,
) -> Result<()> {
    let toolchain = toolchain::resolve(worktree)?;
    let rustc_version = toolchain::version(&toolchain.rustc)?;
    let cargo_version = toolchain::version(&toolchain.cargo)?;

    let mut args = vec![
        "build".to_string(),
        "--profile".into(),
        config.build.profile.clone(),
        "--message-format=json-render-diagnostics".into(),
    ];
    if config.build.locked {
        args.push("--locked".into());
    }
    if config.build.no_default_features {
        args.push("--no-default-features".into());
    }
    if !config.build.cargo_features.is_empty() {
        args.push("--features".into());
        args.push(config.build.cargo_features.join(","));
    }
    if let Some(jobs) = config.build.jobs {
        args.push("--jobs".into());
        args.push(jobs.to_string());
    }
    args.push("--bin".into());
    args.push(config.build.binary.clone());

    let mut envs = sandbox::minimal_environment();
    envs.insert("CARGO_HOME".into(), layout.cargo_home.as_os_str().into());
    envs.insert(
        "CARGO_TARGET_DIR".into(),
        layout.cargo_target.as_os_str().into(),
    );
    envs.insert("CARGO".into(), toolchain.cargo.as_os_str().into());
    envs.insert("RUSTC".into(), toolchain.rustc.as_os_str().into());
    envs.insert("PATH".into(), toolchain::path(&toolchain.cargo)?);
    envs.insert("RUSTUP_NO_UPDATE_CHECK".into(), "1".into());
    if let Some(rustdoc) = &toolchain.rustdoc {
        envs.insert("RUSTDOC".into(), rustdoc.as_os_str().into());
    }
    if let Some(flags) = &config.build.rustflags {
        envs.insert("RUSTFLAGS".into(), flags.into());
    }
    for (key, value) in &config.build.environment {
        envs.insert(key.into(), value.into());
    }

    let metadata = cargo_metadata(config, layout, worktree, &toolchain.cargo, &envs)?;
    let display_description = format!(
        "Building Pumpkin {} ({})",
        &commit[..commit.len().min(8)],
        config.build.profile
    );
    let mut command = sandbox::command(
        &toolchain.cargo,
        &args,
        worktree,
        &envs,
        config.build.sandbox,
        Purpose::Build,
        layout,
    )?;
    process::cargo_build(
        &mut command,
        "compile Pumpkin",
        &display_description,
        metadata.package_names.len(),
        &metadata.package_names,
    )?;

    let built_binary = layout
        .cargo_target
        .join(profile_directory(&config.build.profile))
        .join(executable_name(&config.build.binary));
    if !built_binary.is_file() {
        return Err(LoaderError::MissingBuildArtifact { path: built_binary });
    }

    let parent = artifact
        .parent()
        .ok_or_else(|| LoaderError::ArtifactWithoutParent(artifact.to_path_buf()))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".{}.tmp", config.build.binary));
    fs::copy(&built_binary, &temporary)
        .map_err(|source| LoaderError::io("stage compiled binary", &built_binary, source))?;
    set_executable(&temporary)?;
    fs::rename(&temporary, artifact)?;

    let manifest = BuildManifest {
        schema: 1,
        build_key: build_key.into(),
        repository: config.pumpkin.repository.clone(),
        revision: config.pumpkin.revision.clone(),
        resolved_commit: commit.into(),
        rustc_version,
        cargo_version,
        profile: config.build.profile.clone(),
        binary: config.build.binary.clone(),
        binary_sha256: hash_file(artifact)?,
    };
    let metadata = serde_json::to_vec_pretty(&manifest).map_err(LoaderError::EncodeMetadata)?;
    let manifest_path = parent.join("manifest.json");
    fs::write(&manifest_path, metadata)
        .map_err(|source| LoaderError::io("write build manifest", manifest_path, source))?;
    log::info!("Stored build artifact at {}", artifact.display());
    Ok(())
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoMetadataPackage>,
    resolve: Option<CargoResolve>,
}

#[derive(Debug, Deserialize)]
struct CargoMetadataPackage {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CargoResolve {
    nodes: Vec<CargoResolveNode>,
}

#[derive(Debug, Deserialize)]
struct CargoResolveNode {
    id: String,
}

struct BuildMetadata {
    package_names: HashMap<String, String>,
}

fn cargo_metadata(
    config: &Config,
    layout: &Layout,
    worktree: &Path,
    cargo: &Path,
    envs: &BTreeMap<std::ffi::OsString, std::ffi::OsString>,
) -> Result<BuildMetadata> {
    let mut args = vec!["metadata".to_owned(), "--format-version=1".to_owned()];
    if config.build.locked {
        args.push("--locked".to_owned());
    }
    if config.build.no_default_features {
        args.push("--no-default-features".to_owned());
    }
    if !config.build.cargo_features.is_empty() {
        args.push("--features".to_owned());
        args.push(config.build.cargo_features.join(","));
    }

    let mut command = sandbox::command(
        cargo,
        &args,
        worktree,
        envs,
        config.build.sandbox,
        Purpose::Build,
        layout,
    )?;
    let output = process::output(&mut command, "inspect Cargo dependency graph")?;
    let metadata: CargoMetadata = serde_json::from_str(&output)?;

    let all_names: HashMap<_, _> = metadata
        .packages
        .into_iter()
        .map(|package| (package.id, package.name))
        .collect();

    let package_names = match metadata.resolve {
        Some(resolve) => resolve
            .nodes
            .into_iter()
            .filter_map(|node| all_names.get(&node.id).cloned().map(|name| (node.id, name)))
            .collect(),
        None => all_names,
    };

    log::debug!(
        "Cargo dependency graph contains {} packages",
        package_names.len()
    );
    Ok(BuildMetadata { package_names })
}

pub fn calculate_key(
    config: &Config,
    _layout: &Layout,
    commit: &str,
    mods: &modding::ResolvedMods,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"pumpkin-loader-rich-mixins-v1\0");
    hasher.update(config.pumpkin.repository.as_bytes());
    hasher.update([0]);
    hasher.update(commit.as_bytes());
    hasher.update([0]);
    hasher.update(
        process::output(
            Command::new("rustc").arg("--version"),
            "query rustc version",
        )?
        .as_bytes(),
    );
    hasher.update([0]);
    hasher.update(
        process::output(
            Command::new("cargo").arg("--version"),
            "query Cargo version",
        )?
        .as_bytes(),
    );
    hasher.update([0]);
    hasher.update(serde_json::to_vec(&BuildKeyConfig::from(config))?);
    hasher.update(modding::hash_mixins(mods)?);
    Ok(hex::encode(hasher.finalize()))
}

fn verify_cached(artifact: &Path, manifest_path: &Path, expected_key: &str) -> Result<()> {
    let bytes = fs::read(manifest_path)
        .map_err(|source| LoaderError::io("read build manifest", manifest_path, source))?;
    let manifest: BuildManifest = serde_json::from_slice(&bytes)?;
    if manifest.build_key != expected_key {
        return Err(LoaderError::CacheKeyMismatch);
    }
    let actual = hash_file(artifact)?;
    if actual != manifest.binary_sha256 {
        return Err(LoaderError::IntegrityMismatch {
            expected: manifest.binary_sha256,
            actual,
        });
    }
    Ok(())
}

fn short_key(key: &str) -> &str {
    &key[..key.len().min(12)]
}

#[derive(Serialize)]
struct BuildKeyConfig<'a> {
    profile: &'a str,
    binary: &'a str,
    cargo_features: &'a [String],
    no_default_features: bool,
    locked: bool,
    rustflags: &'a Option<String>,
    environment: &'a BTreeMap<String, String>,
}

impl<'a> From<&'a Config> for BuildKeyConfig<'a> {
    fn from(config: &'a Config) -> Self {
        Self {
            profile: &config.build.profile,
            binary: &config.build.binary,
            cargo_features: &config.build.cargo_features,
            no_default_features: config.build.no_default_features,
            locked: config.build.locked,
            rustflags: &config.build.rustflags,
            environment: &config.build.environment,
        }
    }
}
