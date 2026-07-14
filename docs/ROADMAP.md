# Roadmap

## Перевірений зріз 2026-07-14

- Повний контекст x86-64 (усі GPR, IRET frame і CR3) зберігається та відновлюється під час таймерного перемикання; BIOS QEMU підтвердив `NOVA_FULL_CONTEXT_SWITCH_OK` і `NOVA_PREEMPTIVE_CR3_SWITCH_OK`.
- `runtime-core` містить bounded IPC, життєвий цикл процесів, очікування після crash, scheduler quantum і capability model.
- `hardware-core` містить власні no_std протоколи NVMe, AHCI, USB HID, ACPI та HDA; 24 модульні тести, core завантажений у kernel image.
- `build-core` містить no_std manifest/DAG, job protocol, content-addressed cache keys, signed bundles та атомарне встановлення; 13 тестів, core завантажений у kernel image.
- Kernel має власний 4 MiB Rust heap із free-list, поверненням і злиттям блоків; allocation/deallocation перевірені в booted VFS path через `NOVA_KERNEL_HEAP_OK`.
- Ring-3 ABI виконує реальні `ChannelSend`/`ChannelReceive`; окрема програма передала й отримала `ping` (`NOVA_RING3_IPC_ROUNDTRIP_OK`).
- Повний offline-набір інтегрованих core-crates: 355 тестів без помилок (image-builder перевіряється окремим image/boot gate).
- Власний `compiler-core` компілює строгий NovaRust subset у x86-64 ELF; згенерована в самій завантаженій Nova програма виконана в ізольованому ring 3 і повернула 42 (`NOVA_SELF_HOSTED_RING3_APP_OK`).
- Власні SHA-512, Ed25519, strict DER/X.509 trust core і VirtIO block/net/input protocol core інтегровані в kernel boot gates.
- Nova Guardian виконує durable capability-транзакції: append-only hash-chain/CRC journal, inverse snapshot, reboot recovery і fault-injected power-loss rollback на реальному VirtIO block path.
- BIOS і UEFI запускають однаковий актуальний kernel image з VFS, TLS 1.3, NovaFS v2, deployment, ELF loader, compositor, media, socket, USB та accessibility core gates.
- Інтерактивний QEMU gate відкрив Nova AI, виконав `write /ai.txt hello` з capability policy, перевірив результат і видав `NOVA_AI_CAPABILITY_TASK_OK`.
- AI/operator self-hosted build із VFS вже пройшов реальний Guardian capability/journal шлях і виконав згенерований ELF у новому ring-3 CR3 (`NOVA_AI_SELF_HOSTED_BUILD_OK`).
- Наступний gate: замінити решту bounded proofs довгоживучими службами, підключити реальні IRQ/DMA шляхи нових драйверів та довести editor/package SDK.

## M0 — bootable foundation (complete)

- BIOS/UEFI boot image, framebuffer console, serial diagnostics
- PS/2 polling keyboard, fixed-capacity RAM filesystem
- tested intent parser and privileged deterministic operator router

## M0.5 — Nova Desktop

- adaptive 1280x720 aurora desktop with layered application windows
- PS/2 mouse packets, software cursor and click hit-testing
- floating taskbar, Файли, Термінал, Nova AI, Центр керування, Редактор і Програми
- F1-F8/Esc navigation plus shifted US QWERTY input
- Inter Display Latin/Cyrillic glyph atlas baked into the standalone kernel image
- fully Ukrainian visible shell strings (English fallback commands remain accepted)
- Nova Browser shell plus tested no_std URL/HTTP/HTML/DOM/CSS/layout core
- real filesystem and operator commands inside the graphical shell
- framebuffer, keyboard-command and mouse-click smoke tests in QEMU

## M1 — real kernel (in progress)

- tested allocation-free physical-frame/address-space primitives
- tested capability/process/thread/scheduler/syscall/ELF64 core
- separate CR3 page tables, DPL3 syscall, two ring-3 processes and deliberate-crash isolation verified in QEMU
- PIT/PIC/IDT timer interrupts and preemptive switching between two active CR3 address spaces verified in QEMU
- generalize the verified complete-register-context switch into production run queues with blocking IPC
- init process and crash isolation

## M2 — usable virtual PC (core modules in progress)

- tested own PCI discovery, BAR, VirtIO negotiation/queue primitives
- tested own persistent on-disk superblock, checksum journal and crash replay
- BIOS legacy-IDE ATA PIO path persists A/B-checksummed NovaFS snapshots across reboot
- own e1000 MMIO driver performs DMA TX/RX; DHCP bind and ARP verified in QEMU
- tested Ethernet/ARP/IPv4/UDP/DHCP/DNS/TCP/retransmit primitives; finish DNS/TCP runtime gates
- extend the verified booted legacy VirtIO block DMA path to modern VirtIO, network/input and interrupts
- persistent VFS, isolated compositor, movable windows and native editor process

## M3 — programs that really run

- native Rust standard-library target for Nova
- Rust-written build/package daemon and signed application bundles
- WASM component runtime as a second application ABI
- self-hosted editing/build/test loop
- native Nova Browser process: HTTP/TLS, DOM/CSS layout and sandboxed scripting

## M4 — AI owns the workflow

- quantized CPU tensor kernels, tokenizer and Qwen model loader in Rust
- tool schemas for files, processes, UI, network, build and system settings
- screen/accessibility observation, mouse/keyboard injection
- durable task memory, journal, checkpoints, verification and recovery

## M5 — hardware expansion

- NVMe/AHCI, USB HID, common Ethernet and audio devices
- ACPI power/battery, SMP, power management
- selected real PCs with explicit hardware support matrices
