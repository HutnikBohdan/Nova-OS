use core::arch::asm;

static mut STATE: MouseState = MouseState {
    packet: [0; 3],
    index: 0,
    x: 512,
    y: 384,
    max_x: 1023,
    max_y: 767,
    left: false,
    ready: false,
};

struct MouseState {
    packet: [u8; 3],
    index: usize,
    x: i32,
    y: i32,
    max_x: i32,
    max_y: i32,
    left: bool,
    ready: bool,
}

pub struct MouseEvent {
    pub x: usize,
    pub y: usize,
    pub left_pressed: bool,
}

pub fn init(width: usize, height: usize) {
    unsafe {
        STATE.max_x = width.saturating_sub(1) as i32;
        STATE.max_y = height.saturating_sub(1) as i32;
        STATE.x = (width / 2) as i32;
        STATE.y = (height / 2) as i32;
    }
    if !write_command(0xa8) {
        return;
    }
    let defaults = mouse_command(0xf6);
    let enabled = mouse_command(0xf4);
    unsafe { STATE.ready = defaults && enabled };
}

pub fn poll() -> Option<MouseEvent> {
    let status = unsafe { input(0x64) };
    if status & 0x21 != 0x21 || unsafe { !STATE.ready } {
        return None;
    }
    let byte = unsafe { input(0x60) };
    unsafe {
        if STATE.index == 0 && byte & 0x08 == 0 {
            return None;
        }
        let index = STATE.index;
        STATE.packet[index] = byte;
        STATE.index += 1;
        if STATE.index < 3 {
            return None;
        }
        STATE.index = 0;
        let flags = STATE.packet[0];
        if flags & 0xc0 != 0 {
            return None;
        }
        STATE.x = (STATE.x + STATE.packet[1] as i8 as i32).clamp(0, STATE.max_x);
        STATE.y = (STATE.y - STATE.packet[2] as i8 as i32).clamp(0, STATE.max_y);
        let left = flags & 1 != 0;
        let left_pressed = left && !STATE.left;
        STATE.left = left;
        Some(MouseEvent {
            x: STATE.x as usize,
            y: STATE.y as usize,
            left_pressed,
        })
    }
}

fn mouse_command(command: u8) -> bool {
    if !write_command(0xd4) || !write_data(command) || !wait_output() {
        return false;
    }
    unsafe { input(0x60) == 0xfa }
}

fn write_command(value: u8) -> bool {
    if !wait_input() {
        return false;
    }
    unsafe { output(0x64, value) };
    true
}

fn write_data(value: u8) -> bool {
    if !wait_input() {
        return false;
    }
    unsafe { output(0x60, value) };
    true
}

fn wait_input() -> bool {
    for _ in 0..100_000 {
        if unsafe { input(0x64) } & 2 == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn wait_output() -> bool {
    for _ in 0..100_000 {
        if unsafe { input(0x64) } & 1 != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

unsafe fn input(port: u16) -> u8 {
    let value: u8;
    unsafe { asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack)) };
    value
}

unsafe fn output(port: u16, value: u8) {
    unsafe { asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack)) };
}
