use virtio_core::{block::*, dma::*, input::*, interrupt::*, net::*, queue::*, transport::*};

#[test]
fn pci_modern_capabilities_are_parsed_and_loops_rejected() {
    let mut cfg = [0u8; 256];
    cfg[0x40..0x54].copy_from_slice(&[
        9,
        0x60,
        20,
        VIRTIO_PCI_CAP_NOTIFY_CFG,
        2,
        7,
        0,
        0,
        0x00,
        0x10,
        0,
        0,
        0x00,
        0x10,
        0,
        0,
        4,
        0,
        0,
        0,
    ]);
    cfg[0x60..0x70].copy_from_slice(&[
        9,
        0,
        16,
        VIRTIO_PCI_CAP_COMMON_CFG,
        1,
        0,
        0,
        0,
        0,
        0x20,
        0,
        0,
        0x38,
        0,
        0,
        0,
    ]);
    let caps = CapabilitySet::parse(&cfg, 0x40).unwrap();
    assert_eq!(caps.common.unwrap().offset, 0x2000);
    assert_eq!(caps.notify.unwrap().notify_multiplier, Some(4));
    cfg[0x61] = 0x40;
    assert_eq!(
        CapabilitySet::parse(&cfg, 0x40),
        Err(TransportError::CapabilityLoop)
    );
}

#[test]
fn config_and_notify_offsets_are_checked() {
    let window = ConfigWindow::new(0x1000, 0x100);
    assert_eq!(notify_address(window, 4, 3), Ok(0x100c));
    assert_eq!(
        window.at(0xff, 2),
        Err(TransportError::CapabilityOutOfRange)
    );
}

#[test]
fn split_and_packed_dma_plans_respect_alignment() {
    let split = plan_split(0x1003, 0x10000, 128, true).unwrap();
    assert_eq!(split.descriptor.address % 16, 0);
    assert_eq!(split.available.address % 2, 0);
    assert_eq!(split.used.address % 4, 0);
    let packed = plan_packed(0x2001, 0x10000, 64).unwrap();
    assert_eq!(packed.descriptor.address % 16, 0);
    assert!(plan_split(u64::MAX - 2, 4096, 8, false).is_err());
}

#[test]
fn split_queue_owns_chain_until_completion() {
    let mut queue = SplitQueue::<8>::new();
    let token = queue
        .submit(&[
            Descriptor {
                address: 0x1000,
                length: 16,
                flags: 0,
                next: 0,
            },
            Descriptor {
                address: 0x2000,
                length: 512,
                flags: DESC_F_WRITE,
                next: 0,
            },
        ])
        .unwrap();
    assert_eq!(queue.device_take().unwrap(), token);
    queue.device_complete(token, 512).unwrap();
    assert_eq!(queue.pop_used().unwrap(), (token, 512));
    assert_eq!(queue.device_complete(token, 0), Err(QueueError::StaleToken));
}

#[test]
fn packed_queue_wraps_and_rejects_stale_generation() {
    let mut queue = PackedQueue::<2>::new();
    let (a, _) = queue.submit_one().unwrap();
    let (b, _) = queue.submit_one().unwrap();
    queue.complete_one(a).unwrap();
    queue.reclaim_one(a).unwrap();
    queue.complete_one(b).unwrap();
    queue.reclaim_one(b).unwrap();
    let (next, _) = queue.submit_one().unwrap();
    assert_ne!(a.generation, next.generation);
    assert_eq!(queue.complete_one(a), Err(QueueError::StaleToken));
}

#[test]
fn block_lifecycle_covers_flush_discard_and_timeout() {
    let mut table = RequestTable::<4>::new();
    let flush = table.submit(Operation::Flush, 10, 5, 1024, 512).unwrap();
    table.complete(flush, S_OK, 0).unwrap();
    assert_eq!(table.poll(flush, 11), Ok(Some(Ok(0))));
    table.release(flush).unwrap();
    let range = DiscardRange {
        sector: 8,
        sectors: 4,
        flags: 0,
    };
    let discard = table
        .submit(Operation::Discard(range), 20, 5, 1024, 512)
        .unwrap();
    assert_eq!(header(Operation::Discard(range)).request_type, T_DISCARD);
    assert_eq!(table.poll(discard, 25), Ok(Some(Err(BlockError::TimedOut))));
    assert_eq!(
        table.submit(
            Operation::Read {
                sector: 1024,
                bytes: 512
            },
            0,
            1,
            1024,
            512
        ),
        Err(BlockError::OutOfRange)
    );
}

#[test]
fn network_rx_tx_have_bounded_lifecycles() {
    let mut queue = PacketQueue::<2>::new();
    let rx = queue.provide_rx(1514).unwrap();
    queue.complete(rx, 100).unwrap();
    assert_eq!(queue.poll(rx, 0), Ok(Some(100)));
    queue.release(rx).unwrap();
    let tx = queue.submit_tx(100, 1500, 10, 2).unwrap();
    assert_eq!(queue.poll(tx, 12), Err(NetError::TimedOut));
    assert_eq!(
        queue.submit_tx(1501, 1500, 0, 1),
        Err(NetError::FrameTooLarge)
    );
}

#[test]
fn input_frames_and_overflow_are_explicit() {
    let mut events = EventBuffer::<2>::new();
    events
        .push(InputEvent {
            event_type: EV_KEY,
            code: 30,
            value: 1,
        })
        .unwrap();
    assert!(!events.frame_ready());
    events
        .push(InputEvent {
            event_type: EV_SYN,
            code: SYN_REPORT,
            value: 0,
        })
        .unwrap();
    assert!(events.frame_ready());
    assert_eq!(events.push(InputEvent::default()), Err(InputError::Full));
    assert_eq!(events.dropped(), 1);
}

#[test]
fn interrupt_coalescing_and_reset_timeout_are_deterministic() {
    assert_eq!(decode_isr(3), InterruptReason::QueueAndConfiguration);
    let mut coalescer = Coalescer::new(3, 10);
    assert!(!coalescer.record(5));
    assert!(!coalescer.record(6));
    assert!(coalescer.record(7));
    assert_eq!(coalescer.acknowledge(), 3);
    let mut lifecycle = Lifecycle::new();
    lifecycle.begin(0, 5).unwrap();
    assert_eq!(lifecycle.tick(5), Err(LifecycleError::TimedOut));
    assert_eq!(lifecycle.state(), DeviceState::Failed);
    lifecycle.reset_complete().unwrap();
    assert_eq!(lifecycle.state(), DeviceState::Reset);
}
