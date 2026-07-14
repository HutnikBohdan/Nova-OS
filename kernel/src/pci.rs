use core::arch::asm;
use driver_core::{PciAddress, PciConfig, PciDevice, enumerate};

pub(crate) struct HardwarePci;

impl PciConfig for HardwarePci {
    fn read_u32(&mut self, address: PciAddress, offset: u8) -> u32 {
        unsafe {
            out_u32(0x0cf8, address.config_address(offset));
            in_u32(0x0cfc)
        }
    }

    fn write_u32(&mut self, address: PciAddress, offset: u8, value: u32) {
        unsafe {
            out_u32(0x0cf8, address.config_address(offset));
            out_u32(0x0cfc, value);
        }
    }
}

pub fn discover() -> usize {
    let mut devices: [Option<PciDevice>; 64] = [None; 64];
    enumerate(&mut HardwarePci, &mut devices)
}

pub fn find_class(class: u8, subclass: u8) -> Option<PciDevice> {
    find(|info| info.class == class && info.subclass == subclass)
}

pub fn find_vendor(vendor: u16, devices: &[u16]) -> Option<PciDevice> {
    find(|info| info.vendor == vendor && devices.contains(&info.device))
}

fn find(mut predicate: impl FnMut(&PciDevice) -> bool) -> Option<PciDevice> {
    let mut config = HardwarePci;
    for bus in 0..=255u8 {
        for device in 0..32u8 {
            for function in 0..8u8 {
                let address = PciAddress {
                    bus,
                    device,
                    function,
                };
                let Some(info) = driver_core::probe(&mut config, address) else {
                    if function == 0 {
                        break;
                    }
                    continue;
                };
                if predicate(&info) {
                    return Some(info);
                }
                if function == 0 && info.header_type & 0x80 == 0 {
                    break;
                }
            }
        }
    }
    None
}

pub fn read_config(address: PciAddress, offset: u8) -> u32 {
    HardwarePci.read_u32(address, offset)
}

pub fn write_config(address: PciAddress, offset: u8, value: u32) {
    HardwarePci.write_u32(address, offset, value)
}

unsafe fn out_u32(port: u16, value: u32) {
    unsafe {
        asm!("out dx, eax", in("dx") port, in("eax") value, options(nomem, nostack));
    }
}

unsafe fn in_u32(port: u16) -> u32 {
    let value: u32;
    unsafe {
        asm!("in eax, dx", in("dx") port, out("eax") value, options(nomem, nostack));
    }
    value
}
