use bootloader_api::{BootInfo, info::MemoryRegionKind};
use core::cell::UnsafeCell;
use x86_64::{
    PhysAddr,
    structures::paging::{FrameAllocator, PhysFrame, Size4KiB},
};

const MAX_BOOT_FRAMES: usize = 4096;

struct FrameState {
    frames: [u64; MAX_BOOT_FRAMES],
    returned: [u64; MAX_BOOT_FRAMES],
    returned_count: usize,
    count: usize,
    next: usize,
    initialized: bool,
}

struct SharedFrames(UnsafeCell<FrameState>);
// Allocation/release are serialized only by Nova's current single-core kernel
// entry discipline. Add a real lock before enabling SMP allocation paths.
unsafe impl Sync for SharedFrames {}

static FRAMES: SharedFrames = SharedFrames(UnsafeCell::new(FrameState {
    frames: [0; MAX_BOOT_FRAMES],
    returned: [0; MAX_BOOT_FRAMES],
    returned_count: 0,
    count: 0,
    next: 0,
    initialized: false,
}));

pub fn init(boot_info: &BootInfo) {
    let state = unsafe { &mut *FRAMES.0.get() };
    if state.initialized {
        return;
    }
    for region in boot_info
        .memory_regions
        .iter()
        .filter(|region| region.kind == MemoryRegionKind::Usable)
    {
        let mut address = (region.start + 4095) & !4095;
        while address + 4096 <= region.end && state.count < state.frames.len() {
            state.frames[state.count] = address;
            state.count += 1;
            address += 4096;
        }
        if state.count == state.frames.len() {
            break;
        }
    }
    state.initialized = true;
}

pub fn available() -> usize {
    let state = unsafe { &*FRAMES.0.get() };
    state.count.saturating_sub(state.next) + state.returned_count
}

pub fn release(frame: PhysFrame<Size4KiB>) -> bool {
    let state = unsafe { &mut *FRAMES.0.get() };
    let address = frame.start_address().as_u64();
    if !state.initialized
        || !state.frames[..state.next].contains(&address)
        || state.returned[..state.returned_count].contains(&address)
        || state.returned_count == state.returned.len()
    {
        return false;
    }
    state.returned[state.returned_count] = address;
    state.returned_count += 1;
    true
}

#[derive(Clone, Copy)]
pub struct GlobalFrames;

unsafe impl FrameAllocator<Size4KiB> for GlobalFrames {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let state = unsafe { &mut *FRAMES.0.get() };
        if !state.initialized {
            return None;
        }
        if state.returned_count > 0 {
            state.returned_count -= 1;
            let address = state.returned[state.returned_count];
            state.returned[state.returned_count] = 0;
            return PhysFrame::from_start_address(PhysAddr::new(address)).ok();
        }
        if state.next >= state.count {
            return None;
        }
        let address = state.frames[state.next];
        state.next += 1;
        PhysFrame::from_start_address(PhysAddr::new(address)).ok()
    }
}
