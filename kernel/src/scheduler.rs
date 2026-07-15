use core::{cell::UnsafeCell, ptr};
use runtime_core::{CpuContext, FxSaveArea, ProcessId, SchedulerCore, ThreadId};
use x86_64::registers::control::Cr3;

use crate::user_space::AddressSpaceOwner;

const MAX_TASKS: usize = 8;
const PROOF_EXIT_SWITCH: u64 = 12;
const PROOF_FINISH_SWITCH: u64 = 24;

struct ScheduledTask {
    process: ProcessId,
    thread: ThreadId,
    context: CpuContext,
    extended_state: FxSaveArea,
    alive: bool,
    address_space: Option<AddressSpaceOwner>,
}

pub struct KernelScheduler {
    tasks: [Option<ScheduledTask>; MAX_TASKS],
    dispatch: SchedulerCore<MAX_TASKS>,
    installed: usize,
    switches: u64,
    proof_mode: bool,
    proof_exit_armed: bool,
    proof_exit_handoff: bool,
    post_exit_threads: u64,
    retired: [Option<AddressSpaceOwner>; MAX_TASKS],
    retired_level4: u64,
    reclaimed_frames: usize,
}

impl KernelScheduler {
    pub const fn new() -> Self {
        Self {
            tasks: [const { None }; MAX_TASKS],
            dispatch: SchedulerCore::new(),
            installed: 0,
            switches: 0,
            proof_mode: false,
            proof_exit_armed: false,
            proof_exit_handoff: false,
            post_exit_threads: 0,
            retired: [const { None }; MAX_TASKS],
            retired_level4: 0,
            reclaimed_frames: 0,
        }
    }

    pub fn reset(&mut self, proof_mode: bool) -> bool {
        let _ = self.reclaim_all();
        if self.retired.iter().any(Option::is_some)
            || self
                .tasks
                .iter()
                .flatten()
                .any(|task| task.address_space.is_some())
        {
            return false;
        }
        *self = Self::new();
        self.proof_mode = proof_mode;
        true
    }

    pub fn install(
        &mut self,
        process: ProcessId,
        thread: ThreadId,
        context: CpuContext,
        address_space: AddressSpaceOwner,
    ) -> bool {
        if self.task(thread).is_some() {
            return false;
        }
        let Some(slot) = self.tasks.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        if self.dispatch.install(thread).is_err() {
            return false;
        }
        *slot = Some(ScheduledTask {
            process,
            thread,
            context,
            extended_state: FxSaveArea::clean(),
            alive: true,
            address_space: Some(address_space),
        });
        self.installed += 1;
        true
    }

    fn task(&self, thread: ThreadId) -> Option<&ScheduledTask> {
        self.tasks
            .iter()
            .flatten()
            .find(|task| task.thread == thread)
    }

    fn task_mut(&mut self, thread: ThreadId) -> Option<&mut ScheduledTask> {
        self.tasks
            .iter_mut()
            .flatten()
            .find(|task| task.thread == thread)
    }

    fn context_ptr(&mut self, thread: ThreadId) -> *mut CpuContext {
        self.task_mut(thread)
            .filter(|task| task.alive)
            .map(|task| ptr::addr_of_mut!(task.context))
            .unwrap_or(ptr::null_mut())
    }

    fn current_extended_state_ptr(&mut self) -> *mut u8 {
        let Some(thread) = self.dispatch.current() else {
            return ptr::null_mut();
        };
        self.task_mut(thread)
            .filter(|task| task.alive)
            .map(|task| task.extended_state.as_mut_ptr())
            .unwrap_or(ptr::null_mut())
    }

    fn mark_post_exit_dispatch(&mut self, thread: ThreadId) {
        if self.proof_exit_handoff && thread.0 < 64 {
            self.post_exit_threads |= 1u64 << thread.0;
        }
    }

    pub fn start(&mut self) -> *mut CpuContext {
        let Ok(thread) = self.dispatch.start() else {
            return ptr::null_mut();
        };
        self.context_ptr(thread)
    }

    fn exit_current(&mut self) -> *mut CpuContext {
        let Ok((exited, next)) = self.dispatch.exit_current() else {
            return ptr::null_mut();
        };
        let retired = if let Some(task) = self.task_mut(exited) {
            task.alive = false;
            task.address_space.take()
        } else {
            None
        };
        if let Some(owner) = retired {
            self.retired_level4 = owner.level4_address();
            let Some(slot) = self.retired.iter_mut().find(|slot| slot.is_none()) else {
                return ptr::null_mut();
            };
            *slot = Some(owner);
        }
        let Some(next) = next else {
            return ptr::null_mut();
        };
        self.mark_post_exit_dispatch(next);
        self.context_ptr(next)
    }

    pub fn on_tick(&mut self) -> *mut CpuContext {
        self.reap_retired();
        self.switches = self.switches.saturating_add(1);

        if self.proof_mode && self.switches == PROOF_EXIT_SWITCH {
            let Some(current) = self.dispatch.current() else {
                return ptr::null_mut();
            };
            let Some(task) = self.task_mut(current) else {
                return ptr::null_mut();
            };
            task.context.instruction_pointer = crate::process::PREEMPT_EXIT_IP;
            self.proof_exit_armed = true;
        }

        if self.proof_mode && self.switches >= PROOF_FINISH_SWITCH {
            return ptr::null_mut();
        }

        let Ok(next) = self.dispatch.on_tick() else {
            return ptr::null_mut();
        };
        self.mark_post_exit_dispatch(next);
        self.context_ptr(next)
    }

    pub fn exit_from_ring3(&mut self) -> *mut CpuContext {
        if self.proof_mode && self.proof_exit_armed {
            self.proof_exit_handoff = true;
        }
        self.exit_current()
    }

    fn reap_retired(&mut self) {
        let (current, _) = Cr3::read();
        let current = current.start_address().as_u64();
        let mut reclaimed = 0;
        for slot in &mut self.retired {
            if slot
                .as_ref()
                .is_some_and(|owner| owner.level4_address() != current)
            {
                reclaimed += slot.take().unwrap().reclaim();
            }
        }
        self.reclaimed_frames += reclaimed;
    }

    pub fn reclaimed_frames(&self) -> usize {
        self.reclaimed_frames
    }

    pub fn retired_level4(&self) -> u64 {
        self.retired_level4
    }

    pub fn reclaim_all(&mut self) -> usize {
        self.reap_retired();
        let (current, _) = Cr3::read();
        let current = current.start_address().as_u64();
        let mut reclaimed = 0;
        for slot in &mut self.tasks {
            let Some(task) = slot.as_mut() else {
                continue;
            };
            if task
                .address_space
                .as_ref()
                .is_some_and(|owner| owner.level4_address() != current)
            {
                reclaimed += task.address_space.take().unwrap().reclaim();
            }
        }
        self.reclaimed_frames += reclaimed;
        reclaimed
    }

    pub fn proof_passed(&self) -> bool {
        let mut processes = 0u64;
        for task in self.tasks.iter().flatten() {
            if task.process.0 < 64 {
                processes |= 1u64 << task.process.0;
            }
        }
        self.proof_mode
            && self.installed == 3
            && processes.count_ones() == 3
            && self.switches >= PROOF_FINISH_SWITCH
            && self.proof_exit_armed
            && self.proof_exit_handoff
            && self.post_exit_threads.count_ones() >= 2
            && self
                .tasks
                .iter()
                .flatten()
                .filter(|task| task.alive)
                .count()
                == 2
    }
}

struct SchedulerCell(UnsafeCell<KernelScheduler>);

// Timer and syscall gates enter with maskable interrupts disabled. Nova is
// single-core at this milestone, so kernel entry serializes all mutations.
unsafe impl Sync for SchedulerCell {}

static SCHEDULER: SchedulerCell = SchedulerCell(UnsafeCell::new(KernelScheduler::new()));

unsafe extern "C" {
    static mut nova_scheduler_active: u8;
    static mut nova_scheduler_current_context: *mut CpuContext;
    static mut nova_scheduler_current_fx: *mut u8;
}

fn scheduler() -> &'static mut KernelScheduler {
    unsafe { &mut *SCHEDULER.0.get() }
}

pub fn reset(proof_mode: bool) -> bool {
    if !scheduler().reset(proof_mode) {
        return false;
    }
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_active), 0);
        ptr::write_volatile(
            ptr::addr_of_mut!(nova_scheduler_current_context),
            ptr::null_mut(),
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(nova_scheduler_current_fx),
            ptr::null_mut(),
        );
    }
    true
}

pub fn install(
    process: ProcessId,
    thread: ThreadId,
    context: CpuContext,
    address_space: AddressSpaceOwner,
) -> bool {
    scheduler().install(process, thread, context, address_space)
}

pub fn start() -> *mut CpuContext {
    let context = scheduler().start();
    let extended = scheduler().current_extended_state_ptr();
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_current_context), context);
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_current_fx), extended);
        ptr::write_volatile(
            ptr::addr_of_mut!(nova_scheduler_active),
            (!context.is_null()) as u8,
        );
    }
    context
}

pub fn stop() {
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_active), 0);
        ptr::write_volatile(
            ptr::addr_of_mut!(nova_scheduler_current_context),
            ptr::null_mut(),
        );
        ptr::write_volatile(
            ptr::addr_of_mut!(nova_scheduler_current_fx),
            ptr::null_mut(),
        );
    }
}

pub fn proof_passed() -> bool {
    scheduler().proof_passed()
}

pub fn reclaimed_frames() -> usize {
    scheduler().reclaimed_frames()
}

pub fn retired_level4() -> u64 {
    scheduler().retired_level4()
}

pub fn reclaim_all() -> usize {
    scheduler().reclaim_all()
}

pub fn exit_current() -> *mut CpuContext {
    let context = scheduler().exit_from_ring3();
    let extended = scheduler().current_extended_state_ptr();
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_current_context), context);
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_current_fx), extended);
        if context.is_null() {
            ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_active), 0);
        }
    }
    context
}

#[unsafe(no_mangle)]
extern "C" fn nova_scheduler_on_timer() -> *mut CpuContext {
    let context = scheduler().on_tick();
    let extended = scheduler().current_extended_state_ptr();
    unsafe {
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_current_context), context);
        ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_current_fx), extended);
        if context.is_null() {
            ptr::write_volatile(ptr::addr_of_mut!(nova_scheduler_active), 0);
        }
    }
    context
}

#[unsafe(no_mangle)]
extern "C" fn nova_scheduler_exit_current() -> *mut CpuContext {
    exit_current()
}
