# Architecture

```text
UEFI/BIOS -> Rust bootloader -> Nova microkernel
                               | memory, IRQ, IPC, scheduling
                               +-> driver services (VirtIO first)
                               +-> VFS + process manager + network
                               +-> compositor/input service
                               +-> operator capability service
                                      ^ typed tools + event stream
                                      |
                               local AI process
                                      |
                               user request / autonomous loop
```

The kernel will contain only mechanisms: address spaces, threads, IPC, interrupt
routing, capability validation, and a minimal debug console. Filesystems,
drivers, UI, networking, the model, and generated programs belong in isolated
user processes.

The operator gets an optional root capability bundle. Unlike a traditional
shell, its actions are structured requests with bounded inputs and explicit
results. Unlike a chatbot, it observes system state after every action and can
continue until the requested outcome is verified.

## Standalone installation invariant

QEMU, Nova VM Lab, Windows and Codex are development tools only. A released Nova
installation must boot on an empty supported PC and provide its own compositor,
drivers, persistent storage, installer, recovery, editor, compiler/build service,
package manager, network stack, browser, local AI runtime and update mechanism.
No installed subsystem may call back to a host operating system.

Application path:

1. Native Rust ELF programs using the Nova syscall ABI.
2. A Rust-written package/build service and cross-compiler toolchain.
3. Optional WebAssembly components for portable sandboxed applications.
4. Compatibility layers only after the native platform is stable.
