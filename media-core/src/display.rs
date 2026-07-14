use crate::MediaError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Xrgb8888,
    Argb8888,
    Bgrx8888,
    Rgb565,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> u8 {
        match self {
            Self::Rgb565 => 2,
            _ => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_millihz: u32,
    pub pixel_clock_khz: u32,
    pub interlaced: bool,
}

impl DisplayMode {
    pub const fn validate(self) -> Result<Self, MediaError> {
        if self.width == 0 || self.height == 0 || self.refresh_millihz == 0 {
            Err(MediaError::InvalidValue)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramebufferDescriptor {
    pub physical_address: u64,
    pub byte_len: usize,
    pub stride_bytes: u32,
    pub mode: DisplayMode,
    pub format: PixelFormat,
}

impl FramebufferDescriptor {
    pub fn validate(self) -> Result<Self, MediaError> {
        self.mode.validate()?;
        let row = self
            .mode
            .width
            .checked_mul(self.format.bytes_per_pixel() as u32)
            .ok_or(MediaError::InvalidValue)?;
        let required = (self.stride_bytes as usize)
            .checked_mul(self.mode.height as usize)
            .ok_or(MediaError::InvalidValue)?;
        if self.stride_bytes < row || self.byte_len < required {
            Err(MediaError::InvalidLength)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn clipped(self, width: u32, height: u32) -> Self {
        let x1 = self.x.min(width);
        let y1 = self.y.min(height);
        let x2 = self.x.saturating_add(self.width).min(width);
        let y2 = self.y.saturating_add(self.height).min(height);
        Self::new(x1, y1, x2.saturating_sub(x1), y2.saturating_sub(y1))
    }

    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let x2 = self
            .x
            .saturating_add(self.width)
            .max(other.x.saturating_add(other.width));
        let y2 = self
            .y
            .saturating_add(self.height)
            .max(other.y.saturating_add(other.height));
        Self::new(x, y, x2.saturating_sub(x), y2.saturating_sub(y))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopySpan {
    pub source_offset: usize,
    pub destination_offset: usize,
    pub byte_len: usize,
}

/// One span per damaged scanline; the caller controls the hard upper bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CopyPlan<const N: usize> {
    spans: [CopySpan; N],
    len: usize,
}

impl<const N: usize> CopyPlan<N> {
    pub const fn new() -> Self {
        Self {
            spans: [CopySpan {
                source_offset: 0,
                destination_offset: 0,
                byte_len: 0,
            }; N],
            len: 0,
        }
    }

    pub fn for_damage(
        damage: Rect,
        mode: DisplayMode,
        format: PixelFormat,
        source_stride: u32,
        destination_stride: u32,
    ) -> Result<Self, MediaError> {
        let clipped = damage.clipped(mode.width, mode.height);
        let bpp = format.bytes_per_pixel() as usize;
        let minimum_stride = mode
            .width
            .checked_mul(bpp as u32)
            .ok_or(MediaError::InvalidValue)?;
        if source_stride < minimum_stride || destination_stride < minimum_stride {
            return Err(MediaError::InvalidLength);
        }
        if clipped.height as usize > N {
            return Err(MediaError::Capacity);
        }
        let mut plan = Self::new();
        let mut row = 0;
        while row < clipped.height {
            let y = clipped.y + row;
            let x_bytes = clipped.x as usize * bpp;
            plan.spans[plan.len] = CopySpan {
                source_offset: y as usize * source_stride as usize + x_bytes,
                destination_offset: y as usize * destination_stride as usize + x_bytes,
                byte_len: clipped.width as usize * bpp,
            };
            plan.len += 1;
            row += 1;
        }
        Ok(plan)
    }

    pub fn spans(&self) -> &[CopySpan] {
        &self.spans[..self.len]
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl<const N: usize> Default for CopyPlan<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CursorPlane {
    pub position_x: i32,
    pub position_y: i32,
    pub hotspot_x: u16,
    pub hotspot_y: u16,
    pub width: u16,
    pub height: u16,
    pub visible: bool,
}

impl CursorPlane {
    pub fn damage(self, next: Self) -> Rect {
        cursor_rect(self).union(cursor_rect(next))
    }
}

fn cursor_rect(cursor: CursorPlane) -> Rect {
    if !cursor.visible {
        return Rect::default();
    }
    let x = cursor
        .position_x
        .saturating_sub(cursor.hotspot_x as i32)
        .max(0) as u32;
    let y = cursor
        .position_y
        .saturating_sub(cursor.hotspot_y as i32)
        .max(0) as u32;
    Rect::new(x, y, cursor.width as u32, cursor.height as u32)
}

pub fn convert_argb8888(pixel: u32, destination: PixelFormat) -> u32 {
    let r = (pixel >> 16) & 0xff;
    let g = (pixel >> 8) & 0xff;
    let b = pixel & 0xff;
    match destination {
        PixelFormat::Argb8888 => pixel,
        PixelFormat::Xrgb8888 => (r << 16) | (g << 8) | b,
        PixelFormat::Bgrx8888 => (b << 24) | (g << 16) | (r << 8),
        PixelFormat::Rgb565 => ((r >> 3) << 11) | ((g >> 2) << 5) | (b >> 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode() -> DisplayMode {
        DisplayMode {
            width: 800,
            height: 600,
            refresh_millihz: 60_000,
            pixel_clock_khz: 40_000,
            interlaced: false,
        }
    }

    #[test]
    fn framebuffer_rejects_short_stride() {
        let fb = FramebufferDescriptor {
            physical_address: 0,
            byte_len: 1_920_000,
            stride_bytes: 100,
            mode: mode(),
            format: PixelFormat::Xrgb8888,
        };
        assert_eq!(fb.validate(), Err(MediaError::InvalidLength));
    }

    #[test]
    fn damage_plan_is_clipped_and_stride_aware() {
        let plan = CopyPlan::<8>::for_damage(
            Rect::new(798, 598, 20, 20),
            mode(),
            PixelFormat::Xrgb8888,
            3200,
            4096,
        )
        .unwrap();
        assert_eq!(plan.len(), 2);
        assert_eq!(
            plan.spans()[0],
            CopySpan {
                source_offset: 598 * 3200 + 798 * 4,
                destination_offset: 598 * 4096 + 798 * 4,
                byte_len: 8
            }
        );
    }

    #[test]
    fn plan_has_explicit_capacity() {
        assert_eq!(
            CopyPlan::<1>::for_damage(
                Rect::new(0, 0, 5, 2),
                mode(),
                PixelFormat::Rgb565,
                1600,
                1600
            ),
            Err(MediaError::Capacity)
        );
    }

    #[test]
    fn cursor_move_damages_old_and_new_bounds() {
        let a = CursorPlane {
            position_x: 10,
            position_y: 10,
            hotspot_x: 2,
            hotspot_y: 2,
            width: 16,
            height: 16,
            visible: true,
        };
        let b = CursorPlane {
            position_x: 30,
            ..a
        };
        assert_eq!(a.damage(b), Rect::new(8, 8, 36, 16));
    }

    #[test]
    fn pixel_conversion_is_exact() {
        assert_eq!(convert_argb8888(0xff_ff_00_00, PixelFormat::Rgb565), 0xf800);
        assert_eq!(
            convert_argb8888(0xff_11_22_33, PixelFormat::Bgrx8888),
            0x33221100
        );
    }
}
