# Nova OS repository and backup policy

The private GitHub repository is the durable source of truth for Nova-owned
source code, documentation, tests, and reproducible build metadata. Verified
development binaries are attached to GitHub pre-releases.

## Why the local folder is much larger

Most local disk usage is disposable or reproducible data:

- `target/` directories contain Rust compiler output and incremental caches;
- `tools/qemu/` and `tools/gh/` contain downloaded third-party programs;
- `dist/` contains generated Nova images, VM logs, screenshots, and test disks;
- `vm-data/` contains mutable virtual-machine state.

These paths are intentionally ignored by Git. Committing them would make every
clone enormous, duplicate third-party software, and make source history harder
to audit. Root `Cargo.lock` remains tracked to pin the Rust dependency graph.

## What is backed up

- Every verified source change is committed and pushed to GitHub.
- Substantial work remains on a `codex/<topic>` branch and draft pull request
  until its acceptance tests pass.
- Verified BIOS/UEFI disk images and Nova VM Lab executables are uploaded as
  GitHub pre-release assets. Release notes identify the source branch/commit and
  the runtime checks that actually passed.
- Authentication secrets, local caches, VM state, and downloaded tools are never
  published.

## Restore on another machine

```powershell
git clone https://github.com/HutnikBohdan/Nova-OS.git
cd Nova-OS
git switch master
cargo build --release --locked
```

To inspect or restore unfinished development, switch to its published branch or
checkout the required commit. Download prebuilt development images from the
matching private GitHub pre-release instead of rebuilding when exact artifacts
are required.
