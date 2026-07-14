use crate::topology::ApicId;
use crate::{BoundedVec, Error};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorTables {
    pub gdt_base: u64,
    pub gdt_limit: u16,
    pub idt_base: u64,
    pub idt_limit: u16,
    pub tss_selector: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CpuLifecycle {
    Offline,
    Starting,
    Online,
    Quiescing,
    Halted,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuState {
    pub logical_id: u16,
    pub apic_id: ApicId,
    pub lifecycle: CpuLifecycle,
    pub descriptors: DescriptorTables,
    pub kernel_stack_top: u64,
    pub interrupt_stack_top: u64,
    pub scheduler_ticks: u64,
    pub current_process: Option<u32>,
}
pub struct CpuTable<const N: usize> {
    cpus: BoundedVec<CpuState, N>,
}
impl<const N: usize> CpuTable<N> {
    pub const fn new() -> Self {
        Self {
            cpus: BoundedVec::new(),
        }
    }
    pub fn register(&mut self, cpu: CpuState) -> Result<(), Error> {
        if cpu.kernel_stack_top == 0
            || cpu.interrupt_stack_top == 0
            || cpu.descriptors.gdt_base == 0
            || cpu.descriptors.idt_base == 0
        {
            return Err(Error::InvalidValue);
        }
        if self
            .cpus
            .iter()
            .any(|c| c.logical_id == cpu.logical_id || c.apic_id == cpu.apic_id)
        {
            return Err(Error::Duplicate);
        }
        self.cpus.push(cpu)
    }
    pub fn transition(&mut self, id: u16, to: CpuLifecycle) -> Result<(), Error> {
        let c = self.find_mut(id)?;
        let valid = matches!(
            (c.lifecycle, to),
            (CpuLifecycle::Offline, CpuLifecycle::Starting)
                | (CpuLifecycle::Starting, CpuLifecycle::Online)
                | (CpuLifecycle::Starting, CpuLifecycle::Failed)
                | (CpuLifecycle::Online, CpuLifecycle::Quiescing)
                | (CpuLifecycle::Quiescing, CpuLifecycle::Halted)
        );
        if !valid {
            return Err(Error::Conflict);
        }
        c.lifecycle = to;
        Ok(())
    }
    pub fn account_tick(&mut self, id: u16, process: Option<u32>) -> Result<(), Error> {
        let c = self.find_mut(id)?;
        if c.lifecycle != CpuLifecycle::Online {
            return Err(Error::Conflict);
        }
        c.scheduler_ticks = c.scheduler_ticks.saturating_add(1);
        c.current_process = process;
        Ok(())
    }
    pub fn get(&self, id: u16) -> Option<&CpuState> {
        self.cpus.iter().find(|c| c.logical_id == id)
    }
    fn find_mut(&mut self, id: u16) -> Result<&mut CpuState, Error> {
        let i = (0..self.cpus.len())
            .find(|&i| {
                self.cpus
                    .get(i)
                    .map(|c| c.logical_id == id)
                    .unwrap_or(false)
            })
            .ok_or(Error::NotFound)?;
        Ok(self.cpus.get_mut(i).unwrap())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn cpu() -> CpuState {
        CpuState {
            logical_id: 1,
            apic_id: ApicId::XApic(2),
            lifecycle: CpuLifecycle::Offline,
            descriptors: DescriptorTables {
                gdt_base: 1,
                gdt_limit: 63,
                idt_base: 2,
                idt_limit: 4095,
                tss_selector: 40,
            },
            kernel_stack_top: 0x1000,
            interrupt_stack_top: 0x2000,
            scheduler_ticks: 0,
            current_process: None,
        }
    }
    #[test]
    fn lifecycle_is_strict() {
        let mut t = CpuTable::<2>::new();
        t.register(cpu()).unwrap();
        assert_eq!(t.transition(1, CpuLifecycle::Online), Err(Error::Conflict));
        t.transition(1, CpuLifecycle::Starting).unwrap();
        t.transition(1, CpuLifecycle::Online).unwrap();
        t.account_tick(1, Some(7)).unwrap();
        assert_eq!(t.get(1).unwrap().current_process, Some(7));
    }
    #[test]
    fn rejects_invalid_descriptors() {
        let mut c = cpu();
        c.descriptors.gdt_base = 0;
        assert_eq!(CpuTable::<1>::new().register(c), Err(Error::InvalidValue));
    }
}
