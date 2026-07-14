use crate::topology::ApicId;
use crate::{BoundedVec, Error};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApState {
    Discovered,
    InitAsserted,
    InitDeasserted,
    Sipi1Sent,
    Sipi2Sent,
    Online,
    Failed(ApFailure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApFailure {
    InitTimeout,
    StartupTimeout,
    Rejected,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApAction {
    SendInitAssert(ApicId),
    SendInitDeassert(ApicId),
    SendSipi { destination: ApicId, vector: u8 },
    WaitUntil(u64),
    Complete(ApicId),
    Failed(ApicId, ApFailure),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ApRecord {
    id: ApicId,
    state: ApState,
    deadline: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartupConfig {
    pub init_assert_ticks: u64,
    pub init_deassert_ticks: u64,
    pub sipi_gap_ticks: u64,
    pub online_timeout_ticks: u64,
    pub trampoline_vector: u8,
}
impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            init_assert_ticks: 1,
            init_deassert_ticks: 10,
            sipi_gap_ticks: 1,
            online_timeout_ticks: 200,
            trampoline_vector: 8,
        }
    }
}
pub struct ApStartup<const N: usize> {
    aps: BoundedVec<ApRecord, N>,
    cfg: StartupConfig,
}
impl<const N: usize> ApStartup<N> {
    pub const fn new(cfg: StartupConfig) -> Self {
        Self {
            aps: BoundedVec::new(),
            cfg,
        }
    }
    pub fn add(&mut self, id: ApicId) -> Result<(), Error> {
        if self.aps.iter().any(|a| a.id == id) {
            return Err(Error::Duplicate);
        }
        self.aps.push(ApRecord {
            id,
            state: ApState::Discovered,
            deadline: 0,
        })
    }
    pub fn begin(&mut self, id: ApicId, now: u64) -> Result<ApAction, Error> {
        let init_assert_ticks = self.cfg.init_assert_ticks;
        let a = self.find_mut(id)?;
        if a.state != ApState::Discovered {
            return Err(Error::Conflict);
        }
        a.state = ApState::InitAsserted;
        a.deadline = now.saturating_add(init_assert_ticks);
        Ok(ApAction::SendInitAssert(id))
    }
    pub fn advance(&mut self, id: ApicId, now: u64) -> Result<ApAction, Error> {
        let cfg = self.cfg;
        let a = self.find_mut(id)?;
        match a.state {
            ApState::InitAsserted if now >= a.deadline => {
                a.state = ApState::InitDeasserted;
                a.deadline = now.saturating_add(cfg.init_deassert_ticks);
                Ok(ApAction::SendInitDeassert(id))
            }
            ApState::InitDeasserted if now >= a.deadline => {
                a.state = ApState::Sipi1Sent;
                a.deadline = now.saturating_add(cfg.sipi_gap_ticks);
                Ok(ApAction::SendSipi {
                    destination: id,
                    vector: cfg.trampoline_vector,
                })
            }
            ApState::Sipi1Sent if now >= a.deadline => {
                a.state = ApState::Sipi2Sent;
                a.deadline = now.saturating_add(cfg.online_timeout_ticks);
                Ok(ApAction::SendSipi {
                    destination: id,
                    vector: cfg.trampoline_vector,
                })
            }
            ApState::Sipi2Sent if now >= a.deadline => {
                a.state = ApState::Failed(ApFailure::StartupTimeout);
                Ok(ApAction::Failed(id, ApFailure::StartupTimeout))
            }
            ApState::Online => Ok(ApAction::Complete(id)),
            ApState::Failed(f) => Ok(ApAction::Failed(id, f)),
            _ => Ok(ApAction::WaitUntil(a.deadline)),
        }
    }
    pub fn acknowledge_online(&mut self, id: ApicId) -> Result<ApAction, Error> {
        let a = self.find_mut(id)?;
        if !matches!(a.state, ApState::Sipi1Sent | ApState::Sipi2Sent) {
            return Err(Error::Conflict);
        }
        a.state = ApState::Online;
        Ok(ApAction::Complete(id))
    }
    pub fn state(&self, id: ApicId) -> Option<ApState> {
        self.aps.iter().find(|a| a.id == id).map(|a| a.state)
    }
    fn find_mut(&mut self, id: ApicId) -> Result<&mut ApRecord, Error> {
        let i = (0..self.aps.len())
            .find(|&i| self.aps.get(i).map(|a| a.id == id).unwrap_or(false))
            .ok_or(Error::NotFound)?;
        Ok(self.aps.get_mut(i).unwrap())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_init_sipi_sequence() {
        let id = ApicId::XApic(2);
        let mut s = ApStartup::<2>::new(StartupConfig::default());
        s.add(id).unwrap();
        assert_eq!(s.begin(id, 0), Ok(ApAction::SendInitAssert(id)));
        assert!(matches!(
            s.advance(id, 1),
            Ok(ApAction::SendInitDeassert(_))
        ));
        assert!(matches!(s.advance(id, 11), Ok(ApAction::SendSipi { .. })));
        assert!(matches!(s.advance(id, 12), Ok(ApAction::SendSipi { .. })));
        assert_eq!(s.acknowledge_online(id), Ok(ApAction::Complete(id)));
    }
    #[test]
    fn times_out_boundedly() {
        let id = ApicId::XApic(3);
        let mut s = ApStartup::<1>::new(StartupConfig::default());
        s.add(id).unwrap();
        s.begin(id, 0).unwrap();
        s.advance(id, 1).unwrap();
        s.advance(id, 11).unwrap();
        s.advance(id, 12).unwrap();
        assert_eq!(
            s.advance(id, 212),
            Ok(ApAction::Failed(id, ApFailure::StartupTimeout))
        );
    }
    #[test]
    fn rejects_early_ack() {
        let id = ApicId::XApic(3);
        let mut s = ApStartup::<1>::new(StartupConfig::default());
        s.add(id).unwrap();
        assert_eq!(s.acknowledge_online(id), Err(Error::Conflict));
    }
}
