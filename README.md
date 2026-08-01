# Pumpkin Loader

A base Rust application for reproducibly fetching, compiling, caching, and running a pinned Pumpkin Minecraft server source revision.

Pumpkin's official source build is a normal Cargo release build. This loader wraps that process in an immutable Git source cache, a disposable worktree, isolated Cargo directories, a content-addressed finished-build cache, and a dedicated server-data directory.

## Requirements

- Rust and Cargo
- Git
- Linux `bubblewrap` (`bwrap`) is recommended for actual filesystem/process sandboxing

Without `bwrap`, the loader still separates source, build cache, Cargo cache, and server data, but this is **not a security boundary** against untrusted source mods or `build.rs` scripts.

## Start

```bash
cargo build --release
./target/release/pumpkin-source-loader init --force
```

Edit `pumpkin-loader.toml` and replace `master` with the exact Pumpkin Git commit you want to target.

```bash
./target/release/pumpkin-source-loader fetch
./target/release/pumpkin-source-loader build
./target/release/pumpkin-source-loader run
```

Arguments after `--` are forwarded to Pumpkin:

```bash
./target/release/pumpkin-source-loader run -- --help
```

## Commands

- `init`: create a starter configuration
- `fetch`: update the bare source cache and resolve the configured revision
- `build`: create a disposable worktree and compile it
- `run`: reuse the cached binary or build it, then start the server
- `status`: print revision, paths, resolved commit, and build key
- `clean`: remove disposable worktrees

## Directory layout

```text
.pumpkin-loader/
├── sources/pumpkin.git/       immutable bare Git cache
├── worktrees/<build-key>/     disposable patched source tree
├── cargo-home/                isolated registry/Git dependency cache
├── cargo-target/              persistent incremental compilation cache
└── builds/<build-key>/
    ├── pumpkin                verified finished binary
    └── manifest.json          inputs, toolchain versions, binary hash
server/                        Pumpkin's runtime working directory
```

## Where mod patching goes

`ensure_build` creates a clean worktree and then calls `build_pumpkin`. Apply mod overlays and ordered patches at the marked location between those two operations. Once mods are added, include their ordered content hashes in `calculate_build_key`.

Recommended next modules:

```text
src/mods/manifest.rs       parse mod metadata
src/mods/resolver.rs       dependency graph and deterministic ordering
src/mods/apply.rs          file overlays and git patches
src/mods/hash.rs           hash complete resolved mod inputs
```

## Isolation model

### Build

With `bubblewrap`, the build sees system tool directories read-only and receives writable access only to:

- the disposable source worktree
- the loader Cargo home
- the loader Cargo target directory

Network is currently retained because Cargo may need to fetch dependencies. A stronger two-stage implementation should run `cargo fetch` with network access and then build with `--offline` plus a network namespace.

### Runtime

With `bubblewrap`, the server receives:

- its binary as a read-only bind
- its server-data directory as a writable bind
- system libraries and certificates read-only
- a private process/session namespace

Network remains shared because a Minecraft server must accept connections.
