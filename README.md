# Pumpkin Loader

Pumpkin Loader lets you run a modded Pumpkin server.

It is intended for two audiences:

- **Server administrators** use it to select a Pumpkin revision, install mod packages, and run it.
- **Mod developers** use it to create native Pumpkin plugins, with source patches.

A loader mod package contains a native Pumpkin plugin and may also contain mixins. Prefer the normal Pumpkin plugin API whenever possible, mixins should only be used for functionality that Pumpkin does not expose to plugins.

## Requirements

- Git
- A Rust toolchain capable of building the selected Pumpkin revision
- Linux, macOS, or Windows
- Bubblewrap on Linux when sandboxing is enabled or required

The loader uses the Rust toolchain selected by the pinned Pumpkin source tree and builds plugins against that same source revision.

## Server admin guide

### 1. Create a project

Create a directory for the server and initialize its configuration:

```bash
mkdir my-pumpkin-server
cd my-pumpkin-server
pumpkin-loader init
```

This creates `pumpkin-loader.toml`.

For a reproducible server or modpack, replace the default branch name with a full Pumpkin commit hash:

```toml
[pumpkin]
repository = "https://github.com/Pumpkin-MC/Pumpkin.git"
revision = "75994f0123456789abcdef0123456789abcdef01"
```

A branch or tag is convenient during experimentation, but it can move over time.

### 2. Install mods

Place each mod package under `mods/`:

```text
my-pumpkin-server/
├── pumpkin-loader.toml
└── mods/
    ├── example-mod
    └── another-mod
```

Every package must support the Pumpkin commit selected in `pumpkin-loader.toml`. Required mod dependencies are selected automatically. Missing dependencies, incompatible versions, conflicts, or unsupported Pumpkin revisions cause an error.

### 3. Run the server

```bash
pumpkin-loader run
```

Arguments after `--` are forwarded to Pumpkin:

```bash
pumpkin-loader run -- --help
```

`run` ensures the required build and plugin deployment are up to date before starting Pumpkin.

### Server files

By default, the directory containing `pumpkin-loader.toml` is also the Pumpkin server-data directory. This means files such as `plugins/`, worlds, configuration, logs, and other runtime data are stored in the project directory.

To keep runtime data elsewhere, configure `paths.server_data`:

```toml
[paths]
state = ".pumpkin-loader"
server_data = "server"
```

Relative paths are resolved from the directory containing `pumpkin-loader.toml`.

### Checking the current setup

```bash
pumpkin-loader status
```

This shows the configured repository and revision, loader state directory, server-data directory, resolved commit, and current build key.

### Cleaning loader source state

```bash
pumpkin-loader clean
```

This removes loader-managed Pumpkin worktrees. It does not remove the mod source directories or the server-data directory.

## Configuration reference

A newly generated configuration is equivalent to:

```toml
[pumpkin]
repository = "https://github.com/Pumpkin-MC/Pumpkin.git"
revision = "master"

[paths]
state = ".pumpkin-loader"
# server_data = "server"

[build]
profile = "moddev"
binary = "pumpkin"
cargo_features = []
no_default_features = false
locked = true
# jobs = 8
# rustflags = ""
sandbox = "auto"

[build.environment]

[runtime]
sandbox = "auto"

[runtime.environment]
```

### Build settings

| Field | Meaning |
|---|---|
| `profile` | Cargo profile used to build Pumpkin and the plugins. |
| `binary` | Pumpkin binary target name. |
| `cargo_features` | Cargo features enabled while building Pumpkin. |
| `no_default_features` | Disables Pumpkin's default Cargo features when `true`. |
| `locked` | Uses Cargo's locked dependency resolution when `true`. |
| `jobs` | Optional Cargo job count. |
| `rustflags` | Optional `RUSTFLAGS` value for the build. |
| `environment` | Additional environment variables passed to builds. |
| `sandbox` | Build sandbox policy. |

The default `moddev` profile favors development build speed. A release profile is more appropriate for production servers.

### Runtime settings

`runtime.environment` adds environment variables when starting Pumpkin. `runtime.sandbox` controls runtime sandboxing independently from build sandboxing.

Supported sandbox values are:

```toml
sandbox = "auto"     # use the sandbox when available
sandbox = "off"      # do not use the sandbox
sandbox = "require"  # fail if the sandbox is unavailable
```

## Mod developer guide

### Create a mod package

```bash
pumpkin-loader mod init example-mod
```

This fetches the configured Pumpkin revision and creates:

```text
mods/example-mod/
├── pumpkin-mod.toml
├── Cargo.toml
├── src/
│   └── lib.rs
└── mixins/
```

The generated crate is a native Pumpkin plugin with local dependencies pointing at the selected Pumpkin source tree.

Use `--force` to replace an existing directory:

```bash
pumpkin-loader mod init example-mod --force
```

### Mod manifest

Each package is described by `pumpkin-mod.toml`:

```toml
format = 1
mixins = []

[mod]
id = "example-mod"
name = "Example Mod"
version = "0.1.0"
authors = []

[pumpkin]
revisions = ["f0c7332f0512ff6b9da283959e9d877a4177e0cc"]

[plugin]

[dependencies]
required = []
conflicts = []
load_after = []
load_before = []
```

#### `[mod]`

| Field | Required | Meaning |
|---|---:|---|
| `id` | yes | Unique package identifier. |
| `name` | yes | Human-readable package name. |
| `version` | yes | Semantic version. |
| `description` | no | Package description. |
| `authors` | no | Author names. |
| `license` | no | SPDX license identifier or other license description. |

#### `[pumpkin]`

`revisions` lists the Pumpkin commits supported by the package. Full hashes and unambiguous commit prefixes are accepted.

```toml
[pumpkin]
revisions = [
    "75994f0123456789abcdef0123456789abcdef01",
    "a13c82b4",
]
```

#### `[plugin]`

| Field | Required | Meaning |
|---|---:|---|
| `path` | no | Plugin crate directory relative to the package. Defaults to the package root. |
| `package` | no | Cargo package or target selector when the manifest contains multiple packages. |
| `artifact` | no | Explicit dynamic-library path relative to the Cargo target profile directory. |

Normally an empty `[plugin]` table is sufficient. The loader obtains the produced `cdylib` or `dylib` path from Cargo's build output.

The deployed library is named:

```text
pumpkin-loader-<mod-id>.<so|dll|dylib>
```

#### `[dependencies]`

Required dependencies may include a semantic-version requirement:

```toml
[dependencies]
required = [
    { id = "payload-api", version = "^1.2" },
]
```

Conflicting packages can be rejected explicitly:

```toml
[dependencies]
conflicts = [
    { id = "other-payload-bridge", version = "*" },
]
```

`load_after` and `load_before` control deterministic package ordering:

```toml
[dependencies]
load_after = ["payload-api"]
load_before = ["integration-layer"]
```

### Developing a mixin

First prepare the selected mod and its required dependencies:

```bash
pumpkin-loader mod dev example-mod
```

The command prints the prepared Pumpkin worktree path. Edit Pumpkin in that worktree, then save the current source changes as a patch:

```bash
pumpkin-loader mod patch example-mod \
  --patch-name custom-payload-registration
```

The patch is written under the mod package's `mixins/` directory and a corresponding `[[mixins]]` entry is added to `pumpkin-mod.toml`.

The generated entry intentionally needs review. Declare its target files and add validation markers before distributing the package.

### Mixin manifest entries

Example:

```toml
[[mixins]]
id = "custom-payload-registration"
description = "Adds the hook required by the plugin"
patch = "mixins/001-custom-payload-registration.patch"
required = true
after = []
before = []

[mixins.target]
package = "pumpkin"
files = ["src/net/java/mod.rs"]

[mixins.validation]
contains = ["handle_login_sequence"]
not_contains = ["EXAMPLE_PAYLOAD_HOOK"]
```

Mixin IDs are local to their package. References within the same package may use the unqualified ID:

```toml
after = ["payload-registry"]
```

Cross-package references use `<mod-id>:<mixin-id>`:

```toml
after = ["payload-api:payload-registry"]
```

The loader rejects unknown references and ordering cycles.

`mixins.target.package` selects a Cargo package in the Pumpkin workspace. Target file paths are relative to that package. Without `package`, paths are relative to the Pumpkin worktree root.

Validation is performed before applying the patch:

- Every `contains` value must appear in at least one declared target file.
- Every `not_contains` value must be absent from all declared target files.
- The patch may only modify declared target files.

A required mixin aborts the build when validation fails. An optional mixin is skipped with a warning:

```toml
required = false
```

Keep mixins small and use them only to expose hooks or behavior that cannot reasonably be implemented through Pumpkin's plugin API.

## Command reference

```text
pumpkin-loader init [--force]
pumpkin-loader fetch
pumpkin-loader build [--force]
pumpkin-loader run [-- <pumpkin arguments>]
pumpkin-loader status
pumpkin-loader clean

pumpkin-loader mod init <name> [--force]
pumpkin-loader mod dev <name>
pumpkin-loader mod patch <name> [--patch-name <name>]
```

Global options:

```text
--config <path>   Configuration file; defaults to pumpkin-loader.toml
-v, --verbose     Increase diagnostic output; repeat for more detail
-q, --quiet       Suppress informational output
```

## Important limitations

- Plugins are native Rust dynamic libraries and must be built for the same Pumpkin source revision and a compatible Rust toolchain.
- Generated plugin dependencies point into the loader's state directory. Moving or deleting that directory can invalidate those local paths; rerun `pumpkin-loader mod init` or update the paths when relocating a development project.
- Mixins are source-revision-sensitive. List every tested Pumpkin revision in the mod manifest and keep validation rules specific enough to detect incompatible source changes.
- Pumpkin Loader does not replace Pumpkin's plugin system. It builds and deploys native plugins that Pumpkin loads normally.
