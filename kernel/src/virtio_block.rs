//! Real VirtIO PCI legacy block path used as a QEMU hardware acceptance gate.

use bootloader_api::BootInfo;
use core::{
    arch::asm,
    ptr,
    sync::atomic::{Ordering, compiler_fence},
};
use driver_core::{Bar, decode_bar};
use x86_64::structures::paging::FrameAllocator;

const VIRTIO_VENDOR: u16 = 0x1af4;
const BLOCK_DEVICES: [u16; 1] = [0x1001];
const MIN_QUEUE_SIZE: usize = 8;
const REG_HOST_FEATURES: u16 = 0x00;
const REG_GUEST_FEATURES: u16 = 0x04;
const REG_QUEUE_PFN: u16 = 0x08;
const REG_QUEUE_SIZE: u16 = 0x0c;
const REG_QUEUE_SELECT: u16 = 0x0e;
const REG_QUEUE_NOTIFY: u16 = 0x10;
const REG_STATUS: u16 = 0x12;

#[repr(C)]
#[derive(Clone, Copy)]
struct Descriptor {
    address: u64,
    length: u32,
    flags: u16,
    next: u16,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Request {
    kind: u32,
    reserved: u32,
    sector: u64,
}

#[derive(Clone, Copy)]
struct Device {
    port: u16,
    queue: u64,
    data: u64,
    data_phys: u64,
    queue_size: usize,
    next_index: u16,
}
static mut DEVICE: Option<Device> = None;
const GUARDIAN_SECTOR: u64 = 16;
const GUARDIAN_CAPACITY: usize = 3072;
const UNDO_SECTOR: u64 = 32;
const UNDO_CAPACITY: usize = 1536;
const SNAPSHOT_SECTOR: u64 = 64;
const SNAPSHOT_SLOTS: u64 = 8;
const SNAPSHOT_STRIDE: u64 = 4;
const SNAPSHOT_CAPACITY: usize = 1540;

pub fn initialize(boot_info: &BootInfo) {
    let Some(device) = crate::pci::find_vendor(VIRTIO_VENDOR, &BLOCK_DEVICES) else {
        return;
    };
    let Bar::Io { port } = decode_bar(crate::pci::read_config(device.address, 0x10)) else {
        crate::serial::write_str("NOVA_VIRTIO_BLOCK_NO_LEGACY_IO\n");
        return;
    };
    let command = crate::pci::read_config(device.address, 0x04);
    crate::pci::write_config(device.address, 0x04, command | 0x0000_0005);
    unsafe {
        out8(port + REG_STATUS, 0);
        out8(port + REG_STATUS, 1);
        out8(port + REG_STATUS, 3);
    }
    let features = unsafe { in32(port + REG_HOST_FEATURES) };
    unsafe {
        out32(port + REG_GUEST_FEATURES, 0);
        out16(port + REG_QUEUE_SELECT, 0);
    }
    let size = unsafe { in16(port + REG_QUEUE_SIZE) } as usize;
    if size < MIN_QUEUE_SIZE || size > 256 {
        crate::serial::write_str("NOVA_VIRTIO_BLOCK_QUEUE_SMALL\n");
        return;
    }
    let mut frames = crate::memory::GlobalFrames;
    let Some(first) = allocate_dma_frame(&mut frames) else {
        return;
    };
    let Some(second) = allocate_dma_frame(&mut frames) else {
        return;
    };
    let Some(third) = allocate_dma_frame(&mut frames) else {
        return;
    };
    if second.start_address().as_u64() != first.start_address().as_u64() + 4096
        || third.start_address().as_u64() != first.start_address().as_u64() + 8192
    {
        crate::serial::write_str("NOVA_VIRTIO_BLOCK_DMA_FRAGMENTED\n");
        return;
    }
    let Some(data) = allocate_dma_frame(&mut frames) else {
        return;
    };
    let Some(offset) = boot_info.physical_memory_offset.into_option() else {
        return;
    };
    let queue_phys = first.start_address().as_u64();
    let queue_virt = offset + queue_phys;
    let data_phys = data.start_address().as_u64();
    let data_virt = offset + data_phys;
    unsafe {
        ptr::write_bytes(queue_virt as *mut u8, 0, 12288);
        ptr::write_bytes(data_virt as *mut u8, 0, 4096);
        out32(port + REG_QUEUE_PFN, (queue_phys >> 12) as u32);
        out8(port + REG_STATUS, 7);
    }
    crate::serial::write_fmt(format_args!(
        "NOVA_VIRTIO_BLOCK_PROBE=port:{port:04x},queue:{size},features:{features:08x},phys:{queue_phys:08x},pfn:{:08x}\n",
        unsafe { in32(port + REG_QUEUE_PFN) }
    ));
    let Some(persisted) = roundtrip(port, queue_virt, data_virt, data_phys, size) else {
        crate::serial::write_str("NOVA_VIRTIO_BLOCK_IO_FAILED\n");
        return;
    };
    if persisted {
        crate::serial::write_str("NOVA_VIRTIO_BLOCK_PERSISTED_OK\n");
    }
    crate::serial::write_str("NOVA_VIRTIO_BLOCK_RW_OK\n");
    unsafe {
        *ptr::addr_of_mut!(DEVICE) = Some(Device {
            port,
            queue: queue_virt,
            data: data_virt,
            data_phys,
            queue_size: size,
            next_index: 3,
        });
    }
}

pub fn write_guardian(payload: &[u8]) -> bool {
    write_blob(GUARDIAN_SECTOR, b"NVG1", GUARDIAN_CAPACITY, payload)
}
pub fn write_undo(payload: &[u8]) -> bool {
    write_blob(UNDO_SECTOR, b"NVU1", UNDO_CAPACITY, payload)
}
pub fn write_snapshot(action_id: u32, payload: &[u8]) -> bool {
    if payload.len() + 4 > SNAPSHOT_CAPACITY {
        return false;
    }
    let mut envelope = [0u8; SNAPSHOT_CAPACITY];
    envelope[..4].copy_from_slice(&action_id.to_le_bytes());
    envelope[4..4 + payload.len()].copy_from_slice(payload);
    write_blob(
        snapshot_sector(action_id),
        b"NVS1",
        SNAPSHOT_CAPACITY,
        &envelope[..4 + payload.len()],
    )
}
fn write_blob(start_sector: u64, magic: &[u8; 4], capacity: usize, payload: &[u8]) -> bool {
    if payload.len() > capacity {
        return false;
    }
    x86_64::instructions::interrupts::without_interrupts(|| unsafe {
        let Some(device) = (&mut *ptr::addr_of_mut!(DEVICE)).as_mut() else {
            return false;
        };
        let mut block = [0u8; 512];
        block[..4].copy_from_slice(magic);
        block[4..8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        block[8..12].copy_from_slice(&crc32(payload).to_le_bytes());
        let first = payload.len().min(500);
        block[12..12 + first].copy_from_slice(&payload[..first]);
        if !sector_io(device, 1, start_sector, &mut block) {
            return false;
        }
        let mut at = first;
        let mut sector = start_sector + 1;
        while at < payload.len() {
            block.fill(0);
            let count = (payload.len() - at).min(512);
            block[..count].copy_from_slice(&payload[at..at + count]);
            if !sector_io(device, 1, sector, &mut block) {
                return false;
            }
            at += count;
            sector += 1;
        }
        true
    })
}

pub fn read_guardian(output: &mut [u8]) -> Option<usize> {
    read_blob(GUARDIAN_SECTOR, b"NVG1", GUARDIAN_CAPACITY, output)
}
pub fn read_undo(output: &mut [u8]) -> Option<usize> {
    read_blob(UNDO_SECTOR, b"NVU1", UNDO_CAPACITY, output)
}
pub fn read_snapshot(action_id: u32, output: &mut [u8]) -> Option<usize> {
    let mut envelope = [0u8; SNAPSHOT_CAPACITY];
    let len = read_blob(
        snapshot_sector(action_id),
        b"NVS1",
        SNAPSHOT_CAPACITY,
        &mut envelope,
    )?;
    if len < 4 || u32::from_le_bytes(envelope[..4].try_into().ok()?) != action_id {
        return None;
    }
    let payload_len = len - 4;
    if payload_len > output.len() {
        return None;
    }
    output[..payload_len].copy_from_slice(&envelope[4..len]);
    Some(payload_len)
}

const fn snapshot_sector(action_id: u32) -> u64 {
    SNAPSHOT_SECTOR + (action_id.saturating_sub(1) as u64 % SNAPSHOT_SLOTS) * SNAPSHOT_STRIDE
}
fn read_blob(
    start_sector: u64,
    magic: &[u8; 4],
    capacity: usize,
    output: &mut [u8],
) -> Option<usize> {
    x86_64::instructions::interrupts::without_interrupts(|| unsafe {
        let device = (&mut *ptr::addr_of_mut!(DEVICE)).as_mut()?;
        let mut block = [0u8; 512];
        if !sector_io(device, 0, start_sector, &mut block) || &block[..4] != magic {
            return None;
        }
        let len = u32::from_le_bytes(block[4..8].try_into().ok()?) as usize;
        if len > output.len() || len > capacity {
            return None;
        }
        let expected = u32::from_le_bytes(block[8..12].try_into().ok()?);
        let first = len.min(500);
        output[..first].copy_from_slice(&block[12..12 + first]);
        let mut at = first;
        let mut sector = start_sector + 1;
        while at < len {
            if !sector_io(device, 0, sector, &mut block) {
                return None;
            }
            let count = (len - at).min(512);
            output[at..at + count].copy_from_slice(&block[..count]);
            at += count;
            sector += 1;
        }
        if crc32(&output[..len]) == expected {
            Some(len)
        } else {
            None
        }
    })
}

fn sector_io(device: &mut Device, kind: u32, sector: u64, block: &mut [u8; 512]) -> bool {
    if kind == 1 {
        unsafe {
            ptr::copy_nonoverlapping(block.as_ptr(), (device.data + 64) as *mut u8, 512);
        }
    }
    let index = device.next_index;
    if !request(
        device.port,
        device.queue,
        device.data,
        device.data_phys,
        kind,
        sector,
        index,
        device.queue_size,
    ) {
        return false;
    }
    device.next_index = device.next_index.wrapping_add(1);
    if kind == 0 {
        unsafe {
            ptr::copy_nonoverlapping((device.data + 64) as *const u8, block.as_mut_ptr(), 512);
        }
    }
    true
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn allocate_dma_frame(
    frames: &mut crate::memory::GlobalFrames,
) -> Option<x86_64::structures::paging::PhysFrame> {
    loop {
        let frame = frames.allocate_frame()?;
        if frame.start_address().as_u64() >= 0x10_0000 {
            return Some(frame);
        }
    }
}

fn roundtrip(port: u16, queue: u64, data: u64, data_phys: u64, queue_size: usize) -> Option<bool> {
    const PATTERN: &[u8] = b"Nova VirtIO persistent block proof";
    unsafe {
        ptr::write_bytes((data + 64) as *mut u8, 0, 512);
    }
    if !request(port, queue, data, data_phys, 0, 8, 0, queue_size) {
        return None;
    }
    let persisted =
        unsafe { core::slice::from_raw_parts((data + 64) as *const u8, PATTERN.len()) } == PATTERN;
    unsafe {
        ptr::copy_nonoverlapping(PATTERN.as_ptr(), (data + 64) as *mut u8, PATTERN.len());
    }
    if !request(port, queue, data, data_phys, 1, 8, 1, queue_size) {
        return None;
    }
    unsafe {
        ptr::write_bytes((data + 64) as *mut u8, 0, 512);
    }
    if !request(port, queue, data, data_phys, 0, 8, 2, queue_size) {
        return None;
    }
    let read = unsafe { core::slice::from_raw_parts((data + 64) as *const u8, PATTERN.len()) };
    if read == PATTERN {
        Some(persisted)
    } else {
        None
    }
}

fn request(
    port: u16,
    queue: u64,
    data: u64,
    data_phys: u64,
    kind: u32,
    sector: u64,
    index: u16,
    queue_size: usize,
) -> bool {
    let descriptors = queue as *mut Descriptor;
    let request = Request {
        kind,
        reserved: 0,
        sector,
    };
    unsafe {
        ptr::write_volatile(data as *mut Request, request);
        ptr::write_volatile((data + 576) as *mut u8, 0xff);
        ptr::write_volatile(
            descriptors.add(0),
            Descriptor {
                address: data_phys,
                length: 16,
                flags: 1,
                next: 1,
            },
        );
        ptr::write_volatile(
            descriptors.add(1),
            Descriptor {
                address: data_phys + 64,
                length: 512,
                flags: 1 | if kind == 0 { 2 } else { 0 },
                next: 2,
            },
        );
        ptr::write_volatile(
            descriptors.add(2),
            Descriptor {
                address: data_phys + 576,
                length: 1,
                flags: 2,
                next: 0,
            },
        );
        let avail = queue + queue_size as u64 * 16;
        ptr::write_volatile(
            (avail + 4 + (index as u64 % queue_size as u64) * 2) as *mut u16,
            0,
        );
        compiler_fence(Ordering::Release);
        ptr::write_volatile((avail + 2) as *mut u16, index + 1);
        out16(port + REG_QUEUE_NOTIFY, 0);
        let used = (avail + 6 + queue_size as u64 * 2 + 4095) & !4095;
        for _ in 0..2_000_000 {
            compiler_fence(Ordering::Acquire);
            if ptr::read_volatile((used + 2) as *const u16) == index + 1 {
                return ptr::read_volatile((data + 576) as *const u8) == 0;
            }
            core::hint::spin_loop();
        }
        crate::serial::write_fmt(format_args!(
            "NOVA_VIRTIO_BLOCK_TIMEOUT=used:{},status:{},device:{:02x}\n",
            ptr::read_volatile((used + 2) as *const u16),
            ptr::read_volatile((data + 576) as *const u8),
            in8(port + REG_STATUS),
        ));
    }
    false
}

unsafe fn out8(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
    }
}
unsafe fn out16(port: u16, value: u16) {
    unsafe {
        asm!("out dx, ax", in("dx") port, in("ax") value, options(nomem, nostack));
    }
}
unsafe fn out32(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack));
    }
}
unsafe fn in8(port: u16) -> u8 {
    let value;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack));
    }
    value
}
unsafe fn in16(port: u16) -> u16 {
    let value;
    unsafe {
        asm!("in ax, dx", in("dx") port, out("ax") value, options(nomem, nostack));
    }
    value
}
unsafe fn in32(port: u16) -> u32 {
    let value;
    unsafe {
        asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack));
    }
    value
}
