use crate::ioapic::{Polarity, TriggerMode};
use crate::topology::{ApicId, InterruptOverride};
use crate::{BoundedVec, Error};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity {
    Fixed(ApicId),
    RoundRobin,
    LowestLoad,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrqRoute {
    pub source: u8,
    pub gsi: u32,
    pub vector: u8,
    pub trigger: TriggerMode,
    pub polarity: Polarity,
    pub affinity: Affinity,
    pub masked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IrqTable<const N: usize> {
    routes: BoundedVec<IrqRoute, N>,
    next_vector: u8,
}
impl<const N: usize> IrqTable<N> {
    pub const fn new(first_vector: u8) -> Self {
        Self {
            routes: BoundedVec::new(),
            next_vector: first_vector,
        }
    }
    pub fn route_isa(
        &mut self,
        source: u8,
        overrides: &[InterruptOverride],
        affinity: Affinity,
    ) -> Result<IrqRoute, Error> {
        if self.routes.iter().any(|r| r.source == source) {
            return Err(Error::Duplicate);
        }
        if self.next_vector < 32 {
            return Err(Error::InvalidValue);
        }
        let ov = overrides.iter().find(|x| x.bus == 0 && x.source == source);
        let (gsi, flags) = ov.map(|x| (x.gsi, x.flags)).unwrap_or((source as u32, 0));
        let polarity = match flags & 3 {
            0 | 1 => Polarity::ActiveHigh,
            3 => Polarity::ActiveLow,
            _ => return Err(Error::InvalidValue),
        };
        let trigger = match (flags >> 2) & 3 {
            0 | 1 => TriggerMode::Edge,
            3 => TriggerMode::Level,
            _ => return Err(Error::InvalidValue),
        };
        let route = IrqRoute {
            source,
            gsi,
            vector: self.next_vector,
            trigger,
            polarity,
            affinity,
            masked: true,
        };
        self.next_vector = self.next_vector.checked_add(1).ok_or(Error::Capacity)?;
        self.routes.push(route)?;
        Ok(route)
    }
    pub fn get(&self, source: u8) -> Option<&IrqRoute> {
        self.routes.iter().find(|r| r.source == source)
    }
    pub fn set_masked(&mut self, source: u8, masked: bool) -> Result<(), Error> {
        let i = (0..self.routes.len())
            .find(|&i| {
                self.routes
                    .get(i)
                    .map(|r| r.source == source)
                    .unwrap_or(false)
            })
            .ok_or(Error::NotFound)?;
        self.routes.get_mut(i).unwrap().masked = masked;
        Ok(())
    }
    pub fn choose_destination(
        &self,
        source: u8,
        online: &[ApicId],
        cursor: &mut usize,
        loads: &[u32],
    ) -> Result<ApicId, Error> {
        let r = self.get(source).ok_or(Error::NotFound)?;
        if online.is_empty() {
            return Err(Error::NotFound);
        };
        match r.affinity {
            Affinity::Fixed(id) => {
                if online.contains(&id) {
                    Ok(id)
                } else {
                    Err(Error::NotFound)
                }
            }
            Affinity::RoundRobin => {
                let id = online[*cursor % online.len()];
                *cursor = cursor.wrapping_add(1);
                Ok(id)
            }
            Affinity::LowestLoad => {
                if loads.len() != online.len() {
                    return Err(Error::InvalidLength);
                };
                let mut best = 0;
                for i in 1..loads.len() {
                    if loads[i] < loads[best] {
                        best = i
                    }
                }
                Ok(online[best])
            }
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &IrqRoute> {
        self.routes.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applies_override_semantics() {
        let ov = [InterruptOverride {
            bus: 0,
            source: 1,
            gsi: 9,
            flags: 0xf,
        }];
        let mut t = IrqTable::<4>::new(48);
        let r = t.route_isa(1, &ov, Affinity::RoundRobin).unwrap();
        assert_eq!(
            (r.gsi, r.trigger, r.polarity),
            (9, TriggerMode::Level, Polarity::ActiveLow)
        );
    }
    #[test]
    fn refuses_duplicate() {
        let mut t = IrqTable::<2>::new(32);
        t.route_isa(1, &[], Affinity::Fixed(ApicId::XApic(0)))
            .unwrap();
        assert_eq!(
            t.route_isa(1, &[], Affinity::RoundRobin),
            Err(Error::Duplicate)
        );
    }
    #[test]
    fn round_robin_is_deterministic() {
        let mut t = IrqTable::<2>::new(32);
        t.route_isa(2, &[], Affinity::RoundRobin).unwrap();
        let cpus = [ApicId::XApic(0), ApicId::XApic(2)];
        let mut c = 0;
        assert_eq!(t.choose_destination(2, &cpus, &mut c, &[]), Ok(cpus[0]));
        assert_eq!(t.choose_destination(2, &cpus, &mut c, &[]), Ok(cpus[1]));
    }
    #[test]
    fn lowest_load_wins() {
        let mut t = IrqTable::<2>::new(32);
        t.route_isa(2, &[], Affinity::LowestLoad).unwrap();
        let cpus = [ApicId::XApic(0), ApicId::XApic(2)];
        assert_eq!(t.choose_destination(2, &cpus, &mut 0, &[8, 2]), Ok(cpus[1]));
    }
}
