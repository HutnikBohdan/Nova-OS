use core::{
    alloc::{GlobalAlloc, Layout},
    cell::UnsafeCell,
    mem, ptr,
    sync::atomic::{AtomicBool, Ordering},
};

const HEAP_BYTES: usize = 4 * 1024 * 1024;
const NONE: usize = usize::MAX;

#[repr(align(4096))]
struct Arena([u8; HEAP_BYTES]);

#[repr(C)]
#[derive(Clone, Copy)]
struct FreeNode {
    size: usize,
    next: usize,
}

#[repr(C)]
struct AllocationHeader {
    offset: usize,
    size: usize,
}

pub struct NovaAllocator {
    arena: UnsafeCell<Arena>,
    locked: AtomicBool,
    initialized: AtomicBool,
    head: UnsafeCell<usize>,
}

unsafe impl Sync for NovaAllocator {}

impl NovaAllocator {
    pub const fn new() -> Self {
        Self {
            arena: UnsafeCell::new(Arena([0; HEAP_BYTES])),
            locked: AtomicBool::new(false),
            initialized: AtomicBool::new(false),
            head: UnsafeCell::new(NONE),
        }
    }

    fn acquire(&self) {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn release(&self) {
        self.locked.store(false, Ordering::Release);
    }

    unsafe fn base(&self) -> *mut u8 {
        unsafe { (*self.arena.get()).0.as_mut_ptr() }
    }

    unsafe fn node(&self, offset: usize) -> *mut FreeNode {
        unsafe { self.base().add(offset).cast() }
    }

    unsafe fn initialize_locked(&self) {
        if !self.initialized.load(Ordering::Relaxed) {
            unsafe {
                self.node(0).write(FreeNode {
                    size: HEAP_BYTES,
                    next: NONE,
                });
                *self.head.get() = 0;
            }
            self.initialized.store(true, Ordering::Release);
        }
    }

    fn align_up(value: usize, alignment: usize) -> Option<usize> {
        value
            .checked_add(alignment - 1)
            .map(|v| v & !(alignment - 1))
    }

    unsafe fn insert_free_locked(&self, offset: usize, size: usize) {
        let mut previous = NONE;
        let mut current = unsafe { *self.head.get() };
        while current != NONE && current < offset {
            previous = current;
            current = unsafe { (*self.node(current)).next };
        }
        unsafe {
            self.node(offset).write(FreeNode {
                size,
                next: current,
            });
            if previous == NONE {
                *self.head.get() = offset;
            } else {
                (*self.node(previous)).next = offset;
            }

            if current != NONE && offset + (*self.node(offset)).size == current {
                (*self.node(offset)).size += (*self.node(current)).size;
                (*self.node(offset)).next = (*self.node(current)).next;
            }
            if previous != NONE && previous + (*self.node(previous)).size == offset {
                (*self.node(previous)).size += (*self.node(offset)).size;
                (*self.node(previous)).next = (*self.node(offset)).next;
            }
        }
    }
}

unsafe impl GlobalAlloc for NovaAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.acquire();
        unsafe { self.initialize_locked() };
        let base = unsafe { self.base() as usize };
        let requested = layout.size().max(1);
        let mut previous = NONE;
        let mut current = unsafe { *self.head.get() };
        while current != NONE {
            let node = unsafe { *self.node(current) };
            let Some(user) = Self::align_up(
                base + current + mem::size_of::<AllocationHeader>(),
                layout.align().max(mem::align_of::<AllocationHeader>()),
            ) else {
                self.release();
                return ptr::null_mut();
            };
            let Some(end) = user.checked_add(requested) else {
                self.release();
                return ptr::null_mut();
            };
            let consumed = end - (base + current);
            if consumed <= node.size {
                let remainder = node.size - consumed;
                let replacement = if remainder >= mem::size_of::<FreeNode>() {
                    let offset = current + consumed;
                    unsafe {
                        self.node(offset).write(FreeNode {
                            size: remainder,
                            next: node.next,
                        });
                    }
                    offset
                } else {
                    node.next
                };
                let actual = if remainder < mem::size_of::<FreeNode>() {
                    node.size
                } else {
                    consumed
                };
                unsafe {
                    if previous == NONE {
                        *self.head.get() = replacement;
                    } else {
                        (*self.node(previous)).next = replacement;
                    }
                    (user as *mut AllocationHeader)
                        .sub(1)
                        .write(AllocationHeader {
                            offset: current,
                            size: actual,
                        });
                }
                self.release();
                return user as *mut u8;
            }
            previous = current;
            current = node.next;
        }
        self.release();
        ptr::null_mut()
    }

    unsafe fn dealloc(&self, address: *mut u8, _layout: Layout) {
        if address.is_null() {
            return;
        }
        self.acquire();
        let header = unsafe { (address as *mut AllocationHeader).sub(1).read() };
        unsafe { self.insert_free_locked(header.offset, header.size) };
        self.release();
    }
}

#[global_allocator]
static ALLOCATOR: NovaAllocator = NovaAllocator::new();
