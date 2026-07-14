#![no_std]

//! Детерміноване, allocation-free ядро композитора Nova OS.
//!
//! Модуль не торкається framebuffer, пристроїв або API ядра. Він перевіряє
//! власність буферів, атомарно приймає frame-транзакції та маршрутизує ввід;
//! системна служба композитора виконує фактичне копіювання/сканування пікселів.

pub const MAX_SURFACES: usize = 32;
pub const MAX_BUFFERS: usize = 48;
pub const MAX_DAMAGE: usize = 64;
pub const MAX_TRANSACTION_OPS: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SurfaceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BufferId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn right(self) -> i64 {
        self.x as i64 + self.width as i64
    }
    pub const fn bottom(self) -> i64 {
        self.y as i64 + self.height as i64
    }
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn contains(self, point: Point) -> bool {
        point.x as i64 >= self.x as i64
            && point.y as i64 >= self.y as i64
            && (point.x as i64) < self.right()
            && (point.y as i64) < self.bottom()
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = (self.x as i64).max(other.x as i64);
        let top = (self.y as i64).max(other.y as i64);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (left < right && top < bottom).then_some(Self::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        ))
    }

    pub fn union(self, other: Self) -> Self {
        let left = (self.x as i64).min(other.x as i64);
        let top = (self.y as i64).min(other.y as i64);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self::new(
            left as i32,
            top as i32,
            (right - left) as u32,
            (bottom - top) as u32,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Xrgb8888,
    Argb8888Premultiplied,
    Rgb565,
}

impl PixelFormat {
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgb565 => 2,
            _ => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities(u8);

impl Capabilities {
    pub const NONE: Self = Self(0);
    pub const MANAGE_OWN: Self = Self(1 << 0);
    pub const PLACE_ABOVE: Self = Self(1 << 1);
    pub const CAPTURE_INPUT: Self = Self(1 << 2);
    pub const SYSTEM_COMPOSITOR: Self = Self(1 << 3);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecurityContext {
    pub client: ClientId,
    pub capabilities: Capabilities,
}

impl SecurityContext {
    pub const fn application(client: ClientId) -> Self {
        Self {
            client,
            capabilities: Capabilities::MANAGE_OWN,
        }
    }

    pub const fn compositor(client: ClientId) -> Self {
        Self {
            client,
            capabilities: Capabilities::MANAGE_OWN
                .union(Capabilities::PLACE_ABOVE)
                .union(Capabilities::CAPTURE_INPUT)
                .union(Capabilities::SYSTEM_COMPOSITOR),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositorError {
    Capacity,
    DuplicateId,
    UnknownSurface,
    UnknownBuffer,
    PermissionDenied,
    InvalidGeometry,
    InvalidStride,
    BufferBusy,
    BufferAttached,
    InvalidOperation,
    EmptyTransaction,
    TransactionFull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Buffer {
    pub id: BufferId,
    pub owner: ClientId,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
    pub generation: u64,
    attached_to: Option<SurfaceId>,
}

impl Buffer {
    pub const fn attached_to(&self) -> Option<SurfaceId> {
        self.attached_to
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceRole {
    Desktop,
    Application,
    Dialog,
    Panel,
    Cursor,
}

impl SurfaceRole {
    pub const fn ukrainian_name(self) -> &'static str {
        match self {
            Self::Desktop => "Робочий стіл",
            Self::Application => "Вікно програми",
            Self::Dialog => "Діалогове вікно",
            Self::Panel => "Системна панель",
            Self::Cursor => "Вказівник",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub id: SurfaceId,
    pub owner: ClientId,
    pub role: SurfaceRole,
    pub bounds: Rect,
    pub input_region: Rect,
    pub mapped: bool,
    pub accepts_keyboard: bool,
    pub opacity: u8,
    pub buffer: Option<BufferId>,
    pub committed_frame: u64,
}

impl Surface {
    fn screen_input_region(self) -> Rect {
        Rect::new(
            self.bounds.x.saturating_add(self.input_region.x),
            self.bounds.y.saturating_add(self.input_region.y),
            self.input_region.width.min(self.bounds.width),
            self.input_region.height.min(self.bounds.height),
        )
    }
}

#[derive(Clone, Copy)]
struct SurfaceSlot {
    value: Surface,
    occupied: bool,
}

const EMPTY_SURFACE: SurfaceSlot = SurfaceSlot {
    occupied: false,
    value: Surface {
        id: SurfaceId(0),
        owner: ClientId(0),
        role: SurfaceRole::Application,
        bounds: Rect::new(0, 0, 0, 0),
        input_region: Rect::new(0, 0, 0, 0),
        mapped: false,
        accepts_keyboard: false,
        opacity: 255,
        buffer: None,
        committed_frame: 0,
    },
};

#[derive(Clone, Copy)]
struct BufferSlot {
    value: Buffer,
    occupied: bool,
}

const EMPTY_BUFFER: BufferSlot = BufferSlot {
    occupied: false,
    value: Buffer {
        id: BufferId(0),
        owner: ClientId(0),
        width: 0,
        height: 0,
        stride: 0,
        format: PixelFormat::Xrgb8888,
        generation: 0,
        attached_to: None,
    },
};

#[derive(Clone, Copy)]
pub struct DamageSet {
    rects: [Rect; MAX_DAMAGE],
    len: usize,
    screen: Rect,
    full: bool,
}

impl DamageSet {
    pub const fn new(screen: Rect) -> Self {
        Self {
            rects: [Rect::new(0, 0, 0, 0); MAX_DAMAGE],
            len: 0,
            screen,
            full: false,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.full = false;
    }
    pub const fn is_full(&self) -> bool {
        self.full
    }
    pub fn rects(&self) -> &[Rect] {
        if self.full {
            core::slice::from_ref(&self.screen)
        } else {
            &self.rects[..self.len]
        }
    }

    pub fn add(&mut self, rect: Rect) {
        let Some(mut clipped) = rect.intersection(self.screen) else {
            return;
        };
        if self.full {
            return;
        }
        let mut i = 0;
        while i < self.len {
            let current = self.rects[i];
            if current.intersection(clipped).is_some() || touches(current, clipped) {
                clipped = current.union(clipped);
                self.len -= 1;
                self.rects[i] = self.rects[self.len];
                i = 0;
            } else {
                i += 1;
            }
        }
        if self.len == MAX_DAMAGE {
            self.full = true;
            self.len = 0;
        } else {
            self.rects[self.len] = clipped;
            self.len += 1;
        }
    }
}

fn touches(a: Rect, b: Rect) -> bool {
    let horizontal = (a.right() == b.x as i64 || b.right() == a.x as i64)
        && (a.y as i64) < b.bottom()
        && (b.y as i64) < a.bottom();
    let vertical = (a.bottom() == b.y as i64 || b.bottom() == a.y as i64)
        && (a.x as i64) < b.right()
        && (b.x as i64) < a.right();
    horizontal || vertical
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOp {
    Attach {
        surface: SurfaceId,
        buffer: BufferId,
    },
    Detach {
        surface: SurfaceId,
    },
    Move {
        surface: SurfaceId,
        x: i32,
        y: i32,
    },
    SetMapped {
        surface: SurfaceId,
        mapped: bool,
    },
    SetOpacity {
        surface: SurfaceId,
        opacity: u8,
    },
    SetInputRegion {
        surface: SurfaceId,
        region: Rect,
    },
    Damage {
        surface: SurfaceId,
        local: Rect,
    },
    Raise {
        surface: SurfaceId,
    },
}

pub struct FrameTransaction {
    client: ClientId,
    serial: u64,
    operations: [FrameOp; MAX_TRANSACTION_OPS],
    len: usize,
}

const EMPTY_OP: FrameOp = FrameOp::SetMapped {
    surface: SurfaceId(0),
    mapped: false,
};

impl FrameTransaction {
    pub const fn new(client: ClientId, serial: u64) -> Self {
        Self {
            client,
            serial,
            operations: [EMPTY_OP; MAX_TRANSACTION_OPS],
            len: 0,
        }
    }
    pub const fn serial(&self) -> u64 {
        self.serial
    }
    pub fn push(&mut self, operation: FrameOp) -> Result<(), CompositorError> {
        if self.len == MAX_TRANSACTION_OPS {
            return Err(CompositorError::TransactionFull);
        }
        self.operations[self.len] = operation;
        self.len += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameReceipt {
    pub frame: u64,
    pub serial: u64,
    pub damage_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEvent {
    PointerMotion {
        position: Point,
    },
    PointerButton {
        position: Point,
        button: u8,
        pressed: bool,
    },
    Key {
        usage: u16,
        pressed: bool,
    },
    Text(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoutedInput {
    pub target: SurfaceId,
    pub local_position: Option<Point>,
    pub event: InputEvent,
}

#[derive(Clone, Copy)]
pub struct Compositor {
    screen: Rect,
    surfaces: [SurfaceSlot; MAX_SURFACES],
    buffers: [BufferSlot; MAX_BUFFERS],
    z_order: [SurfaceId; MAX_SURFACES],
    z_len: usize,
    damage: DamageSet,
    keyboard_focus: Option<SurfaceId>,
    pointer_capture: Option<SurfaceId>,
    frame: u64,
}

impl Compositor {
    pub const fn new(width: u32, height: u32) -> Self {
        let screen = Rect::new(0, 0, width, height);
        Self {
            screen,
            surfaces: [EMPTY_SURFACE; MAX_SURFACES],
            buffers: [EMPTY_BUFFER; MAX_BUFFERS],
            z_order: [SurfaceId(0); MAX_SURFACES],
            z_len: 0,
            damage: DamageSet::new(screen),
            keyboard_focus: None,
            pointer_capture: None,
            frame: 0,
        }
    }

    pub const fn frame_number(&self) -> u64 {
        self.frame
    }
    pub const fn screen_bounds(&self) -> Rect {
        self.screen
    }
    pub fn damage(&self) -> &[Rect] {
        self.damage.rects()
    }
    pub fn clear_damage(&mut self) {
        self.damage.clear();
    }
    pub const fn keyboard_focus(&self) -> Option<SurfaceId> {
        self.keyboard_focus
    }

    pub fn surface(&self, id: SurfaceId) -> Option<&Surface> {
        self.surface_index(id).map(|i| &self.surfaces[i].value)
    }

    pub fn buffer(&self, id: BufferId) -> Option<&Buffer> {
        self.buffer_index(id).map(|i| &self.buffers[i].value)
    }

    pub fn create_surface(
        &mut self,
        context: SecurityContext,
        id: SurfaceId,
        role: SurfaceRole,
        bounds: Rect,
    ) -> Result<(), CompositorError> {
        if !context.capabilities.contains(Capabilities::MANAGE_OWN) {
            return Err(CompositorError::PermissionDenied);
        }
        if bounds.is_empty() {
            return Err(CompositorError::InvalidGeometry);
        }
        if self.surface_index(id).is_some() {
            return Err(CompositorError::DuplicateId);
        }
        let index = self
            .surfaces
            .iter()
            .position(|slot| !slot.occupied)
            .ok_or(CompositorError::Capacity)?;
        self.surfaces[index] = SurfaceSlot {
            occupied: true,
            value: Surface {
                id,
                owner: context.client,
                role,
                bounds,
                input_region: Rect::new(0, 0, bounds.width, bounds.height),
                mapped: false,
                accepts_keyboard: !matches!(role, SurfaceRole::Desktop | SurfaceRole::Cursor),
                opacity: 255,
                buffer: None,
                committed_frame: self.frame,
            },
        };
        self.z_order[self.z_len] = id;
        self.z_len += 1;
        Ok(())
    }

    pub fn destroy_surface(
        &mut self,
        context: SecurityContext,
        id: SurfaceId,
    ) -> Result<(), CompositorError> {
        let index = self
            .surface_index(id)
            .ok_or(CompositorError::UnknownSurface)?;
        self.require_owner(context, self.surfaces[index].value.owner)?;
        let old = self.surfaces[index].value;
        if let Some(buffer) = old.buffer {
            let bi = self
                .buffer_index(buffer)
                .ok_or(CompositorError::UnknownBuffer)?;
            self.buffers[bi].value.attached_to = None;
        }
        self.surfaces[index].occupied = false;
        if old.mapped {
            self.damage.add(old.bounds);
        }
        self.remove_z(id);
        if self.keyboard_focus == Some(id) {
            self.keyboard_focus = None;
        }
        if self.pointer_capture == Some(id) {
            self.pointer_capture = None;
        }
        Ok(())
    }

    pub fn create_buffer(
        &mut self,
        context: SecurityContext,
        id: BufferId,
        width: u32,
        height: u32,
        stride: u32,
        format: PixelFormat,
    ) -> Result<(), CompositorError> {
        if !context.capabilities.contains(Capabilities::MANAGE_OWN) {
            return Err(CompositorError::PermissionDenied);
        }
        if width == 0 || height == 0 {
            return Err(CompositorError::InvalidGeometry);
        }
        let minimum = width
            .checked_mul(format.bytes_per_pixel())
            .ok_or(CompositorError::InvalidStride)?;
        if stride < minimum || stride % format.bytes_per_pixel() != 0 {
            return Err(CompositorError::InvalidStride);
        }
        if self.buffer_index(id).is_some() {
            return Err(CompositorError::DuplicateId);
        }
        let index = self
            .buffers
            .iter()
            .position(|slot| !slot.occupied)
            .ok_or(CompositorError::Capacity)?;
        self.buffers[index] = BufferSlot {
            occupied: true,
            value: Buffer {
                id,
                owner: context.client,
                width,
                height,
                stride,
                format,
                generation: 0,
                attached_to: None,
            },
        };
        Ok(())
    }

    pub fn destroy_buffer(
        &mut self,
        context: SecurityContext,
        id: BufferId,
    ) -> Result<(), CompositorError> {
        let index = self
            .buffer_index(id)
            .ok_or(CompositorError::UnknownBuffer)?;
        self.require_owner(context, self.buffers[index].value.owner)?;
        if self.buffers[index].value.attached_to.is_some() {
            return Err(CompositorError::BufferAttached);
        }
        self.buffers[index].occupied = false;
        Ok(())
    }

    /// Commits all operations or none. Validation and application run against a
    /// stack-resident copy so a rejected transaction cannot leak partial state.
    pub fn commit(
        &mut self,
        context: SecurityContext,
        transaction: &FrameTransaction,
    ) -> Result<FrameReceipt, CompositorError> {
        if transaction.len == 0 {
            return Err(CompositorError::EmptyTransaction);
        }
        if transaction.client != context.client
            && !context
                .capabilities
                .contains(Capabilities::SYSTEM_COMPOSITOR)
        {
            return Err(CompositorError::PermissionDenied);
        }
        let mut staged = *self;
        staged.damage.clear();
        let next_frame = self.frame.wrapping_add(1);
        for operation in &transaction.operations[..transaction.len] {
            staged.apply_operation(context, *operation, next_frame)?;
        }
        staged.frame = next_frame;
        let damage_count = staged.damage.rects().len();
        *self = staged;
        Ok(FrameReceipt {
            frame: next_frame,
            serial: transaction.serial,
            damage_count,
        })
    }

    pub fn set_keyboard_focus(
        &mut self,
        context: SecurityContext,
        target: Option<SurfaceId>,
    ) -> Result<(), CompositorError> {
        if let Some(id) = target {
            let surface = *self.surface(id).ok_or(CompositorError::UnknownSurface)?;
            if !surface.mapped || !surface.accepts_keyboard {
                return Err(CompositorError::InvalidOperation);
            }
            if surface.owner != context.client
                && !context
                    .capabilities
                    .contains(Capabilities::SYSTEM_COMPOSITOR)
            {
                return Err(CompositorError::PermissionDenied);
            }
        }
        self.keyboard_focus = target;
        Ok(())
    }

    pub fn capture_pointer(
        &mut self,
        context: SecurityContext,
        target: Option<SurfaceId>,
    ) -> Result<(), CompositorError> {
        if !context.capabilities.contains(Capabilities::CAPTURE_INPUT) {
            return Err(CompositorError::PermissionDenied);
        }
        if let Some(id) = target {
            let surface = self.surface(id).ok_or(CompositorError::UnknownSurface)?;
            if !surface.mapped {
                return Err(CompositorError::InvalidOperation);
            }
        }
        self.pointer_capture = target;
        Ok(())
    }

    pub fn route_input(&self, event: InputEvent) -> Option<RoutedInput> {
        match event {
            InputEvent::Key { .. } | InputEvent::Text(_) => {
                self.keyboard_focus.map(|target| RoutedInput {
                    target,
                    local_position: None,
                    event,
                })
            }
            InputEvent::PointerMotion { position } | InputEvent::PointerButton { position, .. } => {
                let target = self.pointer_capture.or_else(|| self.hit_test(position))?;
                let surface = self.surface(target)?;
                Some(RoutedInput {
                    target,
                    local_position: Some(Point {
                        x: position.x - surface.bounds.x,
                        y: position.y - surface.bounds.y,
                    }),
                    event,
                })
            }
        }
    }

    pub fn hit_test(&self, position: Point) -> Option<SurfaceId> {
        let mut index = self.z_len;
        while index > 0 {
            index -= 1;
            let id = self.z_order[index];
            if let Some(surface) = self.surface(id) {
                if surface.mapped && surface.screen_input_region().contains(position) {
                    return Some(id);
                }
            }
        }
        None
    }

    pub fn paint_order(&self) -> impl Iterator<Item = &Surface> {
        self.z_order[..self.z_len].iter().filter_map(|id| {
            self.surface(*id)
                .filter(|surface| surface.mapped && surface.buffer.is_some())
        })
    }

    fn apply_operation(
        &mut self,
        context: SecurityContext,
        operation: FrameOp,
        frame: u64,
    ) -> Result<(), CompositorError> {
        let surface_id = match operation {
            FrameOp::Attach { surface, .. }
            | FrameOp::Detach { surface }
            | FrameOp::Move { surface, .. }
            | FrameOp::SetMapped { surface, .. }
            | FrameOp::SetOpacity { surface, .. }
            | FrameOp::SetInputRegion { surface, .. }
            | FrameOp::Damage { surface, .. }
            | FrameOp::Raise { surface } => surface,
        };
        let si = self
            .surface_index(surface_id)
            .ok_or(CompositorError::UnknownSurface)?;
        let owner = self.surfaces[si].value.owner;
        self.require_owner(context, owner)?;

        match operation {
            FrameOp::Attach { surface, buffer } => {
                let bi = self
                    .buffer_index(buffer)
                    .ok_or(CompositorError::UnknownBuffer)?;
                let target = self.surfaces[si].value;
                self.require_owner(context, self.buffers[bi].value.owner)?;
                if self.buffers[bi].value.width < target.bounds.width
                    || self.buffers[bi].value.height < target.bounds.height
                {
                    return Err(CompositorError::InvalidGeometry);
                }
                if let Some(attached) = self.buffers[bi].value.attached_to {
                    if attached != surface {
                        return Err(CompositorError::BufferBusy);
                    }
                }
                if let Some(old) = target.buffer {
                    if old != buffer {
                        let old_index = self
                            .buffer_index(old)
                            .ok_or(CompositorError::UnknownBuffer)?;
                        self.buffers[old_index].value.attached_to = None;
                    }
                }
                self.buffers[bi].value.attached_to = Some(surface);
                self.buffers[bi].value.generation =
                    self.buffers[bi].value.generation.wrapping_add(1);
                self.surfaces[si].value.buffer = Some(buffer);
                self.damage.add(target.bounds);
            }
            FrameOp::Detach { .. } => {
                let target = self.surfaces[si].value;
                if let Some(buffer) = target.buffer {
                    let bi = self
                        .buffer_index(buffer)
                        .ok_or(CompositorError::UnknownBuffer)?;
                    self.buffers[bi].value.attached_to = None;
                    self.surfaces[si].value.buffer = None;
                    if target.mapped {
                        self.damage.add(target.bounds);
                    }
                }
            }
            FrameOp::Move { x, y, .. } => {
                let old = self.surfaces[si].value.bounds;
                if self.surfaces[si].value.mapped {
                    self.damage.add(old);
                }
                self.surfaces[si].value.bounds.x = x;
                self.surfaces[si].value.bounds.y = y;
                if self.surfaces[si].value.mapped {
                    self.damage.add(self.surfaces[si].value.bounds);
                }
            }
            FrameOp::SetMapped { mapped, .. } => {
                if mapped && self.surfaces[si].value.buffer.is_none() {
                    return Err(CompositorError::InvalidOperation);
                }
                if self.surfaces[si].value.mapped != mapped {
                    self.damage.add(self.surfaces[si].value.bounds);
                }
                self.surfaces[si].value.mapped = mapped;
                if !mapped {
                    if self.keyboard_focus == Some(surface_id) {
                        self.keyboard_focus = None;
                    }
                    if self.pointer_capture == Some(surface_id) {
                        self.pointer_capture = None;
                    }
                }
            }
            FrameOp::SetOpacity { opacity, .. } => {
                if self.surfaces[si].value.opacity != opacity {
                    self.damage.add(self.surfaces[si].value.bounds);
                }
                self.surfaces[si].value.opacity = opacity;
            }
            FrameOp::SetInputRegion { region, .. } => {
                if region.x < 0
                    || region.y < 0
                    || region.right() > self.surfaces[si].value.bounds.width as i64
                    || region.bottom() > self.surfaces[si].value.bounds.height as i64
                {
                    return Err(CompositorError::InvalidGeometry);
                }
                self.surfaces[si].value.input_region = region;
            }
            FrameOp::Damage { local, .. } => {
                let bounds = self.surfaces[si].value.bounds;
                let local_bounds = Rect::new(0, 0, bounds.width, bounds.height);
                if let Some(clipped) = local.intersection(local_bounds) {
                    self.damage.add(Rect::new(
                        bounds.x + clipped.x,
                        bounds.y + clipped.y,
                        clipped.width,
                        clipped.height,
                    ));
                }
            }
            FrameOp::Raise { .. } => {
                if !context.capabilities.contains(Capabilities::PLACE_ABOVE) {
                    return Err(CompositorError::PermissionDenied);
                }
                self.remove_z(surface_id);
                self.z_order[self.z_len] = surface_id;
                self.z_len += 1;
                if self.surfaces[si].value.mapped {
                    self.damage.add(self.surfaces[si].value.bounds);
                }
            }
        }
        self.surfaces[si].value.committed_frame = frame;
        Ok(())
    }

    fn require_owner(
        &self,
        context: SecurityContext,
        owner: ClientId,
    ) -> Result<(), CompositorError> {
        if context.client == owner && context.capabilities.contains(Capabilities::MANAGE_OWN)
            || context
                .capabilities
                .contains(Capabilities::SYSTEM_COMPOSITOR)
        {
            Ok(())
        } else {
            Err(CompositorError::PermissionDenied)
        }
    }

    fn surface_index(&self, id: SurfaceId) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|slot| slot.occupied && slot.value.id == id)
    }
    fn buffer_index(&self, id: BufferId) -> Option<usize> {
        self.buffers
            .iter()
            .position(|slot| slot.occupied && slot.value.id == id)
    }
    fn remove_z(&mut self, id: SurfaceId) {
        if let Some(index) = self.z_order[..self.z_len]
            .iter()
            .position(|candidate| *candidate == id)
        {
            self.z_order.copy_within(index + 1..self.z_len, index);
            self.z_len -= 1;
        }
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;

    const APP_A: SecurityContext = SecurityContext::application(ClientId(10));
    const APP_B: SecurityContext = SecurityContext::application(ClientId(20));
    const SYSTEM: SecurityContext = SecurityContext::compositor(ClientId(1));

    fn ready_surface(
        c: &mut Compositor,
        context: SecurityContext,
        sid: u32,
        bid: u32,
        bounds: Rect,
    ) {
        c.create_surface(context, SurfaceId(sid), SurfaceRole::Application, bounds)
            .unwrap();
        c.create_buffer(
            context,
            BufferId(bid),
            bounds.width,
            bounds.height,
            bounds.width * 4,
            PixelFormat::Xrgb8888,
        )
        .unwrap();
        let mut tx = FrameTransaction::new(context.client, 1);
        tx.push(FrameOp::Attach {
            surface: SurfaceId(sid),
            buffer: BufferId(bid),
        })
        .unwrap();
        tx.push(FrameOp::SetMapped {
            surface: SurfaceId(sid),
            mapped: true,
        })
        .unwrap();
        c.commit(context, &tx).unwrap();
        c.clear_damage();
    }

    #[test]
    fn ukrainian_roles_are_stable() {
        assert_eq!(SurfaceRole::Application.ukrainian_name(), "Вікно програми");
        assert_eq!(SurfaceRole::Panel.ukrainian_name(), "Системна панель");
    }

    #[test]
    fn buffer_stride_and_ownership_are_enforced() {
        let mut c = Compositor::new(800, 600);
        assert_eq!(
            c.create_buffer(APP_A, BufferId(1), 10, 10, 39, PixelFormat::Xrgb8888),
            Err(CompositorError::InvalidStride)
        );
        c.create_buffer(APP_A, BufferId(1), 10, 10, 40, PixelFormat::Xrgb8888)
            .unwrap();
        assert_eq!(
            c.destroy_buffer(APP_B, BufferId(1)),
            Err(CompositorError::PermissionDenied)
        );
    }

    #[test]
    fn attachment_is_exclusive_and_released() {
        let mut c = Compositor::new(800, 600);
        c.create_surface(
            APP_A,
            SurfaceId(1),
            SurfaceRole::Application,
            Rect::new(0, 0, 10, 10),
        )
        .unwrap();
        c.create_surface(
            APP_A,
            SurfaceId(2),
            SurfaceRole::Dialog,
            Rect::new(0, 0, 10, 10),
        )
        .unwrap();
        c.create_buffer(APP_A, BufferId(1), 10, 10, 40, PixelFormat::Xrgb8888)
            .unwrap();
        let mut first = FrameTransaction::new(APP_A.client, 1);
        first
            .push(FrameOp::Attach {
                surface: SurfaceId(1),
                buffer: BufferId(1),
            })
            .unwrap();
        c.commit(APP_A, &first).unwrap();
        let mut second = FrameTransaction::new(APP_A.client, 2);
        second
            .push(FrameOp::Attach {
                surface: SurfaceId(2),
                buffer: BufferId(1),
            })
            .unwrap();
        assert_eq!(c.commit(APP_A, &second), Err(CompositorError::BufferBusy));
        let mut detach = FrameTransaction::new(APP_A.client, 3);
        detach
            .push(FrameOp::Detach {
                surface: SurfaceId(1),
            })
            .unwrap();
        c.commit(APP_A, &detach).unwrap();
        c.commit(APP_A, &second).unwrap();
    }

    #[test]
    fn transaction_is_atomic_on_late_failure() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(10, 10, 100, 100));
        let before = *c.surface(SurfaceId(1)).unwrap();
        let frame = c.frame_number();
        let mut tx = FrameTransaction::new(APP_A.client, 9);
        tx.push(FrameOp::Move {
            surface: SurfaceId(1),
            x: 300,
            y: 200,
        })
        .unwrap();
        tx.push(FrameOp::Attach {
            surface: SurfaceId(1),
            buffer: BufferId(999),
        })
        .unwrap();
        assert_eq!(c.commit(APP_A, &tx), Err(CompositorError::UnknownBuffer));
        assert_eq!(*c.surface(SurfaceId(1)).unwrap(), before);
        assert_eq!(c.frame_number(), frame);
        assert!(c.damage().is_empty());
    }

    #[test]
    fn damage_clips_merges_and_promotes_to_full() {
        let mut damage = DamageSet::new(Rect::new(0, 0, 100, 100));
        damage.add(Rect::new(-10, 10, 20, 10));
        damage.add(Rect::new(10, 10, 10, 10));
        assert_eq!(damage.rects(), &[Rect::new(0, 10, 20, 10)]);
        damage.clear();
        for i in 0..=MAX_DAMAGE {
            damage.add(Rect::new((i % 10 * 9) as i32, (i / 10 * 9) as i32, 1, 1));
        }
        assert!(damage.is_full());
        assert_eq!(damage.rects(), &[Rect::new(0, 0, 100, 100)]);
    }

    #[test]
    fn move_damages_old_and_new_positions() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(10, 10, 100, 100));
        let mut tx = FrameTransaction::new(APP_A.client, 8);
        tx.push(FrameOp::Move {
            surface: SurfaceId(1),
            x: 300,
            y: 200,
        })
        .unwrap();
        c.commit(APP_A, &tx).unwrap();
        assert_eq!(c.damage().len(), 2);
        assert!(c.damage().contains(&Rect::new(10, 10, 100, 100)));
        assert!(c.damage().contains(&Rect::new(300, 200, 100, 100)));
    }

    #[test]
    fn z_order_hit_test_and_raise_are_deterministic() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(0, 0, 100, 100));
        ready_surface(&mut c, APP_B, 2, 2, Rect::new(20, 20, 100, 100));
        assert_eq!(c.hit_test(Point { x: 30, y: 30 }), Some(SurfaceId(2)));
        let mut denied = FrameTransaction::new(APP_A.client, 2);
        denied
            .push(FrameOp::Raise {
                surface: SurfaceId(1),
            })
            .unwrap();
        assert_eq!(
            c.commit(APP_A, &denied),
            Err(CompositorError::PermissionDenied)
        );
        let mut raised = FrameTransaction::new(SYSTEM.client, 3);
        raised
            .push(FrameOp::Raise {
                surface: SurfaceId(1),
            })
            .unwrap();
        c.commit(SYSTEM, &raised).unwrap();
        assert_eq!(c.hit_test(Point { x: 30, y: 30 }), Some(SurfaceId(1)));
    }

    #[test]
    fn input_region_and_local_coordinates_are_honored() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(100, 80, 100, 100));
        let mut tx = FrameTransaction::new(APP_A.client, 4);
        tx.push(FrameOp::SetInputRegion {
            surface: SurfaceId(1),
            region: Rect::new(10, 10, 20, 20),
        })
        .unwrap();
        c.commit(APP_A, &tx).unwrap();
        assert!(
            c.route_input(InputEvent::PointerMotion {
                position: Point { x: 105, y: 85 }
            })
            .is_none()
        );
        let routed = c
            .route_input(InputEvent::PointerMotion {
                position: Point { x: 115, y: 95 },
            })
            .unwrap();
        assert_eq!(routed.target, SurfaceId(1));
        assert_eq!(routed.local_position, Some(Point { x: 15, y: 15 }));
    }

    #[test]
    fn keyboard_focus_requires_mapped_authorized_surface() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(0, 0, 100, 100));
        assert_eq!(
            c.set_keyboard_focus(APP_B, Some(SurfaceId(1))),
            Err(CompositorError::PermissionDenied)
        );
        c.set_keyboard_focus(APP_A, Some(SurfaceId(1))).unwrap();
        let routed = c.route_input(InputEvent::Text('ї')).unwrap();
        assert_eq!(routed.target, SurfaceId(1));
    }

    #[test]
    fn pointer_capture_needs_capability_and_clears_on_unmap() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(0, 0, 100, 100));
        assert_eq!(
            c.capture_pointer(APP_A, Some(SurfaceId(1))),
            Err(CompositorError::PermissionDenied)
        );
        c.capture_pointer(SYSTEM, Some(SurfaceId(1))).unwrap();
        assert_eq!(
            c.route_input(InputEvent::PointerMotion {
                position: Point { x: 700, y: 500 }
            })
            .unwrap()
            .target,
            SurfaceId(1)
        );
        let mut tx = FrameTransaction::new(APP_A.client, 8);
        tx.push(FrameOp::SetMapped {
            surface: SurfaceId(1),
            mapped: false,
        })
        .unwrap();
        c.commit(APP_A, &tx).unwrap();
        assert!(
            c.route_input(InputEvent::PointerMotion {
                position: Point { x: 700, y: 500 }
            })
            .is_none()
        );
    }

    #[test]
    fn destroy_surface_releases_buffer_and_focus() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(0, 0, 100, 100));
        c.set_keyboard_focus(APP_A, Some(SurfaceId(1))).unwrap();
        c.destroy_surface(APP_A, SurfaceId(1)).unwrap();
        assert_eq!(c.keyboard_focus(), None);
        assert_eq!(c.buffer(BufferId(1)).unwrap().attached_to(), None);
        c.destroy_buffer(APP_A, BufferId(1)).unwrap();
    }

    #[test]
    fn paint_order_omits_unmapped_and_bufferless_surfaces() {
        let mut c = Compositor::new(800, 600);
        ready_surface(&mut c, APP_A, 1, 1, Rect::new(0, 0, 100, 100));
        c.create_surface(
            APP_A,
            SurfaceId(2),
            SurfaceRole::Dialog,
            Rect::new(0, 0, 50, 50),
        )
        .unwrap();
        assert_eq!(
            c.paint_order().map(|s| s.id).collect::<std::vec::Vec<_>>(),
            std::vec![SurfaceId(1)]
        );
    }

    #[test]
    fn frame_receipt_has_serial_and_monotonic_frame() {
        let mut c = Compositor::new(800, 600);
        c.create_surface(
            APP_A,
            SurfaceId(1),
            SurfaceRole::Application,
            Rect::new(0, 0, 10, 10),
        )
        .unwrap();
        let mut tx = FrameTransaction::new(APP_A.client, 77);
        tx.push(FrameOp::SetOpacity {
            surface: SurfaceId(1),
            opacity: 200,
        })
        .unwrap();
        let receipt = c.commit(APP_A, &tx).unwrap();
        assert_eq!(receipt.frame, 1);
        assert_eq!(receipt.serial, 77);
        assert_eq!(c.surface(SurfaceId(1)).unwrap().committed_frame, 1);
    }
}
