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

The Cargo build output is `target/release/nova-vm-lab.exe`. A single EXE is not
the distributable product because QEMU, firmware and Nova images are runtime
resources. Assemble the release directory using this deterministic layout:

```text
Nova-VM-Lab.exe
SHA256SUMS.txt
qemu/qemu-system-x86_64.exe
qemu/qemu-img.exe
qemu/share/...
images/nova-os-bios.img
images/nova-os-uefi.img
```

Resources are resolved relative to `Nova-VM-Lab.exe`, so the whole directory
can be moved to another machine. An explicit resource root can be selected in
the UI or set with `NOVA_VM_LAB_HOME`; the UI setting has priority. The package
publisher must include a sorted SHA-256 manifest for all bundle files. The UI
provides a Rust-native bundle check for QEMU, firmware and both Nova images. VM
profiles, disks, writable firmware state and logs are stored in
`%LOCALAPPDATA%/NovaVmLab` so the installed bundle remains immutable.

To install a conventional OS: select its ISO, create a QCOW2 disk, keep
"boot from installation media" enabled and start. After the installer finishes,
disable that option and restart the VM.

For debugging a custom OS, configure its COM1 serial output. Nova OS already
does this, so its boot markers and shell output appear in the **Serial log** tab.
Keep snapshot mode enabled while testing risky changes; disable it when you want
the guest's disk writes to persist.
