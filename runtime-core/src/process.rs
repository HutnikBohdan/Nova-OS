#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessId(pub u64);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ThreadId(pub u64);

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode(pub i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashReason {
    InvalidOpcode,
    PageFault,
    ProtectionFault,
    Killed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Created,
    Running,
    Exiting,
    Exited(ExitCode),
    Crashed(CrashReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Created,
    Ready,
    Running,
    Blocked,
    Exited(ExitCode),
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CpuContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub instruction_pointer: u64,
    pub code_segment: u64,
    pub flags: u64,
    pub stack_pointer: u64,
    pub stack_segment: u64,
    pub cr3: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Process {
    pub id: ProcessId,
    pub parent: Option<ProcessId>,
    pub state: ProcessState,
    pub address_space: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thread {
    pub id: ThreadId,
    pub process: ProcessId,
    pub state: ThreadState,
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub context: CpuContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessError {
    TableFull,
    NotFound,
    InvalidTransition,
    ProcessNotRunning,
}

pub struct ProcessTable<const N: usize> {
    slots: [Option<Process>; N],
    next_id: u64,
}

impl<const N: usize> ProcessTable<N> {
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            next_id: 1,
        }
    }

    pub fn create(
        &mut self,
        parent: Option<ProcessId>,
        address_space: u64,
    ) -> Result<ProcessId, ProcessError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ProcessError::TableFull)?;
        let id = ProcessId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        *slot = Some(Process {
            id,
            parent,
            state: ProcessState::Created,
            address_space,
        });
        Ok(id)
    }

    pub fn get(&self, id: ProcessId) -> Option<&Process> {
        self.slots.iter().flatten().find(|process| process.id == id)
    }

    pub fn get_mut(&mut self, id: ProcessId) -> Option<&mut Process> {
        self.slots
            .iter_mut()
            .flatten()
            .find(|process| process.id == id)
    }

    pub fn start(&mut self, id: ProcessId) -> Result<(), ProcessError> {
        let process = self.get_mut(id).ok_or(ProcessError::NotFound)?;
        if process.state != ProcessState::Created {
            return Err(ProcessError::InvalidTransition);
        }
        process.state = ProcessState::Running;
        Ok(())
    }

    pub fn begin_exit(&mut self, id: ProcessId) -> Result<(), ProcessError> {
        let process = self.get_mut(id).ok_or(ProcessError::NotFound)?;
        if process.state != ProcessState::Running {
            return Err(ProcessError::InvalidTransition);
        }
        process.state = ProcessState::Exiting;
        Ok(())
    }

    pub fn finish_exit(&mut self, id: ProcessId, code: ExitCode) -> Result<(), ProcessError> {
        let process = self.get_mut(id).ok_or(ProcessError::NotFound)?;
        if process.state != ProcessState::Exiting {
            return Err(ProcessError::InvalidTransition);
        }
        process.state = ProcessState::Exited(code);
        Ok(())
    }

    pub fn crash(&mut self, id: ProcessId, reason: CrashReason) -> Result<(), ProcessError> {
        let process = self.get_mut(id).ok_or(ProcessError::NotFound)?;
        if !matches!(
            process.state,
            ProcessState::Created | ProcessState::Running | ProcessState::Exiting
        ) {
            return Err(ProcessError::InvalidTransition);
        }
        process.state = ProcessState::Crashed(reason);
        Ok(())
    }

    pub fn wait_result(
        &self,
        parent: ProcessId,
        child: ProcessId,
    ) -> Result<Option<ExitCode>, ProcessError> {
        let process = self.get(child).ok_or(ProcessError::NotFound)?;
        if process.parent != Some(parent) {
            return Err(ProcessError::InvalidTransition);
        }
        Ok(match process.state {
            ProcessState::Exited(code) => Some(code),
            ProcessState::Crashed(_) => Some(ExitCode(-1)),
            _ => None,
        })
    }

    pub fn reap(&mut self, id: ProcessId) -> Result<Process, ProcessError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|p| p.id == id))
            .ok_or(ProcessError::NotFound)?;
        if !matches!(
            slot.as_ref().unwrap().state,
            ProcessState::Exited(_) | ProcessState::Crashed(_)
        ) {
            return Err(ProcessError::InvalidTransition);
        }
        Ok(slot.take().unwrap())
    }
}

impl<const N: usize> Default for ProcessTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ThreadTable<const N: usize> {
    slots: [Option<Thread>; N],
    next_id: u64,
}

impl<const N: usize> ThreadTable<N> {
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            next_id: 1,
        }
    }

    pub fn create(
        &mut self,
        process: &Process,
        ip: u64,
        sp: u64,
    ) -> Result<ThreadId, ProcessError> {
        if process.state != ProcessState::Running {
            return Err(ProcessError::ProcessNotRunning);
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ProcessError::TableFull)?;
        let id = ThreadId(self.next_id);
        self.next_id = self.next_id.wrapping_add(1).max(1);
        *slot = Some(Thread {
            id,
            process: process.id,
            state: ThreadState::Created,
            instruction_pointer: ip,
            stack_pointer: sp,
            context: CpuContext {
                instruction_pointer: ip,
                stack_pointer: sp,
                cr3: process.address_space,
                flags: 0x202,
                ..CpuContext::default()
            },
        });
        Ok(id)
    }

    pub fn get(&self, id: ThreadId) -> Option<&Thread> {
        self.slots.iter().flatten().find(|thread| thread.id == id)
    }
    pub fn get_mut(&mut self, id: ThreadId) -> Option<&mut Thread> {
        self.slots
            .iter_mut()
            .flatten()
            .find(|thread| thread.id == id)
    }

    pub fn transition(&mut self, id: ThreadId, next: ThreadState) -> Result<(), ProcessError> {
        let thread = self.get_mut(id).ok_or(ProcessError::NotFound)?;
        let valid = matches!(
            (thread.state, next),
            (ThreadState::Created, ThreadState::Ready)
                | (ThreadState::Ready, ThreadState::Running)
                | (ThreadState::Running, ThreadState::Ready)
                | (ThreadState::Running, ThreadState::Blocked)
                | (ThreadState::Blocked, ThreadState::Ready)
                | (ThreadState::Created, ThreadState::Exited(_))
                | (ThreadState::Ready, ThreadState::Exited(_))
                | (ThreadState::Running, ThreadState::Exited(_))
                | (ThreadState::Blocked, ThreadState::Exited(_))
        );
        if !valid {
            return Err(ProcessError::InvalidTransition);
        }
        thread.state = next;
        Ok(())
    }

    pub fn remove(&mut self, id: ThreadId) -> Result<Thread, ProcessError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.as_ref().is_some_and(|thread| thread.id == id))
            .ok_or(ProcessError::NotFound)?;
        if !matches!(slot.as_ref().unwrap().state, ThreadState::Exited(_)) {
            return Err(ProcessError::InvalidTransition);
        }
        Ok(slot.take().unwrap())
    }
}

impl<const N: usize> Default for ThreadTable<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_process_lifecycle() {
        let mut processes = ProcessTable::<2>::new();
        let id = processes.create(None, 0x1000).unwrap();
        assert_eq!(
            processes.begin_exit(id),
            Err(ProcessError::InvalidTransition)
        );
        processes.start(id).unwrap();
        processes.begin_exit(id).unwrap();
        processes.finish_exit(id, ExitCode(0)).unwrap();
        assert_eq!(processes.reap(id).unwrap().id, id);
    }

    #[test]
    fn thread_requires_running_process() {
        let mut processes = ProcessTable::<1>::new();
        let id = processes.create(None, 1).unwrap();
        let mut threads = ThreadTable::<1>::new();
        assert_eq!(
            threads.create(processes.get(id).unwrap(), 10, 20),
            Err(ProcessError::ProcessNotRunning)
        );
        processes.start(id).unwrap();
        let thread = threads.create(processes.get(id).unwrap(), 10, 20).unwrap();
        threads.transition(thread, ThreadState::Ready).unwrap();
        threads.transition(thread, ThreadState::Running).unwrap();
    }

    #[test]
    fn supervisor_observes_child_crash_and_reaps_it() {
        let mut processes = ProcessTable::<3>::new();
        let init = processes.create(None, 1).unwrap();
        processes.start(init).unwrap();
        let child = processes.create(Some(init), 2).unwrap();
        processes.start(child).unwrap();
        assert_eq!(processes.wait_result(init, child).unwrap(), None);
        processes.crash(child, CrashReason::PageFault).unwrap();
        assert_eq!(
            processes.wait_result(init, child).unwrap(),
            Some(ExitCode(-1))
        );
        assert!(matches!(
            processes.reap(child).unwrap().state,
            ProcessState::Crashed(CrashReason::PageFault)
        ));
    }

    #[test]
    fn exited_thread_can_be_removed_but_ready_thread_cannot() {
        let mut processes = ProcessTable::<1>::new();
        let process = processes.create(None, 1).unwrap();
        processes.start(process).unwrap();
        let mut threads = ThreadTable::<1>::new();
        let thread = threads
            .create(processes.get(process).unwrap(), 10, 20)
            .unwrap();
        threads.transition(thread, ThreadState::Ready).unwrap();
        assert_eq!(threads.remove(thread), Err(ProcessError::InvalidTransition));
        threads
            .transition(thread, ThreadState::Exited(ExitCode(0)))
            .unwrap();
        assert_eq!(threads.remove(thread).unwrap().id, thread);
        assert!(threads.get(thread).is_none());
    }
}
