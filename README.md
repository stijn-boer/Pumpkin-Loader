# Pumpkin Loader

Pumpkin Loader builds a pinned [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin) revision with mixins, builds normal Pumpkin plugins against that same local source tree, deploys those plugins into the server directory, and starts the resulting server.

The loader deliberately separates the two extension mechanisms:

- **Pumpkin plugins** provide normal runtime behavior, lifecycle handling, events, commands, and access to Pumpkin's plugin API.
- **Mixins** are ordered Git patches for changes that Pumpkin's plugin API does not expose.

Changing only plugin code does not invalidate the Pumpkin binary. Pumpkin is rebuilt only when its revision, build configuration, or enabled mixins change.

## Architecture

```text
Pumpkin Loader
├── resolves mod dependencies
├── fetches and pins a Pumpkin commit
├── validates and applies ordered mixins
├── builds the patched Pumpkin binary
├── builds each native Pumpkin plugin
└── runs Pumpkin`
```

A loader-managed **mod package** contains one native Pumpkin plugin and zero or more mixins.

## Requirements

- Git
- A Rust toolchain capable of building the selected Pumpkin revision
- Linux, macOS, or Windows dynamic-library support
- Bubblewrap on Linux when build or runtime sandboxing is enabled or required

Pumpkin Loader reads the Rust toolchain from the pinned Pumpkin worktree. Plugins are built with that same toolchain so their Rust ABI matches the server.

## Getting started

Create a configuration file:

```bash
pumpkin-loader init
```

Fetch and resolve the configured Pumpkin revision:

```bash
pumpkin-loader fetch
```

Build Pumpkin and all discovered plugins:

```bash
pumpkin-loader build
```

Run the server:

```bash
pumpkin-loader run
```

Arguments after `--` are forwarded to Pumpkin:

```bash
pumpkin-loader run -- --help
```

## Configuration

The default `pumpkin-loader.toml` is equivalent to:

```toml
[pumpkin]
repository = "https://github.com/Pumpkin-MC/Pumpkin.git"
revision = "master"

[paths]
state = ".pumpkin-loader"

[build]
profile = "moddev"
binary = "pumpkin"
cargo_features = []
no_default_features = false
locked = true
sandbox = "auto"

[build.environment]

[runtime]
sandbox = "auto"

[runtime.environment]
```

### Pumpkin revision

For local experimentation, `pumpkin.revision` may be a branch or tag. For reproducible modpacks, pin it to a full commit hash:

```toml
[pumpkin]
repository = "https://github.com/Pumpkin-MC/Pumpkin.git"
revision = "75994f0123456789abcdef0123456789abcdef01"
```

Every discovered mod package must declare that same resolved commit in its own manifest.

### Build profile

The default `moddev` profile is injected into Pumpkin's workspace manifest:

```toml
[profile.moddev]
inherits = "dev"
opt-level = 0
debug = 0
incremental = true
codegen-units = 256
panic = "abort"
```

This profile favors iteration speed over runtime performance. Use a release-oriented profile for production deployments.

### Sandboxing

Supported values are:

```toml
sandbox = "auto"     # use the sandbox when available
sandbox = "off"      # never sandbox
sandbox = "require"  # fail when sandboxing is unavailable
```

Build and runtime sandbox settings are independent.

## Creating a mod package

```bash
pumpkin-loader mod init jei
```

This creates:

```text
mods/jei/
├── pumpkin-mod.toml
├── plugin/
│   ├── Cargo.toml
│   └── src/lib.rs
└── mixins/
```

The generated plugin follows Pumpkin's native plugin API

## Mod manifest

Each mod package is described by `pumpkin-mod.toml`.

```toml
format = 1
mixins = []

[mod]
id = "jei"
name = "jei"
version = "0.1.0"
authors = []

[pumpkin]
revision = "f0c7332f0512ff6b9da283959e9d877a4177e0cc"

[plugin]
path = "plugin"

[dependencies]
required = []
conflicts = []
load_after = []
load_before = []

```

### `[mod]`

| Field | Required | Meaning |
|---|---:|---|
| `id` | yes | Unique package identifier. |
| `name` | yes | Human-readable name. |
| `version` | yes | Semantic version. |
| `description` | no | Package description. |
| `authors` | no | Author names. |
| `license` | no | SPDX-style license identifier or description. |

### `[pumpkin]`

`revision` must equal the resolved commit selected by the loader configuration. This prevents a mixin or plugin from silently being used against an untested Pumpkin source revision.

### `[plugin]`

| Field | Required | Meaning |
|---|---:|---|
| `path` | yes | Plugin crate directory relative to the mod package. |
| `package` | no | Cargo package/target selector when the manifest builds multiple packages. |
| `artifact` | no | Explicit dynamic-library path relative to the plugin target profile directory. |

Without `artifact`, the loader reads Cargo JSON output and selects the reported `cdylib` or `dylib` artifact.

The deployed filename is normalized to:

```text
server/plugins/pumpkin-loader-<mod-id>.<so|dll|dylib>
```

### `[dependencies]`

Required dependencies select additional mod packages transitively and may include a semantic version requirement:

```toml
required = [
    { id = "payload-api", version = "^1.2" },
]
```

Conflicts reject incompatible combinations:

```toml
conflicts = [
    { id = "other-jei-bridge", version = "*" },
]
```

`load_after` and `load_before` affect deterministic mod-package ordering. This order is also used as a stable tie-breaker for mixins.

## Mixins

A mixin is an ordered Git patch applied to the persistent Pumpkin worktree before Pumpkin is compiled.

Use a mixin only when the behavior cannot be implemented through Pumpkin's plugin API. Ordinary runtime logic belongs in the plugin.

### Ordering

Mixin IDs are globally qualified as:

```text
<mod-id>:<mixin-id>
```

References without a colon are local to the same mod package:

```toml
after = ["payload-registry"]
```

Cross-package ordering uses a qualified reference:

```toml
after = ["payload-api:payload-registry"]
```

The loader detects ordering cycles and aborts with the involved mixin IDs.

### Targets

`mixins.target.package` resolves a Cargo package inside the Pumpkin worktree. Target file paths are relative to that package directory.

```toml
[mixins.target]
package = "pumpkin"
files = ["src/net/java/mod.rs"]
```

When no package is specified, paths are relative to the worktree root.

The loader checks that every declared target file exists and that the patch only modifies declared targets.

### Compatibility

Restrict a mixin to explicitly tested Pumpkin commits:

```toml
[mixins.compatibility]
revisions = [
    "75994f0123456789abcdef0123456789abcdef01",
]
```

An empty list accepts the mod package's declared Pumpkin revision.

### Validation

Validation checks source text before applying the patch:

```toml
[mixins.validation]
contains = ["handle_login_sequence"]
not_contains = ["JEI_PAYLOAD_HOOK"]
```

- Every `contains` string must occur in at least one declared target file.
- Every `not_contains` string must be absent from all declared target files.

A failed required mixin aborts preparation. A failed optional mixin logs a warning and is skipped:

```toml
required = false
```

## Development workflow

Prepare the selected mod package and its required dependencies:

```bash
pumpkin-loader mod dev jei
```

This resets the persistent worktree, resolves the selected package's mixins, validates them, and applies them in dependency order. The command prints the worktree path.

Edit plugin code directly under:

```text
mods/jei/plugin/
```

Edit Pumpkin source in the prepared worktree when developing a mixin. Save the current worktree diff as a new mixin:

```bash
pumpkin-loader mod patch jei \
  --patch-name custom-payload-registration
```

`mod save` is an alias of `mod patch`:

```bash
pumpkin-loader mod save jei \
  --patch-name custom-payload-registration
```

The loader writes:

```text
mods/jei/mixins/NNN-custom-payload-registration.patch
```

and appends a new `[[mixins]]` entry to the manifest. Newly generated entries intentionally have empty target and validation metadata; fill those fields in before distributing the mod package.

`mod sync` remains as a compatibility command and prints the plugin source directory:

```bash
pumpkin-loader mod sync jei
```

## Build and deployment flow

`pumpkin-loader build` performs the following steps:

1. Fetch and resolve the configured Pumpkin revision.
2. Discover all mod packages under `mods/`.
3. Resolve required dependencies, conflicts, and deterministic package ordering.
4. Resolve and validate mixin ordering.
5. Calculate the Pumpkin build key.
6. Reuse a cached Pumpkin binary when possible.
7. Otherwise reset the persistent worktree, apply mixins, inject the requested Cargo profile, and compile Pumpkin.
8. Build every plugin independently using the pinned Pumpkin toolchain.
9. Copy plugin dynamic libraries into `server/plugins/`.

`pumpkin-loader run` first ensures the same build and deployment state, then starts Pumpkin with `server/` as its runtime data directory.

## Commands

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
--config <path>   Configuration file, default: pumpkin-loader.toml
-v, --verbose     Increase diagnostic output; repeat for more detail
-q, --quiet       Suppress informational output
```

### `status`

Shows the configured repository and revision, state paths, resolved commit, and current Pumpkin build key.

### `clean`

Removes loader-managed source/worktree state according to the source-management implementation. Server data and user mod sources should be treated as persistent project data.


## Design constraints

- Plugins are native Rust dynamic libraries and must be built with a toolchain compatible with Pumpkin.
- Local path dependencies generated by `mod init` point to the commit-specific persistent worktree. Moving or deleting the loader state directory invalidates those paths.
- Mixins are source-revision-sensitive. Use explicit compatibility revisions and validation markers.
- A change inside Pumpkin's monolithic main crate can still be expensive to compile. Keep behavior in plugins wherever possible and limit mixins to small hooks.
- The loader does not replace Pumpkin's plugin system. It prepares, builds, and deploys plugins that Pumpkin itself loads and unloads.
