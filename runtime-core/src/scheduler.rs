use crate::process::ThreadId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerError {
    QueueFull,
    AlreadyQueued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchError {
    Queue(SchedulerError),
    NoRunnableThread,
}

impl From<SchedulerError> for DispatchError {
    fn from(value: SchedulerError) -> Self {
        Self::Queue(value)
    }
}

/// Allocation-free FIFO run queue. A thread is appended after its time slice,
/// producing deterministic round-robin scheduling.
pub struct RoundRobin<const N: usize> {
    queue: [Option<ThreadId>; N],
    head: usize,
    len: usize,
}

impl<const N: usize> RoundRobin<N> {
    pub const fn new() -> Self {
        Self {
            queue: [None; N],
            head: 0,
            len: 0,
        }
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, id: ThreadId) -> bool {
        (0..self.len).any(|offset| self.queue[(self.head + offset) % N] == Some(id))
    }

    pub fn enqueue(&mut self, id: ThreadId) -> Result<(), SchedulerError> {
        if self.contains(id) {
            return Err(SchedulerError::AlreadyQueued);
        }
        if self.len == N {
            return Err(SchedulerError::QueueFull);
        }
        if N == 0 {
            return Err(SchedulerError::QueueFull);
        }
        let tail = (self.head + self.len) % N;
        self.queue[tail] = Some(id);
        self.len += 1;
        Ok(())
    }

    pub fn next(&mut self) -> Option<ThreadId> {
        if self.len == 0 || N == 0 {
            return None;
        }
        let id = self.queue[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        id
    }

    pub fn reschedule(&mut self, id: ThreadId) -> Result<Option<ThreadId>, SchedulerError> {
        self.enqueue(id)?;
        Ok(self.next())
    }

    pub fn remove(&mut self, id: ThreadId) -> bool {
        if self.len == 0 || N == 0 {
            return false;
        }
        let mut found = false;
        let original = self.len;
        for _ in 0..original {
            let current = self.next().unwrap();
            if current == id {
                found = true;
            } else {
                let _ = self.enqueue(current);
            }
        }
        found
    }
}

impl<const N: usize> Default for RoundRobin<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Architecture-neutral runnable-thread state used by the kernel dispatcher.
/// Context storage remains architecture-specific; this type owns only ordering
/// and the identity of the currently running thread.
pub struct SchedulerCore<const N: usize> {
    ready: RoundRobin<N>,
    current: Option<ThreadId>,
}

impl<const N: usize> SchedulerCore<N> {
    pub const fn new() -> Self {
        Self {
            ready: RoundRobin::new(),
            current: None,
        }
    }

    pub const fn current(&self) -> Option<ThreadId> {
        self.current
    }

    pub const fn ready_len(&self) -> usize {
        self.ready.len()
    }

    pub fn install(&mut self, thread: ThreadId) -> Result<(), DispatchError> {
        if self.current == Some(thread) {
            return Err(DispatchError::Queue(SchedulerError::AlreadyQueued));
        }
        self.ready.enqueue(thread)?;
        Ok(())
    }

    pub fn start(&mut self) -> Result<ThreadId, DispatchError> {
        if let Some(current) = self.current {
            return Ok(current);
        }
        let next = self.ready.next().ok_or(DispatchError::NoRunnableThread)?;
        self.current = Some(next);
        Ok(next)
    }

    pub fn on_tick(&mut self) -> Result<ThreadId, DispatchError> {
        let current = self.current.take().ok_or(DispatchError::NoRunnableThread)?;
        self.ready.enqueue(current)?;
        let next = self.ready.next().ok_or(DispatchError::NoRunnableThread)?;
        self.current = Some(next);
        Ok(next)
    }

    /// Removes the running thread and dispatches its successor. The exited
    /// thread is deliberately never reinserted into the ready queue.
    pub fn exit_current(&mut self) -> Result<(ThreadId, Option<ThreadId>), DispatchError> {
        let exited = self.current.take().ok_or(DispatchError::NoRunnableThread)?;
        let next = self.ready.next();
        self.current = next;
        Ok((exited, next))
    }

    pub fn remove(&mut self, thread: ThreadId) -> bool {
        if self.current == Some(thread) {
            self.current = None;
            true
        } else {
            self.ready.remove(thread)
        }
    }
}

impl<const N: usize> Default for SchedulerCore<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    SleepUntil(u64),
    ChannelReceive(u64),
    ObjectSignal(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockedThread {
    pub thread: ThreadId,
    pub reason: WaitReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    Full,
    AlreadyBlocked,
}

pub struct BlockedSet<const N: usize> {
    entries: [Option<BlockedThread>; N],
}

impl<const N: usize> BlockedSet<N> {
    pub const fn new() -> Self {
        Self { entries: [None; N] }
    }

    pub fn block(&mut self, thread: ThreadId, reason: WaitReason) -> Result<(), BlockError> {
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| entry.thread == thread)
        {
            return Err(BlockError::AlreadyBlocked);
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(BlockError::Full)?;
        *slot = Some(BlockedThread { thread, reason });
        Ok(())
    }

    pub fn wake(&mut self, thread: ThreadId) -> Option<BlockedThread> {
        self.entries
            .iter_mut()
            .find(|entry| entry.is_some_and(|value| value.thread == thread))?
            .take()
    }

    pub fn wake_expired(&mut self, now: u64, mut ready: impl FnMut(ThreadId)) -> usize {
        let mut count = 0;
        for entry in &mut self.entries {
            if entry.is_some_and(
                |value| matches!(value.reason, WaitReason::SleepUntil(deadline) if deadline <= now),
            ) {
                let value = entry.take().unwrap();
                ready(value.thread);
                count += 1;
            }
        }
        count
    }
}

impl<const N: usize> Default for BlockedSet<N> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Quantum {
    ticks_per_slice: u32,
    remaining: u32,
}

impl Quantum {
    pub const fn new(ticks_per_slice: u32) -> Self {
        let value = if ticks_per_slice == 0 {
            1
        } else {
            ticks_per_slice
        };
        Self {
            ticks_per_slice: value,
            remaining: value,
        }
    }
    pub fn tick(&mut self) -> bool {
        self.remaining -= 1;
        if self.remaining == 0 {
            self.remaining = self.ticks_per_slice;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_in_round_robin_order() {
        let mut queue = RoundRobin::<3>::new();
        queue.enqueue(ThreadId(1)).unwrap();
        queue.enqueue(ThreadId(2)).unwrap();
        assert_eq!(queue.reschedule(ThreadId(3)).unwrap(), Some(ThreadId(1)));
        queue.enqueue(ThreadId(1)).unwrap();
        assert_eq!(queue.next(), Some(ThreadId(2)));
        assert_eq!(queue.next(), Some(ThreadId(3)));
        assert_eq!(queue.next(), Some(ThreadId(1)));
    }

    #[test]
    fn removes_blocked_thread() {
        let mut queue = RoundRobin::<4>::new();
        for id in 1..=3 {
            queue.enqueue(ThreadId(id)).unwrap();
        }
        assert!(queue.remove(ThreadId(2)));
        assert_eq!(queue.next(), Some(ThreadId(1)));
        assert_eq!(queue.next(), Some(ThreadId(3)));
    }

    #[test]
    fn wakes_only_expired_sleepers() {
        let mut blocked = BlockedSet::<3>::new();
        blocked
            .block(ThreadId(1), WaitReason::SleepUntil(10))
            .unwrap();
        blocked
            .block(ThreadId(2), WaitReason::SleepUntil(20))
            .unwrap();
        let mut ready = [ThreadId(0); 2];
        let mut count = 0;
        assert_eq!(
            blocked.wake_expired(10, |thread| {
                ready[count] = thread;
                count += 1;
            }),
            1
        );
        assert_eq!(ready[0], ThreadId(1));
        assert!(blocked.wake(ThreadId(2)).is_some());
    }

    #[test]
    fn quantum_requests_preemption_at_boundary() {
        let mut quantum = Quantum::new(3);
        assert!(!quantum.tick());
        assert!(!quantum.tick());
        assert!(quantum.tick());
        assert!(!quantum.tick());
    }

    #[test]
    fn scheduler_core_rotates_three_threads() {
        let mut scheduler = SchedulerCore::<3>::new();
        for id in 1..=3 {
            scheduler.install(ThreadId(id)).unwrap();
        }
        assert_eq!(scheduler.start().unwrap(), ThreadId(1));
        assert_eq!(scheduler.on_tick().unwrap(), ThreadId(2));
        assert_eq!(scheduler.on_tick().unwrap(), ThreadId(3));
        assert_eq!(scheduler.on_tick().unwrap(), ThreadId(1));
    }

    #[test]
    fn scheduler_core_hands_off_after_exit() {
        let mut scheduler = SchedulerCore::<3>::new();
        for id in 1..=3 {
            scheduler.install(ThreadId(id)).unwrap();
        }
        assert_eq!(scheduler.start().unwrap(), ThreadId(1));
        assert_eq!(scheduler.on_tick().unwrap(), ThreadId(2));
        assert_eq!(
            scheduler.exit_current().unwrap(),
            (ThreadId(2), Some(ThreadId(3)))
        );
        assert_eq!(scheduler.on_tick().unwrap(), ThreadId(1));
        assert_eq!(scheduler.on_tick().unwrap(), ThreadId(3));
        assert_ne!(scheduler.current(), Some(ThreadId(2)));
    }
}
