use core::arch::asm;
use core::fmt::{self, Write};

const COM1: u16 = 0x3f8;

pub fn init() {
    unsafe {
        out(COM1 + 1, 0x00);
        out(COM1 + 3, 0x80);
        out(COM1, 0x03);
        out(COM1 + 1, 0x00);
        out(COM1 + 3, 0x03);
        out(COM1 + 2, 0xc7);
        out(COM1 + 4, 0x0b);
    }
}

pub fn write_str(text: &str) {
    for byte in text.bytes() {
        while unsafe { input(COM1 + 5) } & 0x20 == 0 {
            core::hint::spin_loop();
        }
        unsafe {
            out(COM1, byte);
        }
    }
}

pub fn write_fmt(args: fmt::Arguments<'_>) {
    let _ = SerialWriter.write_fmt(args);
}

struct SerialWriter;

impl Write for SerialWriter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        write_str(text);
        Ok(())
    }
}

unsafe fn out(port: u16, value: u8) {
    unsafe {
        asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
    }
}
unsafe fn input(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack));
    }
    value
}
