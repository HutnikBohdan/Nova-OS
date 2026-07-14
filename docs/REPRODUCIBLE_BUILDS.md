# Reproducible build inputs

Nova separates versioned source inputs from generated output and downloaded
developer tools. A Git clone is the source of truth, but it is not by itself a
complete offline build appliance.

## Versioned source inputs

The following files belong in Git and define the build graph:

- Rust sources, assets, workspace manifests and `build.rs` files;
- the root `Cargo.lock`, which pins the complete crates.io dependency graph;
- the committed `vendor/` tree, which supplies that locked graph offline;
- `third-party/bootloader`, Nova's minimal audited 0.11.15 fork for offline
  stage resolution and deterministic sequential nested builds;
- `rust-toolchain.toml` and `.cargo/config.toml`;
- `build-support/nova-build-lock.psd1`, which records exact tool identities,
  boot-critical crate checksums, and known QEMU/firmware hashes;
- `scripts/Verify-NovaBuildEnvironment.ps1`.

Run the read-only verification from PowerShell 5.1 or newer:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\Verify-NovaBuildEnvironment.ps1
```

Use `-Mode Bootstrap` only when network access and user-level rustup changes are
intended. It installs the pinned Rust toolchain/components/targets and performs
`cargo fetch --locked`; it does not download QEMU or firmware.

## Generated and external state

These paths do not belong in the source commit:

| Path | Meaning |
| --- | --- |
| `target/` | Rust build cache, generated boot images, and downloaded OVMF cache |
| `dist/` | release images and executable artifacts produced from a commit |
| `tools/` | downloaded third-party QEMU/GitHub binaries |
| `vm-data/` | mutable VM disks and logs |

Verified release images are published separately and must be tied to the exact
source commit. Their presence never replaces the source and lock files.

## Committed offline dependency mirror

`vendor/` is an intentional, versioned source dependency of Nova. It contains
the crates selected by the root `Cargo.lock`; Cargo validates each package
against its `.cargo-checksum.json`. Changes to `Cargo.lock` and `vendor/` must
be reviewed together.

The mirror includes the root graph, the five hidden bootloader stage graphs,
and the pinned nightly `build-std` graph. A refresh must synchronize all three;
plain `cargo vendor` alone is insufficient because upstream bootloader uses
independent `cargo install --locked` calls.

Start a refresh on a connected maintenance machine with the pinned toolchain:

```powershell
cargo fetch --locked
cargo vendor --locked --versioned-dirs vendor --sync build-support\bootloader-stage-deps\Cargo.toml
```

Archive these items together:

- the clean Git checkout, including the root `Cargo.lock`;
- the committed `vendor/` tree;
- the rustup toolchain or an independently provisioned identical toolchain;
- the external QEMU bundle matching `nova-build-lock.psd1`, if VM gates are
  required.

Verify the prepared checkout without network access:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\Verify-NovaBuildEnvironment.ps1 -Offline -RequireQemu
powershell -ExecutionPolicy Bypass -File scripts\Verify-NovaBuildEnvironment.ps1 -Build
```

The `-Build` mode creates a unique ignored `.nova-cargo-home/` child and writes
a Cargo config containing the absolute normalized path to `vendor/` plus
`[net] offline = true`. It exports that directory as `CARGO_HOME` for the root
build, so nested Cargo processes launched by `bootloader` inherit the same
offline source policy. A project-local config alone is not enough for nested
builds whose working directory is outside the checkout.

Nova's bootloader fork resolves stage packages from `vendor/`, builds them
sequentially to avoid Windows Cargo-lock deadlocks, and keeps their temporary
targets below the parent build output. A clean offline release build and both
BIOS/UEFI QEMU boot gates passed with this path.

On managed Windows systems, Application Control may reject freshly generated
Cargo build-script executables with OS error 4551. That is an execution-policy
failure, not permission to weaken the offline or checksum checks. The verifier
reports the build as failed; use an approved build execution policy/location
and rerun the same command before claiming clone-to-build reproduction.

## Current firmware limitation

The checked-in dependency version for `ovmf-prebuilt` is pinned by `Cargo.lock`,
but the current UEFI runner asks that crate for `Source::LATEST`. The downloaded
payload lives below `target/ovmf`, is generated/external state, and has no pinned
payload digest in the source tree. Consequently, `cargo run -- uefi` is not yet
bit-for-bit reproducible even when Rust and Cargo dependencies match.

The QEMU bundle currently present on the development machine has known hashes
for its executable, SeaBIOS images, and selected EDK2 files; those values are in
the build lock. A clean clone is expected not to contain `tools/qemu`. Runtime
verification must either provision that exact external bundle or update the
manifest through a reviewed toolchain change.
