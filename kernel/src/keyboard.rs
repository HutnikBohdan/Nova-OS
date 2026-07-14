use core::arch::asm;

const NORMAL: &[u8] = b"\0\x1b1234567890-=\x08\tqwertyuiop[]\n\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 \0";
const SHIFTED: &[u8] =
    b"\0\x1b!@#$%^&*()_+\x08\tQWERTYUIOP{}\n\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 \0";

static mut SHIFT: bool = false;
static mut UKRAINIAN: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Escape,
    Tab,
    Function(u8),
}

pub fn read_key() -> Option<Key> {
    let status = unsafe { input(0x64) };
    if status & 1 == 0 || status & 0x20 != 0 {
        return None;
    }
    let scan = unsafe { input(0x60) };
    match scan {
        0x2a | 0x36 => {
            unsafe { SHIFT = true };
            return None;
        }
        0xaa | 0xb6 => {
            unsafe { SHIFT = false };
            return None;
        }
        0x3b..=0x44 => return Some(Key::Function(scan - 0x3a)),
        _ if scan & 0x80 != 0 => return None,
        _ => {}
    }
    let byte = unsafe {
        if SHIFT {
            SHIFTED.get(scan as usize)
        } else {
            NORMAL.get(scan as usize)
        }
    }
    .copied()?;
    match byte {
        0 => None,
        b'\n' => Some(Key::Enter),
        8 => Some(Key::Backspace),
        0x1b => Some(Key::Escape),
        b'\t' => Some(Key::Tab),
        value => {
            let ukrainian = unsafe { UKRAINIAN };
            let shifted = unsafe { SHIFT };
            Some(Key::Char(if ukrainian {
                ukrainian_char(scan, shifted).unwrap_or(value as char)
            } else {
                value as char
            }))
        }
    }
}

pub fn toggle_layout() -> bool {
    unsafe {
        UKRAINIAN = !UKRAINIAN;
        UKRAINIAN
    }
}

fn ukrainian_char(scan: u8, shifted: bool) -> Option<char> {
    let lower = match scan {
        0x10 => 'й',
        0x11 => 'ц',
        0x12 => 'у',
        0x13 => 'к',
        0x14 => 'е',
        0x15 => 'н',
        0x16 => 'г',
        0x17 => 'ш',
        0x18 => 'щ',
        0x19 => 'з',
        0x1a => 'х',
        0x1b => 'ї',
        0x1e => 'ф',
        0x1f => 'і',
        0x20 => 'в',
        0x21 => 'а',
        0x22 => 'п',
        0x23 => 'р',
        0x24 => 'о',
        0x25 => 'л',
        0x26 => 'д',
        0x27 => 'ж',
        0x28 => 'є',
        0x29 => 'ґ',
        0x2c => 'я',
        0x2d => 'ч',
        0x2e => 'с',
        0x2f => 'м',
        0x30 => 'и',
        0x31 => 'т',
        0x32 => 'ь',
        0x33 => 'б',
        0x34 => 'ю',
        0x35 => '.',
        _ => return None,
    };
    if shifted {
        lower.to_uppercase().next()
    } else {
        Some(lower)
    }
}

unsafe fn input(port: u16) -> u8 {
    let value: u8;
    unsafe {
        asm!("in al, dx", in("dx") port, out("al") value, options(nomem, nostack));
    }
    value
}
