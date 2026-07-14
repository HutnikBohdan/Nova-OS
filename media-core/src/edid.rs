use crate::{display::DisplayMode, MediaError};

pub const BASE_BLOCK_LEN: usize = 128;
const HEADER: [u8; 8] = [0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EdidInfo<const N: usize> {
    pub manufacturer: [u8; 3],
    pub product_code: u16,
    pub serial_number: u32,
    pub manufacture_week: u8,
    pub manufacture_year: u16,
    pub digital_input: bool,
    pub extension_count: u8,
    modes: [Option<DisplayMode>; N],
    mode_count: usize,
}

impl<const N: usize> EdidInfo<N> {
    pub fn modes(&self) -> impl Iterator<Item = DisplayMode> + '_ {
        self.modes[..self.mode_count].iter().flatten().copied()
    }

    pub const fn mode_count(&self) -> usize {
        self.mode_count
    }
    pub fn preferred_mode(&self) -> Option<DisplayMode> {
        self.modes().next()
    }
}

pub fn checksum_valid(block: &[u8]) -> bool {
    block.len() == BASE_BLOCK_LEN
        && block.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

pub fn parse<const N: usize>(block: &[u8]) -> Result<EdidInfo<N>, MediaError> {
    if block.len() != BASE_BLOCK_LEN {
        return Err(MediaError::InvalidLength);
    }
    if block[..8] != HEADER {
        return Err(MediaError::InvalidValue);
    }
    if !checksum_valid(block) {
        return Err(MediaError::InvalidChecksum);
    }
    let raw_manufacturer = u16::from_be_bytes([block[8], block[9]]);
    let manufacturer = [
        decode_letter((raw_manufacturer >> 10) as u8),
        decode_letter((raw_manufacturer >> 5) as u8),
        decode_letter(raw_manufacturer as u8),
    ];
    if manufacturer.contains(&b'?') {
        return Err(MediaError::InvalidValue);
    }
    let mut result = EdidInfo {
        manufacturer,
        product_code: u16::from_le_bytes([block[10], block[11]]),
        serial_number: u32::from_le_bytes([block[12], block[13], block[14], block[15]]),
        manufacture_week: block[16],
        manufacture_year: 1990 + block[17] as u16,
        digital_input: block[20] & 0x80 != 0,
        extension_count: block[126],
        modes: [None; N],
        mode_count: 0,
    };
    let mut offset = 54;
    while offset < 126 {
        if let Some(mode) = parse_detailed_timing(&block[offset..offset + 18]) {
            if result.mode_count >= N {
                return Err(MediaError::Capacity);
            }
            result.modes[result.mode_count] = Some(mode);
            result.mode_count += 1;
        }
        offset += 18;
    }
    Ok(result)
}

const fn decode_letter(value: u8) -> u8 {
    let value = value & 0x1f;
    if value >= 1 && value <= 26 {
        b'A' + value - 1
    } else {
        b'?'
    }
}

fn parse_detailed_timing(data: &[u8]) -> Option<DisplayMode> {
    let pixel_clock_10khz = u16::from_le_bytes([data[0], data[1]]) as u32;
    if pixel_clock_10khz == 0 {
        return None;
    }
    let horizontal_active = data[2] as u32 | (((data[4] >> 4) as u32) << 8);
    let horizontal_blank = data[3] as u32 | (((data[4] & 0x0f) as u32) << 8);
    let vertical_active = data[5] as u32 | (((data[7] >> 4) as u32) << 8);
    let vertical_blank = data[6] as u32 | (((data[7] & 0x0f) as u32) << 8);
    let total_pixels = horizontal_active
        .checked_add(horizontal_blank)?
        .checked_mul(vertical_active.checked_add(vertical_blank)?)?;
    if horizontal_active == 0 || vertical_active == 0 || total_pixels == 0 {
        return None;
    }
    let pixel_clock_hz = pixel_clock_10khz as u64 * 10_000;
    Some(DisplayMode {
        width: horizontal_active,
        height: vertical_active,
        refresh_millihz: ((pixel_clock_hz * 1000) / total_pixels as u64) as u32,
        pixel_clock_khz: pixel_clock_10khz * 10,
        interlaced: data[17] & 0x80 != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> [u8; 128] {
        let mut edid = [0u8; 128];
        edid[..8].copy_from_slice(&HEADER);
        // Manufacturer "NVA".
        let vendor = ((14u16) << 10) | ((22u16) << 5) | 1;
        edid[8..10].copy_from_slice(&vendor.to_be_bytes());
        edid[10..12].copy_from_slice(&0x2026u16.to_le_bytes());
        edid[12..16].copy_from_slice(&42u32.to_le_bytes());
        edid[16] = 28;
        edid[17] = 36;
        edid[20] = 0x80;
        // 1920x1080 @ 60 Hz: 148.5 MHz, totals 2200x1125.
        let d = &mut edid[54..72];
        d[0..2].copy_from_slice(&14850u16.to_le_bytes());
        d[2] = 0x80;
        d[3] = 0x18;
        d[4] = 0x71;
        d[5] = 0x38;
        d[6] = 0x2d;
        d[7] = 0x40;
        let sum = edid[..127]
            .iter()
            .fold(0u8, |acc, byte| acc.wrapping_add(*byte));
        edid[127] = 0u8.wrapping_sub(sum);
        edid
    }

    #[test]
    fn parses_identity_and_preferred_timing() {
        let info = parse::<4>(&sample()).unwrap();
        assert_eq!(info.manufacturer, *b"NVA");
        assert_eq!(info.manufacture_year, 2026);
        assert!(info.digital_input);
        let mode = info.preferred_mode().unwrap();
        assert_eq!(
            (mode.width, mode.height, mode.refresh_millihz),
            (1920, 1080, 60_000)
        );
    }

    #[test]
    fn rejects_corrupt_checksum() {
        let mut edid = sample();
        edid[20] ^= 1;
        assert_eq!(parse::<4>(&edid), Err(MediaError::InvalidChecksum));
    }

    #[test]
    fn checksum_requires_exact_block() {
        assert!(!checksum_valid(&[0; 127]));
    }

    #[test]
    fn bounded_mode_capacity_is_enforced() {
        assert_eq!(parse::<0>(&sample()), Err(MediaError::Capacity));
    }
}
