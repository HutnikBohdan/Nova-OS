# Research record (2026-07-14)

## What is technically viable

Rust officially supports `no_std`/`core` for bare-metal code. The rust-osdev
bootloader produces BIOS and UEFI disk images and passes framebuffer and memory
map information to an `x86_64-unknown-none` kernel. UEFI owns boot resources only
until `ExitBootServices`; after that point the loader/kernel owns continued
execution.

A useful virtual-hardware target exists today: `virtio-drivers` is `no_std` Rust
and supports block, network, GPU, input, console, sound, RNG and 9P devices. The
`smoltcp` project supplies a freestanding TCP/IP stack. This makes QEMU/VirtIO the
fastest honest route to files, network, graphics, input, and later real-hardware
drivers without inheriting another operating system.

Rust-native model inference exists. Candle offers CPU and optional GPU backends;
mistral.rs offers quantized GGUF and tool-calling support. Neither is ready to
drop into a new `no_std` OS: both assume mature threads, files, memory mapping,
networking, and platform acceleration. We therefore need a native user-space
runtime/ABI first, then port a CPU-only subset or implement the required
quantized transformer path in Rust.

For the free default model, Qwen2.5-Coder-1.5B-Instruct is Apache-2.0, has 1.54B
parameters and a 32,768 token context. A 4-bit build is small enough for ordinary
PCs, though model quality will be below hosted frontier systems. The 3B variant
uses a more restrictive Qwen Research license, so it is not the default.

## What “AI controls the PC” means

The model is a planner, not a driver. A privileged `operator` service owns a
full-system capability token and exposes typed tools: inspect screen/tree,
keyboard/mouse injection, filesystem transactions, process control, networking,
package/build operations, and reboot/power actions. Tool results return to the
model until the task is complete. Every mutation is journaled, and destructive
steps can have checkpoints even when the user enables autonomous mode.

Putting inference or natural-language parsing in ring 0 would let a model fault
crash or corrupt the kernel. Keeping it in a process does not reduce its practical
authority; the capability service still lets it behave exactly like the user.

## Honest constraints

- A broadly usable desktop OS is a multi-year engineering program, not one chat turn.
- “Only Rust” can govern our source. CPU microcode, UEFI firmware, model weights,
  and vendor GPU firmware are data/external platform components, not Rust source.
- NVIDIA/AMD acceleration cannot honestly be promised early under a strict
  Rust-only rule. The first local model must use portable CPU kernels.
- Running arbitrary Windows/Linux binaries requires a compatibility subsystem;
  the first runnable programs will target Nova's own ABI.
- The image build is pinned to nightly 2026-04-20: it is new enough for Cargo's
  JSON-target gate and old enough to accept bootloader 0.11.15's `x86-softfloat`
  spelling, which Rust 1.98 removed.

## Primary sources

- Rust `no_std`: https://doc.rust-lang.org/stable/embedded-book/intro/no-std.html
- rust-osdev bootloader: https://github.com/rust-osdev/bootloader
- UEFI specification: https://uefi.org/specifications
- Rust x86-64 primitives: https://github.com/rust-osdev/x86_64
- VirtIO Rust drivers: https://github.com/rcore-os/virtio-drivers
- smoltcp: https://github.com/smoltcp-rs/smoltcp
- Redox OS (comparison/reference): https://github.com/redox-os/redox
- Candle: https://github.com/huggingface/candle
- mistral.rs: https://github.com/EricLBuehler/mistral.rs
- Qwen2.5-Coder 1.5B model card: https://huggingface.co/Qwen/Qwen2.5-Coder-1.5B-Instruct
- Wasmtime/WASI reference: https://github.com/bytecodealliance/wasmtime
