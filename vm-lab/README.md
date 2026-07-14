# Nova VM Lab

Native Rust GUI for installing and debugging operating systems in QEMU. It can:

- boot ISO, IMG and raw disk images;
- create growable QCOW2 system disks;
- use BIOS or UEFI firmware;
- configure RAM, virtual CPUs and NAT networking;
- run changes in disposable snapshot mode;
- capture the guest serial port and QEMU error log;
- load the current Nova OS image with one click.

Build and run from the repository root:

```powershell
cargo run -p nova-vm-lab
cargo build --release -p nova-vm-lab
```

The ready-to-run copy is `dist/Nova-VM-Lab.exe`; the Cargo build output is
`target/release/nova-vm-lab.exe`. VM profiles, disks,
firmware state and logs are stored in `%LOCALAPPDATA%/NovaVmLab` so QEMU never
needs to open a mutable virtual disk through the Cyrillic OneDrive path.

To install a conventional OS: select its ISO, create a QCOW2 disk, keep
"boot from installation media" enabled and start. After the installer finishes,
disable that option and restart the VM.

For debugging a custom OS, configure its COM1 serial output. Nova OS already
does this, so its boot markers and shell output appear in the **Serial log** tab.
Keep snapshot mode enabled while testing risky changes; disable it when you want
the guest's disk writes to persist.
