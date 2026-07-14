# Self-hosting contract

Nova is not complete until it can develop Nova from Nova.

The self-hosting acceptance test is performed on a machine containing only Nova:

1. Clone or open the Nova source tree on a persistent Nova filesystem.
2. Edit Rust source with a native Nova editor.
3. Compile and test the kernel and user programs using a Nova-hosted Rust toolchain.
4. Build new BIOS/UEFI installation and recovery images.
5. Boot the new image in Nova's native VM service or a second supported machine.
6. Apply a signed system update and roll back after an intentionally bad build.

Current status: none of these six end-to-end steps is complete yet. The present
kernel provides boot, framebuffer UI, PS/2 input, RAMFS and deterministic tools.
Persistent storage, processes and the native build service are the next blocking
layers. Host-side QEMU and the Rust image builder remain development scaffolding.
