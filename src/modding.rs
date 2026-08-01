use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Value};
use walkdir::WalkDir;

use crate::{
    config::Config,
    error::{LoaderError, Result},
    layout::Layout,
    process,
    sandbox::{self, Purpose},
    source, toolchain,
    util::profile_directory,
};

const MANIFEST_NAME: &str = "pumpkin-mod.toml";
const DEFAULT_FORMAT: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModManifest {
    #[serde(default = "default_format")]
    pub format: u32,
    #[serde(rename = "mod")]
    pub mod_info: ModInfo,
    pub pumpkin: ModPumpkin,
    pub plugin: PluginDefinition,
    #[serde(default)]
    pub dependencies: ModDependencies,
    #[serde(default)]
    pub mixins: Vec<MixinDefinition>,
}

fn default_format() -> u32 { DEFAULT_FORMAT }
fn default_true() -> bool { true }
fn default_plugin_path() -> PathBuf { PathBuf::from(".") }
fn is_default_plugin_path(path: &PathBuf) -> bool { path.as_path() == Path::new(".") || path.as_os_str().is_empty() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModInfo {
    pub id: String,
    pub name: String,
    pub version: Version,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModPumpkin {
    /// Legacy single-revision field. Kept for backwards compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Pumpkin commits supported by this mod package. Short commit prefixes are accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<String>,
}

impl ModPumpkin {
    fn supported_revisions(&self) -> impl Iterator<Item = &str> {
        self.revision
            .iter()
            .map(String::as_str)
            .chain(self.revisions.iter().map(String::as_str))
    }

    fn supports(&self, resolved_commit: &str) -> bool {
        self.supported_revisions()
            .any(|revision| resolved_commit == revision || resolved_commit.starts_with(revision))
    }

    fn display_revisions(&self) -> String {
        let revisions = self.supported_revisions().collect::<Vec<_>>();
        if revisions.is_empty() { "<none>".into() } else { revisions.join(", ") }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDefinition {
    #[serde(default = "default_plugin_path", skip_serializing_if = "is_default_plugin_path")]
    pub path: PathBuf,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub artifact: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModDependencies {
    #[serde(default)] pub required: Vec<ModDependency>,
    #[serde(default)] pub conflicts: Vec<ModDependency>,
    #[serde(default)] pub load_after: Vec<String>,
    #[serde(default)] pub load_before: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDependency {
    pub id: String,
    #[serde(default = "any_version")]
    pub version: VersionReq,
}
fn any_version() -> VersionReq { VersionReq::STAR }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixinDefinition {
    pub id: String,
    #[serde(default)] pub description: Option<String>,
    pub patch: PathBuf,
    #[serde(default = "default_true")] pub required: bool,
    #[serde(default)] pub after: Vec<String>,
    #[serde(default)] pub before: Vec<String>,
    #[serde(default)] pub target: MixinTarget,
    #[serde(default)] pub validation: MixinValidation,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MixinTarget {
    #[serde(default)] pub package: Option<String>,
    #[serde(default)] pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MixinValidation {
    #[serde(default)] pub contains: Vec<String>,
    #[serde(default)] pub not_contains: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredMod {
    pub directory: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest: ModManifest,
}

#[derive(Debug, Clone)]
pub struct ResolvedMods { pub ordered: Vec<DiscoveredMod> }
impl ResolvedMods {
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.ordered.iter().map(|item| item.manifest.mod_info.id.as_str())
    }
}

#[derive(Clone)]
struct ResolvedMixin<'a> {
    qualified_id: String,
    owner: &'a DiscoveredMod,
    definition: &'a MixinDefinition,
}

pub fn init(config: &Config, layout: &Layout, name: &str, force: bool) -> Result<PathBuf> {
    validate_name(name)?;
    let slug = slugify(name);
    let mod_dir = layout.mods.join(&slug);
    if mod_dir.exists() {
        if !force { return Err(LoaderError::ModAlreadyExists { path: mod_dir }); }
        fs::remove_dir_all(&mod_dir)
            .map_err(|source| LoaderError::io("remove existing mod directory", &mod_dir, source))?;
    }

    fs::create_dir_all(mod_dir.join("src"))?;
    fs::create_dir_all(mod_dir.join("mixins/patches"))?;
    let revision = source::fetch(config, layout)?;
    let worktree = source::prepare_worktree(layout, &revision)?;
    let pumpkin = find_package(&worktree, "pumpkin")?;
    let pumpkin_data = find_package(&worktree, "pumpkin-data")?;
    let pumpkin_protocol = find_package(&worktree, "pumpkin-protocol")?;
    let pumpkin_util = find_package(&worktree, "pumpkin-util")?;
    let pumpkin_api_macros = find_package(&worktree, "pumpkin-api-macros")?;
    let manifest = ModManifest {
        format: DEFAULT_FORMAT,
        mod_info: ModInfo {
            id: slug.clone(), name: name.into(), version: Version::new(0, 1, 0),
            description: None, authors: Vec::new(), license: None,
        },
        pumpkin: ModPumpkin { revision: None, revisions: vec![revision] },
        plugin: PluginDefinition { path: default_plugin_path(), package: None, artifact: None },
        dependencies: ModDependencies::default(),
        mixins: Vec::new(),
    };
    write_manifest(&mod_dir.join(MANIFEST_NAME), &manifest)?;
    let pumpkin = toml_path(&pumpkin);
    let pumpkin_data = toml_path(&pumpkin_data);
    let pumpkin_protocol = toml_path(&pumpkin_protocol);
    let pumpkin_util = toml_path(&pumpkin_util);
    let pumpkin_api_macros = toml_path(&pumpkin_api_macros);
    fs::write(mod_dir.join("Cargo.toml"), render_template(
        include_str!("../embedded/templates/plugin/Cargo.toml.template"),
        &[
            ("{{MOD_ID}}", &slug),
            ("{{MOD_NAME}}", name),
            ("{{PUMPKIN_PATH}}", &pumpkin),
            ("{{PUMPKIN_DATA_PATH}}", &pumpkin_data),
            ("{{PUMPKIN_PROTOCOL_PATH}}", &pumpkin_protocol),
            ("{{PUMPKIN_UTIL_PATH}}", &pumpkin_util),
            ("{{PUMPKIN_API_MACROS_PATH}}", &pumpkin_api_macros),
        ],
    ))?;
    fs::write(mod_dir.join("src/lib.rs"), render_template(
        include_str!("../embedded/templates/plugin/src/lib.rs.template"),
        &[("{{MOD_ID}}", &slug), ("{{MOD_NAME}}", name)],
    ))?;
    log::info!("Created plugin/mixin mod package `{slug}` at {}", mod_dir.display());
    Ok(mod_dir)
}

pub fn discover(config: &Config, layout: &Layout, commit: &str) -> Result<HashMap<String, DiscoveredMod>> {
    let mut result = HashMap::new();
    for entry in fs::read_dir(&layout.mods)
        .map_err(|source| LoaderError::io("read mods directory", &layout.mods, source))?
    {
        let directory = entry?.path();
        if !directory.is_dir() || !directory.join(MANIFEST_NAME).is_file() { continue; }
        let manifest_path = directory.join(MANIFEST_NAME);
        let manifest = load_manifest(&directory)?;
        if manifest.format != DEFAULT_FORMAT {
            return Err(LoaderError::InvalidModManifest { path: manifest_path, message: format!("unsupported manifest format {}; expected {DEFAULT_FORMAT}", manifest.format) });
        }
        ensure_revision(&manifest, commit, config)?;
        validate_name(&manifest.mod_info.id)?;
        validate_manifest_paths(&directory, &manifest)?;
        let id = manifest.mod_info.id.clone();
        if result.insert(id.clone(), DiscoveredMod { directory, manifest_path, manifest }).is_some() {
            return Err(LoaderError::DuplicateModId { id });
        }
    }
    Ok(result)
}


fn plugin_root(directory: &Path, manifest: &ModManifest) -> PathBuf {
    if manifest.plugin.path.as_os_str().is_empty() || manifest.plugin.path.as_path() == Path::new(".") {
        directory.to_path_buf()
    } else {
        directory.join(&manifest.plugin.path)
    }
}

fn validate_manifest_paths(directory: &Path, manifest: &ModManifest) -> Result<()> {
    let supported_revisions = manifest.pumpkin.supported_revisions().collect::<Vec<_>>();
    if supported_revisions.is_empty() || supported_revisions.iter().any(|revision| revision.trim().is_empty()) {
        return Err(LoaderError::InvalidModManifest {
            path: directory.join(MANIFEST_NAME),
            message: "`pumpkin.revisions` must contain at least one non-empty revision".into(),
        });
    }
    let plugin = plugin_root(directory, manifest);
    if !plugin.join("Cargo.toml").is_file() {
        return Err(LoaderError::InvalidModManifest { path: directory.join(MANIFEST_NAME), message: format!("plugin root `{}` does not contain Cargo.toml", plugin.display()) });
    }
    let mut ids = BTreeSet::new();
    for mixin in &manifest.mixins {
        validate_name(&mixin.id)?;
        if !ids.insert(mixin.id.clone()) {
            return Err(LoaderError::InvalidModManifest { path: directory.join(MANIFEST_NAME), message: format!("duplicate mixin id `{}`", mixin.id) });
        }
        let patch = directory.join(&mixin.patch);
        if !patch.is_file() { return Err(LoaderError::MissingModPatch { path: patch }); }
    }
    Ok(())
}

pub fn resolve_all(config: &Config, layout: &Layout, commit: &str) -> Result<ResolvedMods> {
    let discovered = discover(config, layout, commit)?;
    let roots = discovered.keys().cloned().collect();
    resolve_from(discovered, roots)
}

pub fn resolve_for_dev(config: &Config, layout: &Layout, commit: &str, selected: &str) -> Result<ResolvedMods> {
    let discovered = discover(config, layout, commit)?;
    let selected = if discovered.contains_key(selected) { selected.to_owned() } else { slugify(selected) };
    if !discovered.contains_key(&selected) { return Err(LoaderError::ModNotFound { name: selected }); }
    resolve_from(discovered, BTreeSet::from([selected]))
}

fn resolve_from(discovered: HashMap<String, DiscoveredMod>, roots: BTreeSet<String>) -> Result<ResolvedMods> {
    let mut selected = BTreeSet::new();
    let mut stack: Vec<String> = roots.into_iter().collect();
    while let Some(id) = stack.pop() {
        if !selected.insert(id.clone()) { continue; }
        let item = discovered.get(&id).ok_or_else(|| LoaderError::MissingModDependency { mod_id: "root".into(), dependency: id.clone() })?;
        for dependency in &item.manifest.dependencies.required {
            let target = discovered.get(&dependency.id).ok_or_else(|| LoaderError::MissingModDependency { mod_id: id.clone(), dependency: dependency.id.clone() })?;
            if !dependency.version.matches(&target.manifest.mod_info.version) {
                return Err(LoaderError::ModVersionMismatch { mod_id: id.clone(), dependency: dependency.id.clone(), required: dependency.version.to_string(), actual: target.manifest.mod_info.version.to_string() });
            }
            stack.push(dependency.id.clone());
        }
    }
    for id in &selected {
        let item = &discovered[id];
        for conflict in &item.manifest.dependencies.conflicts {
            if let Some(other) = discovered.get(&conflict.id)
                && selected.contains(&conflict.id)
                && conflict.version.matches(&other.manifest.mod_info.version)
            { return Err(LoaderError::ModConflict { first: id.clone(), second: conflict.id.clone() }); }
        }
    }
    let mut edges: BTreeMap<String, BTreeSet<String>> = selected.iter().map(|id| (id.clone(), BTreeSet::new())).collect();
    let mut indegree: BTreeMap<String, usize> = selected.iter().map(|id| (id.clone(), 0)).collect();
    let mut add_edge = |from: &str, to: &str| {
        if selected.contains(from) && selected.contains(to) && edges.get_mut(from).is_some_and(|set| set.insert(to.to_owned())) {
            *indegree.get_mut(to).expect("selected node") += 1;
        }
    };
    for id in &selected {
        let item = &discovered[id];
        for dependency in &item.manifest.dependencies.required { add_edge(&dependency.id, id); }
        for before in &item.manifest.dependencies.load_before { add_edge(id, before); }
        for after in &item.manifest.dependencies.load_after { add_edge(after, id); }
    }
    let mut ready: BTreeSet<String> = indegree.iter().filter(|(_, value)| **value == 0).map(|(id, _)| id.clone()).collect();
    let mut order = Vec::new();
    while let Some(id) = ready.pop_first() {
        order.push(id.clone());
        for target in edges[&id].clone() {
            let value = indegree.get_mut(&target).expect("selected node");
            *value -= 1;
            if *value == 0 { ready.insert(target); }
        }
    }
    if order.len() != selected.len() {
        return Err(LoaderError::ModDependencyCycle { mods: indegree.into_iter().filter(|(_, n)| *n > 0).map(|(id, _)| id).collect() });
    }
    Ok(ResolvedMods { ordered: order.into_iter().map(|id| discovered[&id].clone()).collect() })
}

pub fn prepare_worktree(_config: &Config, layout: &Layout, commit: &str, mods: &ResolvedMods) -> Result<PathBuf> {
    let worktree = source::prepare_worktree(layout, commit)?;
    apply_mixins(&worktree, commit, mods)?;
    log::info!("Prepared mixins for {} mods: {}", mods.ordered.len(), mods.ids().collect::<Vec<_>>().join(", "));
    Ok(worktree)
}

pub fn prepare_dev(config: &Config, layout: &Layout, name: &str) -> Result<PathBuf> {
    let commit = source::fetch(config, layout)?;
    let mods = resolve_for_dev(config, layout, &commit, name)?;
    let worktree = prepare_worktree(config, layout, &commit, &mods)?;
    let selected = resolve_mod(layout, name)?;
    let manifest = load_manifest(&selected)?;
    log::info!("Plugin source: {}", plugin_root(&selected, &manifest).display());
    log::info!("Mixin worktree: {}", worktree.display());
    Ok(worktree)
}

fn apply_mixins(worktree: &Path, commit: &str, mods: &ResolvedMods) -> Result<()> {
    for mixin in resolve_mixins(mods)? {
        match validate_mixin(worktree, commit, &mixin) {
            Ok(()) => {
                let patch = mixin.owner.directory.join(&mixin.definition.patch);
                log::info!("Applying mixin `{}` from {}", mixin.qualified_id, patch.display());
                process::checked(
                    Command::new("git").current_dir(worktree).args(["apply", "--index", "--3way"]).arg(&patch),
                    &format!("apply mixin {}", mixin.qualified_id),
                )?;
            }
            Err(error) if !mixin.definition.required => {
                log::warn!("Skipping optional mixin `{}`: {error}", mixin.qualified_id);
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn resolve_mixins(mods: &ResolvedMods) -> Result<Vec<ResolvedMixin<'_>>> {
    let mut all = BTreeMap::<String, ResolvedMixin<'_>>::new();
    for owner in &mods.ordered {
        for definition in &owner.manifest.mixins {
            let id = format!("{}:{}", owner.manifest.mod_info.id, definition.id);
            all.insert(id.clone(), ResolvedMixin { qualified_id: id, owner, definition });
        }
    }
    let mut edges: BTreeMap<String, BTreeSet<String>> = all.keys().map(|id| (id.clone(), BTreeSet::new())).collect();
    let mut indegree: BTreeMap<String, usize> = all.keys().map(|id| (id.clone(), 0)).collect();
    for (id, mixin) in &all {
        for after in &mixin.definition.after {
            let dependency = qualify_mixin_ref(&mixin.owner.manifest.mod_info.id, after);
            if !all.contains_key(&dependency) { return Err(LoaderError::UnknownMixinReference { mixin: id.clone(), reference: dependency }); }
            if edges.get_mut(&dependency).expect("mixin node").insert(id.clone()) { *indegree.get_mut(id).expect("mixin node") += 1; }
        }
        for before in &mixin.definition.before {
            let target = qualify_mixin_ref(&mixin.owner.manifest.mod_info.id, before);
            if !all.contains_key(&target) { return Err(LoaderError::UnknownMixinReference { mixin: id.clone(), reference: target }); }
            if edges.get_mut(id).expect("mixin node").insert(target.clone()) { *indegree.get_mut(&target).expect("mixin node") += 1; }
        }
    }
    let mut ready: BTreeSet<_> = indegree.iter().filter(|(_, n)| **n == 0).map(|(id, _)| id.clone()).collect();
    let mut ordered = Vec::new();
    while let Some(id) = ready.pop_first() {
        ordered.push(all[&id].clone());
        for target in edges[&id].clone() {
            let n = indegree.get_mut(&target).expect("mixin node"); *n -= 1; if *n == 0 { ready.insert(target); }
        }
    }
    if ordered.len() != all.len() {
        return Err(LoaderError::MixinDependencyCycle { mixins: indegree.into_iter().filter(|(_, n)| *n > 0).map(|(id, _)| id).collect() });
    }
    Ok(ordered)
}

fn qualify_mixin_ref(owner: &str, reference: &str) -> String {
    if reference.contains(':') { reference.to_owned() } else { format!("{owner}:{reference}") }
}

fn validate_mixin(worktree: &Path, commit: &str, mixin: &ResolvedMixin<'_>) -> Result<()> {
    let definition = mixin.definition;
    if !mixin.owner.manifest.pumpkin.supports(commit) {
        return Err(LoaderError::MixinValidation {
            mixin: mixin.qualified_id.clone(),
            message: format!(
                "commit {commit} is not supported by mod `{}` (supported: {})",
                mixin.owner.manifest.mod_info.id,
                mixin.owner.manifest.pumpkin.display_revisions(),
            ),
        });
    }
    let package_root = if let Some(package) = &definition.target.package { find_package(worktree, package)? } else { worktree.to_path_buf() };
    let files: Vec<PathBuf> = definition.target.files.iter().map(|file| package_root.join(file)).collect();
    for file in &files {
        if !file.is_file() { return Err(LoaderError::MixinValidation { mixin: mixin.qualified_id.clone(), message: format!("target file does not exist: {}", file.display()) }); }
    }
    let combined = files.iter().map(fs::read_to_string).collect::<std::io::Result<Vec<_>>>()?.join("\n");
    for needle in &definition.validation.contains {
        if !combined.contains(needle) { return Err(LoaderError::MixinValidation { mixin: mixin.qualified_id.clone(), message: format!("required text was not found in target files: {needle:?}") }); }
    }
    for needle in &definition.validation.not_contains {
        if combined.contains(needle) { return Err(LoaderError::MixinValidation { mixin: mixin.qualified_id.clone(), message: format!("forbidden text is already present in target files: {needle:?}") }); }
    }
    if !definition.target.files.is_empty() {
        let patch = mixin.owner.directory.join(&definition.patch);
        let output = process::output(Command::new("git").current_dir(worktree).args(["apply", "--numstat"]).arg(&patch), "inspect mixin patch paths")?;
        let allowed: BTreeSet<PathBuf> = definition.target.files.iter().map(|path| package_root.strip_prefix(worktree).unwrap_or(Path::new("")).join(path)).collect();
        for line in output.lines() {
            let Some(path) = line.split('\t').nth(2) else { continue; };
            let path = PathBuf::from(path);
            if !allowed.contains(&path) {
                return Err(LoaderError::MixinValidation { mixin: mixin.qualified_id.clone(), message: format!("patch changes undeclared target file `{}`", path.display()) });
            }
        }
    }
    Ok(())
}

pub fn build_and_deploy_plugins(config: &Config, layout: &Layout, commit: &str, mods: &ResolvedMods) -> Result<()> {
    let plugins_dir = layout.server_data.join("plugins");
    fs::create_dir_all(&plugins_dir)?;
    let expected: BTreeSet<String> = mods.ordered.iter().map(|item| item.manifest.mod_info.id.clone()).collect();
    for entry in fs::read_dir(&plugins_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_prefix("pumpkin-loader-").and_then(|name| name.split('.').next())
            && !expected.contains(id)
        { let _ = fs::remove_file(entry.path()); }
    }
    for item in &mods.ordered { build_and_deploy_plugin(config, layout, commit, item, &plugins_dir)?; }
    Ok(())
}

fn build_and_deploy_plugin(config: &Config, layout: &Layout, commit: &str, item: &DiscoveredMod, plugins_dir: &Path) -> Result<()> {
    let plugin_root = plugin_root(&item.directory, &item.manifest);
    let worktree = layout.worktrees.join(commit);
    rewrite_plugin_worktree_paths(&plugin_root.join("Cargo.toml"), &worktree)?;
    let toolchain = toolchain::resolve(&worktree)?;
    let profile = if config.build.profile == "moddev" { "dev" } else { &config.build.profile };
    let mut args = vec!["build".to_owned(), "--manifest-path".to_owned(), plugin_root.join("Cargo.toml").display().to_string(), "--profile".to_owned(), profile.to_owned(), "--message-format=json-render-diagnostics".to_owned()];
    if let Some(package) = &item.manifest.plugin.package { args.extend(["--package".to_owned(), package.clone()]); }
    if config.build.locked && plugin_root.join("Cargo.lock").is_file() { args.push("--locked".to_owned()); }
    if let Some(jobs) = config.build.jobs { args.extend(["--jobs".to_owned(), jobs.to_string()]); }
    let mut envs = sandbox::minimal_environment();
    envs.insert("CARGO_HOME".into(), layout.cargo_home.as_os_str().into());
    envs.insert("CARGO_TARGET_DIR".into(), layout.plugin_target.as_os_str().into());
    envs.insert("CARGO".into(), toolchain.cargo.as_os_str().into());
    envs.insert("RUSTC".into(), toolchain.rustc.as_os_str().into());
    envs.insert("PATH".into(), toolchain::path(&toolchain.cargo)?);
    envs.insert("RUSTUP_NO_UPDATE_CHECK".into(), "1".into());
    let mut command = sandbox::command(&toolchain.cargo, &args, &plugin_root, &envs, config.build.sandbox, Purpose::Build, layout)?;
    let output = process::output(&mut command, &format!("build plugin {}", item.manifest.mod_info.id))?;
    let artifact = if let Some(path) = &item.manifest.plugin.artifact {
        layout.plugin_target.join(profile_directory(profile)).join(path)
    } else {
        find_dynamic_library_artifact(&output, item.manifest.plugin.package.as_deref())?
    };
    if !artifact.is_file() { return Err(LoaderError::MissingPluginArtifact { mod_id: item.manifest.mod_info.id.clone(), path: artifact }); }
    let extension = artifact.extension().and_then(|value| value.to_str()).unwrap_or("so");
    let destination = plugins_dir.join(format!("pumpkin-loader-{}.{}", item.manifest.mod_info.id, extension));
    fs::copy(&artifact, &destination).map_err(|source| LoaderError::io("deploy Pumpkin plugin", &destination, source))?;
    log::info!("Deployed plugin `{}` to {}", item.manifest.mod_info.id, destination.display());
    Ok(())
}

fn rewrite_plugin_worktree_paths(manifest_path: &Path, worktree: &Path) -> Result<()> {
    let source = fs::read_to_string(manifest_path)
        .map_err(|error| LoaderError::io("read plugin Cargo manifest", manifest_path, error))?;
    let mut document = source
        .parse::<DocumentMut>()
        .map_err(|source| LoaderError::EditCargoManifest { path: manifest_path.to_path_buf(), source })?;

    let package_paths = [
        ("pumpkin", find_package(worktree, "pumpkin")?),
        ("pumpkin-data", find_package(worktree, "pumpkin-data")?),
        ("pumpkin-protocol", find_package(worktree, "pumpkin-protocol")?),
        ("pumpkin-util", find_package(worktree, "pumpkin-util")?),
        ("pumpkin-api-macros", find_package(worktree, "pumpkin-api-macros")?),
    ];

    let Some(dependencies) = document.get_mut("dependencies").and_then(Item::as_table_mut) else {
        return Ok(());
    };

    for (dependency_name, dependency) in dependencies.iter_mut() {
        let package_name = dependency
            .as_inline_table()
            .and_then(|table| table.get("package"))
            .and_then(Value::as_str)
            .unwrap_or(&dependency_name)
            .to_owned();

        let Some((_, package_path)) = package_paths.iter().find(|(name, _)| *name == package_name.as_str()) else {
            continue;
        };
        let path = toml_path(package_path);

        if let Some(table) = dependency.as_inline_table_mut() {
            table.insert("path", Value::from(path));
        } else if let Some(table) = dependency.as_table_mut() {
            table.insert("path", toml_edit::value(path));
        }
    }

    let rendered = document.to_string();
    if rendered != source {
        fs::write(manifest_path, rendered)
            .map_err(|error| LoaderError::io("update plugin Pumpkin worktree paths", manifest_path, error))?;
    }
    Ok(())
}

fn find_dynamic_library_artifact(output: &str, package: Option<&str>) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for line in output.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else { continue; };
        if value.get("reason").and_then(|v| v.as_str()) != Some("compiler-artifact") { continue; }
        if let Some(package) = package {
            let target_name = value.pointer("/target/name").and_then(|v| v.as_str());
            if target_name.map(|name| name.replace('-', "_")) != Some(package.replace('-', "_")) { continue; }
        }
        let kinds = value.pointer("/target/crate_types").and_then(|v| v.as_array());
        if !kinds.is_some_and(|items| items.iter().any(|item| matches!(item.as_str(), Some("cdylib" | "dylib")))) { continue; }
        if let Some(files) = value.get("filenames").and_then(|v| v.as_array()) {
            for file in files.iter().filter_map(|v| v.as_str()) {
                if matches!(Path::new(file).extension().and_then(|v| v.to_str()), Some("so" | "dll" | "dylib")) { candidates.push(PathBuf::from(file)); }
            }
        }
    }
    candidates.sort(); candidates.pop().ok_or(LoaderError::PluginArtifactNotReported)
}

pub fn sync_crate(layout: &Layout, name: &str) -> Result<PathBuf> {
    let mod_dir = resolve_mod(layout, name)?;
    let manifest = load_manifest(&mod_dir)?;
    Ok(plugin_root(&mod_dir, &manifest))
}

pub fn create_patch(config: &Config, layout: &Layout, name: &str, patch_name: &str) -> Result<PathBuf> {
    let mod_dir = resolve_mod(layout, name)?;
    let manifest = load_manifest(&mod_dir)?;
    let commit = source::fetch(config, layout)?;
    ensure_revision(&manifest, &commit, config)?;
    let worktree = layout.worktrees.join(&commit);
    if !worktree.join(".git").exists() { return Err(LoaderError::ModWorktreeNotPrepared { mod_id: manifest.mod_info.id }); }
    let mut diff = process::output(Command::new("git").current_dir(&worktree).args(["diff", "--binary", "--full-index", "--no-ext-diff", "--src-prefix=a/", "--dst-prefix=b/", "--", ".", ":(glob,exclude)**/Cargo.lock"]), "generate mixin patch")?;
    if diff.trim().is_empty() { return Err(LoaderError::NoModChanges { worktree }); }
    // `process::output` trims command output. Unified diff files must end in a
    // newline, otherwise `git apply` can report the final hunk as corrupt.
    if !diff.ends_with('\n') {
        diff.push('\n');
    }
    let patches = mod_dir.join("mixins/patches"); fs::create_dir_all(&patches)?;
    let filename = format!("{:03}-{}.patch", next_patch_number(&patches)?, slugify(patch_name));
    let relative = PathBuf::from("mixins/patches").join(&filename);
    let path = mod_dir.join(&relative); fs::write(&path, diff)?;
    append_mixin_to_manifest(&mod_dir.join(MANIFEST_NAME), patch_name, &relative)?;
    process::checked(Command::new("git").current_dir(&worktree).args(["add", "--all", "--", ".", ":(glob,exclude)**/Cargo.lock"]), "advance mixin patch baseline")?;
    log::info!("Generated mixin patch {}", path.display()); Ok(path)
}

pub fn save(config: &Config, layout: &Layout, name: &str, patch_name: &str) -> Result<PathBuf> {
    create_patch(config, layout, name, patch_name)
}

pub fn hash_mixins(mods: &ResolvedMods) -> Result<Vec<u8>> {
    let mut hasher = Sha256::new();
    for item in &mods.ordered {
        hasher.update(item.manifest.mod_info.id.as_bytes()); hasher.update([0]);
        for revision in item.manifest.pumpkin.supported_revisions() {
            hasher.update(revision.as_bytes());
            hasher.update([0]);
        }
        hasher.update(serde_json::to_vec(&item.manifest.mixins)?); hasher.update([0]);
        for mixin in &item.manifest.mixins { hasher.update(fs::read(item.directory.join(&mixin.patch))?); hasher.update([0]); }
    }
    Ok(hasher.finalize().to_vec())
}

fn find_package(worktree: &Path, package_name: &str) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for entry in WalkDir::new(worktree).max_depth(5) {
        let entry = entry?;
        if entry.file_name() != "Cargo.toml" || !entry.file_type().is_file() { continue; }
        let relative = entry.path().strip_prefix(worktree).unwrap_or(entry.path());
        if relative.components().any(|part| matches!(part.as_os_str().to_str(), Some("target" | ".git"))) { continue; }
        let document = fs::read_to_string(entry.path())?.parse::<toml_edit::DocumentMut>().map_err(|source| LoaderError::EditCargoManifest { path: entry.path().to_path_buf(), source })?;
        if document.get("package").and_then(toml_edit::Item::as_table_like).and_then(|table| table.get("name")).and_then(toml_edit::Item::as_str) == Some(package_name)
            && let Some(parent) = entry.path().parent() { candidates.push(parent.to_path_buf()); }
    }
    candidates.sort(); candidates.into_iter().next().ok_or_else(|| LoaderError::InvalidModManifest { path: worktree.join("Cargo.toml"), message: format!("could not locate target package `{package_name}`") })
}

fn append_mixin_to_manifest(path: &Path, id: &str, patch: &Path) -> Result<()> {
    let mut manifest: ModManifest = toml::from_str(&fs::read_to_string(path)?).map_err(|source| LoaderError::ParseModManifest { path: path.to_path_buf(), source })?;
    let id = slugify(id);
    if manifest.mixins.iter().any(|mixin| mixin.id == id) {
        return Err(LoaderError::InvalidModManifest { path: path.to_path_buf(), message: format!("mixin id `{id}` already exists") });
    }
    manifest.mixins.push(MixinDefinition {
        id, description: None, patch: patch.to_path_buf(), required: true,
        after: Vec::new(), before: Vec::new(), target: MixinTarget::default(),
        validation: MixinValidation::default(),
    });
    write_manifest(path, &manifest)
}

fn write_manifest(path: &Path, manifest: &ModManifest) -> Result<()> { fs::write(path, toml::to_string_pretty(manifest)?)?; Ok(()) }
fn load_manifest(directory: &Path) -> Result<ModManifest> {
    let path = directory.join(MANIFEST_NAME);
    toml::from_str(&fs::read_to_string(&path)?).map_err(|source| LoaderError::ParseModManifest { path, source })
}
fn resolve_mod(layout: &Layout, name: &str) -> Result<PathBuf> {
    for candidate in [layout.mods.join(name), layout.mods.join(slugify(name))] { if candidate.join(MANIFEST_NAME).is_file() { return Ok(candidate); } }
    Err(LoaderError::ModNotFound { name: name.into() })
}
fn ensure_revision(manifest: &ModManifest, resolved: &str, config: &Config) -> Result<()> {
    if !manifest.pumpkin.supports(resolved) {
        return Err(LoaderError::ModRevisionMismatch {
            mod_id: manifest.mod_info.id.clone(),
            declared: manifest.pumpkin.display_revisions(),
            configured: config.pumpkin.revision.clone(),
            resolved: resolved.into(),
        });
    }
    Ok(())
}
fn next_patch_number(directory: &Path) -> Result<u32> {
    let mut highest = 0;
    for entry in fs::read_dir(directory)? { if let Some(prefix) = entry?.file_name().to_string_lossy().split('-').next() { if let Ok(value) = prefix.parse() { highest = highest.max(value); } } }
    Ok(highest + 1)
}
fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.contains('/') || name.contains('\\') || name == "." || name == ".." { return Err(LoaderError::InvalidModName { name: name.into() }); }
    Ok(())
}
fn slugify(value: &str) -> String {
    value.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '-' }).collect::<String>().split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-")
}
fn toml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "\\\\")
}

fn render_template(template: &str, replacements: &[(&str, &str)]) -> String {
    replacements.iter().fold(template.to_owned(), |rendered, (placeholder, value)| rendered.replace(placeholder, value))
}
