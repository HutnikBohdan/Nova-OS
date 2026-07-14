# Nova Developer Image

Nova's installable development edition is designed to rebuild Nova from inside
Nova without making the active system slot writable.

## Target layout

- `system-a` and `system-b`: verified read-only boot slots;
- `recovery`: minimal kernel, verifier, rollback UI, and image installer;
- `workspace`: writable source tree, local package cache, build output, and user
  projects;
- `state`: Guardian journal, boot-attempt counters, signed manifests, and audit
  records.

The active system slot is never modified in place. Nova Builder compiles in the
workspace, Nova Image Composer creates a candidate image in the inactive slot,
and Guardian verifies hashes, capabilities, ABI compatibility, and boot tests.
Firmware then attempts the candidate slot. A missing health confirmation rolls
back automatically; a successful confirmation promotes the candidate.

## Required self-hosting chain

1. A standalone `init` ELF and VFS/storage services run outside the kernel.
2. The Nova Rust SDK and the complete vendored dependency graph are available
   in the workspace image.
3. Builder runs as an unprivileged service and emits content-addressed output.
4. Image Composer owns only the inactive-slot capability.
5. Guardian records a before-state, build manifest, verification result, and
   rollback point before allowing a boot-slot change.
6. Recovery can select either known-good slot without AI, network, or the main
   desktop.

## Current status

The repository can now build BIOS and UEFI images fully offline from committed
Rust dependencies. The kernel boots real ring-3 tasks, preempts three processes,
reclaims exited address spaces, and defaults to a microkernel-only dependency
boundary. The `legacy-monolith-proofs` feature still provides the existing
desktop/Guardian/AI smoke environment for regression testing.

Nova does **not yet** provide the complete standalone VFS/init/SDK/Image Composer
chain above, so current images are development proofs rather than a supported
self-hosting installer. A release may be called `Developer Image` only after an
installed VM rebuilds a changed service, writes an inactive-slot image, boots
it, confirms health, and demonstrates automatic rollback from an injected bad
image.
