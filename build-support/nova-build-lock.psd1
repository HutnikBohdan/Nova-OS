@{
    SchemaVersion = 1

    SourceInputs = @{
        CargoLock = @{
            Path   = 'Cargo.lock'
            Sha256 = '8ABD31203C431B91C9666A706E3CCC215B51AF7DA3149A08D30E38882D755661'
        }
        RustToolchain = @{
            Path   = 'rust-toolchain.toml'
            Sha256 = 'D8DBE82C6311E8FAECC8EB2D3207A8FB046ABE8D8B23225EA0B5606AC4E436A3'
        }
        CargoConfig = @{
            Path   = '.cargo/config.toml'
            Sha256 = 'F2D028D14B8A9DD8EA08D5559192FCF388FDC917D5B5A4AE102E5B687EACBEF4'
        }
        Vendor = @{
            Path     = 'vendor'
            Tracked  = $true
            LockedBy = 'Cargo.lock and per-package .cargo-checksum.json files'
        }
        BootloaderFork = @{
            Path    = 'third-party/bootloader'
            Version = '0.11.15-nova-offline.1'
            Purpose = 'Use committed stage crates by path and serialize nested Cargo builds'
        }
    }

    Rust = @{
        Toolchain   = 'nightly-2026-04-20-x86_64-pc-windows-msvc'
        Channel     = 'nightly-2026-04-20'
        Host        = 'x86_64-pc-windows-msvc'
        Rustc       = '1.97.0-nightly'
        RustcCommit = 'e22c616e4e87914135c1db261a03e0437255335e'
        Cargo       = '1.97.0-nightly'
        LLVM        = '22.1.2'
        Components  = @('llvm-tools', 'rust-src')
        Targets     = @('x86_64-unknown-none', 'x86_64-unknown-uefi')
    }

    # Cargo.lock is authoritative for the complete transitive graph. These
    # entries call out boot-critical crates for quick human review.
    BootCriticalCrates = @(
        @{ Name = 'bootloader';      Version = '0.11.15'; Checksum = 'b5fb69232f250c0d5b74eeb285702b4b0479f574980f45dc9bef97b2c65e2021' }
        @{ Name = 'bootloader_api';  Version = '0.11.15'; Checksum = '5c074a3d1075b26ed77e502d6af8ab1263abbdc3d026c166e46480f973287293' }
        @{ Name = 'fontdue';         Version = '0.9.3';   Checksum = '2e57e16b3fe8ff4364c0661fdaac543fb38b29ea9bc9c2f45612d90adf931d2b' }
        @{ Name = 'ovmf-prebuilt';   Version = '0.2.9';   Checksum = '8b2ed5d406bc56f93035547fe364feb90395682dd8cc9efea09e806f8f845a17' }
    )

    Qemu = @{
        Optional          = $true
        Version           = '11.0.50'
        VersionBanner     = 'QEMU emulator version 11.0.50 (v11.0.0-12631-g54e84cdc7a)'
        Executable        = 'tools/qemu/qemu-system-x86_64.exe'
        ExecutableSha256  = 'B396EB9B669F6282EC60F0D46E6ADDCA8C669992E67A34365BE44CD3CB97C9A7'
        VersionFile       = 'tools/qemu/VERSION'
        VersionFileSha256 = '8A0F3108A7D3F61A4EC02D3A00C67210D2730E567FCAFFD76C5EDEF68BEBCA88'
        Firmware = @(
            @{ Path = 'tools/qemu/share/bios.bin';               Sha256 = '3DFD946D0C03AB0E022F84F10C3EB5F1DD507761F73E7D8067511BA35A10F776' }
            @{ Path = 'tools/qemu/share/bios-256k.bin';          Sha256 = 'AE6F6AA973AACCC143F57AA960FB035FD9DE4DAEE4AD0CD713322F8C259E7650' }
            @{ Path = 'tools/qemu/share/edk2-x86_64-code.fd';    Sha256 = '33090CC07675BAA5190D9F1E84BF5176B33BCBFA9BACAC522961150CDB6DBB2A' }
            @{ Path = 'tools/qemu/share/edk2-i386-vars.fd';      Sha256 = '5D2AC383371B408398ACCEE7EC27C8C09EA5B74A0DE0CEEA6513388B15BE5D1E' }
        )
    }

    OvmfRuntime = @{
        CrateVersion = '0.2.9'
        CachePath    = 'target/ovmf'
        SourcePolicy = 'unpinned-latest'
        Reproducible = $false
        Note = 'src/main.rs uses ovmf_prebuilt Source::LATEST; pin the firmware payload before treating cargo run -- uefi as reproducible.'
    }

    Offline = @{
        VendorDirectory = 'vendor'
        TemporaryCargoHome = '.nova-cargo-home'
    }

    GeneratedOrExternal = @(
        @{ Path = 'target/';     Kind = 'generated build cache';          Tracked = $false }
        @{ Path = 'dist/';       Kind = 'generated release artifacts';    Tracked = $false }
        @{ Path = 'tools/';      Kind = 'downloaded third-party tools';   Tracked = $false }
        @{ Path = 'vm-data/';    Kind = 'mutable VM disks and logs';      Tracked = $false }
    )
}
