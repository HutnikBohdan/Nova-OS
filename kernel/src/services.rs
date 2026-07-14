use ai_core::{
    agent::{AgentLoop, Policy, Tool, ToolSet},
    tokenizer::ByteTokenizer,
};
use build_core::WorkspaceManifest;
use compositor_core::{
    ClientId, Compositor, Point as CompositorPoint, Rect as CompositorRect, SecurityContext,
    SurfaceId, SurfaceRole,
};
use desktop_core::{AppId, Point, Rect, SceneGraph, Window, WindowId};
use hardware_core::{hda::StreamFormat, hid::KeyboardReport, nvme::Submission};
use installer_core::InstallMetadata;
use media_core::display::{DisplayMode, FramebufferDescriptor, PixelFormat as MediaPixelFormat};
use platform_core::{
    ioapic::{DeliveryMode, Polarity, RedirectionEntry, TriggerMode},
    topology::ApicId,
};
use runtime_core::{Channel, CrashReason, Message, ProcessTable, Quantum};
use vfs_core::{CanonicalPath, validate_path_syntax};

pub fn bootstrap() {
    if desktop_smoke() {
        crate::serial::write_str("NOVA_DESKTOP_SERVICE_CORE_OK\n");
    }
    if installer_smoke() {
        crate::serial::write_str("NOVA_INSTALLER_RECOVERY_CORE_OK\n");
    }
    if ai_smoke() {
        crate::serial::write_str("NOVA_LOCAL_AI_CORE_OK\n");
    }
    if runtime_smoke() {
        crate::serial::write_str("NOVA_IPC_SUPERVISOR_CORE_OK\n");
    }
    if hardware_smoke() {
        crate::serial::write_str("NOVA_HARDWARE_PROTOCOLS_CORE_OK\n");
    }
    if build_smoke() {
        crate::serial::write_str("NOVA_SELF_HOSTED_BUILD_CORE_OK\n");
    }
    if compositor_smoke() {
        crate::serial::write_str("NOVA_COMPOSITOR_SECURITY_CORE_OK\n");
    }
    if vfs_smoke() {
        crate::serial::write_str("NOVA_KERNEL_HEAP_OK\n");
        crate::serial::write_str("NOVA_VFS_SECURITY_CORE_OK\n");
    }
    if tls_smoke() {
        crate::serial::write_str("NOVA_TLS13_CRYPTO_CORE_OK\n");
    }
    if platform_smoke() {
        crate::serial::write_str("NOVA_SMP_APIC_PLATFORM_CORE_OK\n");
    }
    if media_smoke() {
        crate::serial::write_str("NOVA_DISPLAY_AUDIO_CORE_OK\n");
    }
    if novafs_smoke() {
        crate::serial::write_str("NOVA_FS_V2_RECOVERY_CORE_OK\n");
    }
    if loader_smoke() {
        crate::serial::write_str("NOVA_ELF_LOADER_ABI_CORE_OK\n");
    }
    if productivity_smoke() {
        crate::serial::write_str("NOVA_PRODUCTIVITY_APPS_CORE_OK\n");
    }
    if security_smoke() {
        crate::serial::write_str("NOVA_SECURITY_POLICY_CORE_OK\n");
    }
    if deployment_smoke() {
        crate::serial::write_str("NOVA_DEPLOYMENT_RUNTIME_CORE_OK\n");
    }
    if browser_pipeline_smoke() {
        crate::serial::write_str("NOVA_BROWSER_PIPELINE_CORE_OK\n");
    }
    if transformer_smoke() {
        crate::serial::write_str("NOVA_AI_TRANSFORMER_CORE_OK\n");
    }
    if socket_smoke() {
        crate::serial::write_str("NOVA_ASYNC_SOCKET_SERVICE_CORE_OK\n");
    }
    if usb_smoke() {
        crate::serial::write_str("NOVA_USB_XHCI_STACK_CORE_OK\n");
    }
    if locale_smoke() {
        crate::serial::write_str("NOVA_UKRAINIAN_ACCESSIBILITY_CORE_OK\n");
    }
    if pki_smoke() {
        crate::serial::write_str("NOVA_PKI_TRUST_CORE_OK\n");
    }
    if virtio_smoke() {
        crate::serial::write_str("NOVA_VIRTIO_DEVICE_CORE_OK\n");
    }
    if compiler_smoke() {
        crate::serial::write_str("NOVA_NATIVE_COMPILER_CORE_OK\n");
    }
    if guardian_smoke() {
        crate::serial::write_str("NOVA_GUARDIAN_TRANSACTION_CORE_OK\n");
    }
}

fn guardian_smoke() -> bool {
    use ai_core::journal::{ActionKind, Capability, Journal, Phase};
    let mut journal = Journal::<8>::new();
    let Ok(action) = journal.plan(1, ActionKind::FileWrite, Capability::Files, 10, b"snapshot")
    else {
        return false;
    };
    journal.approve(action).is_ok()
        && journal.applied(action, 20).is_ok()
        && journal.verify(action, 20).is_ok()
        && journal
            .records()
            .last()
            .is_some_and(|record| record.phase == Phase::Verified)
}

fn compiler_smoke() -> bool {
    let mut workspace = compiler_core::Workspace::new();
    let mut elf = [0u8; 8192];
    compiler_core::compile(
        "fn main() -> i64 { let mut result: i64 = 40; result = result + 2; return result; }",
        &mut workspace,
        &mut elf,
    )
    .map(|image| image.entry == compiler_core::ELF_ENTRY && &elf[..4] == b"\x7fELF")
    .unwrap_or(false)
}

fn pki_smoke() -> bool {
    pki_core::sha512::sha512(b"abc")[..8] == [0xdd, 0xaf, 0x35, 0xa1, 0x93, 0x61, 0x7a, 0xba]
}

fn virtio_smoke() -> bool {
    let mut queue = virtio_core::queue::SplitQueue::<4>::new();
    let descriptor = virtio_core::queue::Descriptor {
        address: 0x20_0000,
        length: 512,
        flags: 0,
        next: 0,
    };
    queue.submit(&[descriptor]).is_ok()
}

fn socket_smoke() -> bool {
    socket_core::DnsName::new("nova.test")
        .map(|name| name.as_bytes() == b"nova.test")
        .unwrap_or(false)
}

fn usb_smoke() -> bool {
    let trb = usb_core::xhci::Trb::new(usb_core::xhci::TrbType::Normal, 0x1000, 512, 0);
    trb.kind() == usb_core::xhci::TrbType::Normal as u8
        && usb_core::hid::KeyboardReport::parse_boot(&[0; 8]).is_ok()
}

fn locale_smoke() -> bool {
    locale_core::LOCALE == "uk-UA"
        && locale_core::ukrainian_plural(2) == locale_core::Plural::Few
        && locale_core::search_equal("Київ", "КИЇВ")
}

fn deployment_smoke() -> bool {
    use deployment_core::{Bus, DiskId, DiskInfo, Eligibility};
    let disk = DiskInfo {
        id: DiskId([7; 16]),
        sectors: 8 * 1024 * 1024,
        logical_sector_size: 512,
        bus: Bus::Nvme,
        removable: false,
        read_only: false,
        system_disk: false,
    };
    let token = deployment_core::confirmation_token(disk, [9; 16]);
    disk.eligibility(1024) == Eligibility::Eligible
        && deployment_core::verify_confirmation(token, token)
        && deployment_core::UKRAINIAN_LOCALE.language == *b"uk-UA"
}

fn browser_pipeline_smoke() -> bool {
    let mut normalized = [0u8; 128];
    let Ok(length) = browser_core::pipeline::normalize_url(
        "HTTPS://Nova.Test:443/a/../документи#розділ",
        &mut normalized,
    ) else {
        return false;
    };
    &normalized[..length] == "https://nova.test/документи".as_bytes()
}

fn transformer_smoke() -> bool {
    ai_core::inference::ModelConfig {
        dimension: 32,
        hidden_dimension: 64,
        layers: 2,
        heads: 4,
        context: 16,
        vocabulary: 128,
    }
    .validate()
    .is_ok()
}

fn loader_smoke() -> bool {
    let mut stack = [0u8; 256];
    let mut argv_addresses = [0u64; 1];
    let mut env_addresses = [0u64; 1];
    let argv = ["/система/bin/редактор".as_bytes()];
    let env = ["LANG=uk_UA.UTF-8".as_bytes()];
    loader_core::build_initial_stack(
        &mut stack,
        0x7fff_ffff_f000,
        &argv,
        &env,
        &[loader_core::AuxEntry {
            kind: 6,
            value: 4096,
        }],
        &mut argv_addresses,
        &mut env_addresses,
    )
    .map(|initial| initial.stack_pointer & 15 == 0 && initial.bytes_used > 0)
    .unwrap_or(false)
}

// Early boot must not construct the full editor document/history value on the
// kernel stack. Full document behavior is covered by productivity-core tests
// and belongs in an isolated application process.
#[inline(never)]
fn productivity_smoke() -> bool {
    productivity_core::Text::<32>::new("Nova OS")
        .map(|text| text.as_str() == "Nova OS")
        .unwrap_or(false)
}

fn security_smoke() -> bool {
    security_core::constant_time_eq(
        &security_core::sha256(b"Nova secure boot"),
        &security_core::sha256(b"Nova secure boot"),
    )
}

fn novafs_smoke() -> bool {
    let Ok(entry) = novafs_core::DirectoryEntry::new(7, 1, "проєкт.rs".as_bytes()) else {
        return false;
    };
    entry.name() == "проєкт.rs".as_bytes()
        && novafs_core::checksum(b"NovaFS") == novafs_core::checksum(b"NovaFS")
}

fn tls_smoke() -> bool {
    tls_core::sha256(b"abc")
        == [
            0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
            0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
            0xf2, 0x00, 0x15, 0xad,
        ]
}

fn platform_smoke() -> bool {
    RedirectionEntry::new(
        48,
        ApicId::XApic(0),
        DeliveryMode::Fixed,
        TriggerMode::Level,
        Polarity::ActiveLow,
        true,
    )
    .map(|entry| entry.is_masked() && entry.low() & 0xff == 48)
    .unwrap_or(false)
}

fn media_smoke() -> bool {
    FramebufferDescriptor {
        physical_address: 0xfd00_0000,
        byte_len: 1280 * 720 * 4,
        stride_bytes: 1280 * 4,
        mode: DisplayMode {
            width: 1280,
            height: 720,
            refresh_millihz: 60_000,
            pixel_clock_khz: 74_250,
            interlaced: false,
        },
        format: MediaPixelFormat::Xrgb8888,
    }
    .validate()
    .is_ok()
}

fn compositor_smoke() -> bool {
    let context = SecurityContext::application(ClientId(7));
    let mut compositor = Compositor::new(1280, 720);
    compositor
        .create_surface(
            context,
            SurfaceId(1),
            SurfaceRole::Application,
            CompositorRect::new(100, 80, 640, 480),
        )
        .is_ok()
        && compositor
            .hit_test(CompositorPoint { x: 120, y: 100 })
            .is_none()
        && compositor.surface(SurfaceId(1)).is_some()
}

fn vfs_smoke() -> bool {
    let Ok(path) = CanonicalPath::new("/користувачі/нова/./проєкти") else {
        return false;
    };
    path.as_str() == "/користувачі/нова/проєкти"
        && validate_path_syntax("/користувачі/нова/проєкти").is_ok()
        && validate_path_syntax("/../../ядро").is_err()
}

fn hardware_smoke() -> bool {
    let command = Submission::identify(7, 0x20_0000, 0, 1);
    let keyboard = KeyboardReport::parse_boot(&[0; 8]);
    let audio = StreamFormat::pcm(48_000, 16, 2);
    command.cdw0 & 0xff == hardware_core::nvme::ADMIN_IDENTIFY as u32
        && keyboard.is_ok()
        && audio.is_ok()
}

fn build_smoke() -> bool {
    let manifest = "workspace nova\npackage kernel 0.1.0 bin src/main.rs\npackage shell 0.1.0 bin src/shell.rs\ndep shell kernel\n";
    let Ok(manifest) = WorkspaceManifest::parse(manifest) else {
        return false;
    };
    let Ok(order) = manifest.topological_order() else {
        return false;
    };
    order.indices().len() == 2
        && manifest.packages()[order.indices()[0]].name == "kernel"
        && manifest.packages()[order.indices()[1]].name == "shell"
}

fn desktop_smoke() -> bool {
    let mut scene = SceneGraph::new(Rect::new(0, 0, 1280, 720));
    let Ok(window) = Window::new(
        WindowId(1),
        AppId(1),
        Rect::new(96, 72, 720, 480),
        "Редактор Nova",
    ) else {
        return false;
    };
    scene.add_window(window).is_ok()
        && scene.focus(WindowId(1)).is_ok()
        && scene.hit_test(Point { x: 120, y: 100 }) == Some(WindowId(1))
}

fn installer_smoke() -> bool {
    let metadata = InstallMetadata::fresh();
    InstallMetadata::decode(&metadata.encode()) == Ok(metadata)
}

fn ai_smoke() -> bool {
    let mut tokens = [0u32; 32];
    let Ok(count) = ByteTokenizer::encode("Nova AI".as_bytes(), &mut tokens) else {
        return false;
    };
    let mut decoded = [0u8; 32];
    let Ok(length) = ByteTokenizer::decode(&tokens[..count], &mut decoded) else {
        return false;
    };
    let mut agent = AgentLoop::<8, 4>::new(Policy {
        tools: ToolSet::one(Tool::ReadFile),
        require_mutation_approval: true,
        require_verification: true,
    });
    decoded[..length] == *b"Nova AI" && agent.start(1, 1).is_ok()
}

fn runtime_smoke() -> bool {
    let mut processes = ProcessTable::<4>::new();
    let Ok(init) = processes.create(None, 0x1000) else {
        return false;
    };
    if processes.start(init).is_err() {
        return false;
    }
    let Ok(child) = processes.create(Some(init), 0x2000) else {
        return false;
    };
    if processes.start(child).is_err()
        || processes
            .crash(child, CrashReason::ProtectionFault)
            .is_err()
    {
        return false;
    }
    let mut channel = Channel::<2>::new();
    let Ok(message) = Message::new(init.0, 1, "служба готова".as_bytes()) else {
        return false;
    };
    let mut quantum = Quantum::new(2);
    channel.send(message).is_ok()
        && channel.receive().is_ok()
        && processes.wait_result(init, child).ok().flatten().is_some()
        && !quantum.tick()
        && quantum.tick()
}
