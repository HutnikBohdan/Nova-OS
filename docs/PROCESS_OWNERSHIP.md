# Process and address-space ownership

This document defines Nova's kernel ownership invariants. **Implemented** means
the invariant is enforced by the current kernel path and its QEMU gates.
**Planned** means required architecture that must not yet be treated as a
release guarantee.

## Implemented now

- **Slim scheduler records.** A scheduled task keeps identifiers, CPU context,
  lifecycle state, and one kernel-owned address-space owner. User mappings and
  page-table contents are not copied into the process/thread records.
- **Move-only address-space ownership.** `AddressSpaceOwner` is a private,
  non-`Copy` kernel value. It may move from the address-space builder into the
  scheduler, but cannot be exported to userspace, sent over IPC, duplicated, or
  transferred between processes.
- **Private-frame ledger.** Every frame allocated for a user address space is
  recorded in its fixed-capacity ledger, including the PML4, private
  intermediate page tables, code, exchange, and stack pages. Shared kernel
  mappings copied into the upper PML4 are not recorded and are never reclaimed
  as process-private frames.
- **Deferred destruction.** `ProcessExit` first detaches the task from runnable
  scheduling and moves its owner to the retire queue. The syscall path does not
  free the page tables named by the current CR3.
- **Active-CR3 guard.** The reaper releases an owner only after execution has
  switched to another CR3. Bulk cleanup and scheduler reset also refuse to
  reclaim an owner whose PML4 is active. Reset fails closed rather than
  overwriting unreclaimed ownership state.
- **Allocation-free teardown.** Frame ownership and the retire queue use fixed
  capacity storage. Reclamation returns recorded frames to the reusable kernel
  pool without allocating memory.
- **Current concurrency boundary.** Frame allocation, return, scheduling, and
  reaping rely on the current single-core, interrupt-serialized kernel entry
  model. They are not yet safe for concurrent SMP mutation.

The BIOS QEMU gates `NOVA_PROCESS_EXIT_FRAMES_RECLAIMED_OK` and
`NOVA_ADDRESS_SPACE_FRAME_REUSE_OK` prove deferred exit reclamation and reuse of
a returned address-space frame. They do not prove the planned shared-object or
SMP designs below.

## Planned invariants

- **Reference-only PCB.** The production PCB will contain handles/references to
  address spaces, capability tables, threads, and shared VM objects—not inline
  variable-size mapping or object data.
- **Shared `VmObject` table.** Shared memory, file-backed mappings, and similar
  objects will live in a global kernel object table. A process mapping holds an
  attenuated object reference plus mapping permissions and range; private page
  ledgers must never reclaim globally owned shared frames. Object frames are
  released only when the global reference count reaches zero.
- **Reserved teardown capacity.** Process creation will reserve all PCB,
  mapping-node, capability-node, and retire-record slab capacity needed for
  rollback and destruction. Exit, crash, kill, and low-memory teardown must not
  depend on a fallible heap allocation.
- **`Dying` detach phases.** Termination will be explicit and idempotent:
  `Running -> Dying`, remove from run/wait/IPC queues, reject new handles and
  mappings, detach capabilities and shared-object references, switch away from
  the address space, quarantine/reap private frames, then publish the terminal
  state for `wait` and final PCB reaping.
- **Per-CPU retirement.** SMP will use per-CPU retire/quarantine queues and
  synchronization for the global frame/object tables. An address space may be
  reclaimed only after no CPU is executing it and required TLB shootdowns have
  completed.
- **Capability attenuation.** Derived or transferred references may only lose
  rights. Mapping teardown, object lookup, process restart, supervisor recovery,
  and handle-slot reuse must never silently restore rights that were absent or
  revoked. Generation checks remain mandatory for stale handles.
- **Page-fault policy.** A user fault will be classified as recoverable demand
  paging/COW or fatal policy violation. Recoverable faults may resolve only
  through an authorized mapping and `VmObject`; invalid, protection, executable,
  kernel-address, or unresolved faults transition the process to `Dying` and
  notify its supervisor. Kernel faults remain kernel-fatal. Fault handling must
  not allocate from unreserved memory while locks or teardown ownership are
  held.

## Non-invariants today

Nova does not yet provide shared `VmObject` lifetime management, demand paging,
copy-on-write, an SMP-safe frame allocator, TLB shootdown, per-CPU quarantine,
or the complete `Dying`/wait/supervisor teardown protocol. Those features require
their own tests and QEMU acceptance markers before documentation may label them
implemented.
