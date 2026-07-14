//! Bounded, capability-gated agent loop. Mutations cannot be marked complete
//! without explicit approval, execution evidence and verification.

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Tool {
    ReadFile = 0,
    WriteFile = 1,
    ListDirectory = 2,
    RunProgram = 3,
    Network = 4,
    InstallPackage = 5,
    SystemControl = 6,
    BuildProject = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolSet(u64);
impl ToolSet {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self((1 << 8) - 1);
    pub const fn one(tool: Tool) -> Self {
        Self(1u64 << tool as u8)
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
    pub const fn contains(self, tool: Tool) -> bool {
        self.0 & (1u64 << tool as u8) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Idle,
    Thinking,
    AwaitingApproval,
    ReadyToExecute,
    AwaitingVerification,
    RollbackRequired,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolCall {
    pub id: u32,
    pub tool: Tool,
    pub arguments_hash: u64,
    pub mutating: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Checkpoint {
    pub sequence: u32,
    pub call_id: u32,
    pub before_hash: u64,
    pub result_hash: u64,
    pub verified: bool,
}
impl Checkpoint {
    pub const EMPTY: Self = Self {
        sequence: 0,
        call_id: 0,
        before_hash: 0,
        result_hash: 0,
        verified: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verification {
    Accepted,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Policy {
    pub tools: ToolSet,
    pub require_mutation_approval: bool,
    pub require_verification: bool,
}

pub struct AgentLoop<const MAX_STEPS: usize, const MAX_CHECKPOINTS: usize> {
    policy: Policy,
    phase: Phase,
    objective_hash: u64,
    world_hash: u64,
    steps: usize,
    next_call_id: u32,
    active: Option<ToolCall>,
    pending_result: u64,
    checkpoints: [Checkpoint; MAX_CHECKPOINTS],
    checkpoint_count: usize,
}

impl<const MAX_STEPS: usize, const MAX_CHECKPOINTS: usize> AgentLoop<MAX_STEPS, MAX_CHECKPOINTS> {
    pub const fn new(policy: Policy) -> Self {
        Self {
            policy,
            phase: Phase::Idle,
            objective_hash: 0,
            world_hash: 0,
            steps: 0,
            next_call_id: 1,
            active: None,
            pending_result: 0,
            checkpoints: [Checkpoint::EMPTY; MAX_CHECKPOINTS],
            checkpoint_count: 0,
        }
    }
    pub const fn phase(&self) -> Phase {
        self.phase
    }
    pub const fn objective_hash(&self) -> u64 {
        self.objective_hash
    }
    pub const fn world_hash(&self) -> u64 {
        self.world_hash
    }
    pub const fn steps(&self) -> usize {
        self.steps
    }
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints[..self.checkpoint_count]
    }

    pub fn start(&mut self, objective_hash: u64, initial_world_hash: u64) -> Result<()> {
        if self.phase != Phase::Idle || MAX_STEPS == 0 {
            return Err(Error::InvalidTransition);
        }
        self.objective_hash = objective_hash;
        self.world_hash = initial_world_hash;
        self.phase = Phase::Thinking;
        Ok(())
    }

    pub fn propose(&mut self, tool: Tool, arguments_hash: u64, mutating: bool) -> Result<ToolCall> {
        if self.phase != Phase::Thinking {
            return Err(Error::InvalidTransition);
        }
        if self.steps >= MAX_STEPS {
            self.phase = Phase::Failed;
            return Err(Error::StepLimit);
        }
        if !self.policy.tools.contains(tool) {
            return Err(Error::PermissionDenied);
        }
        let call = ToolCall {
            id: self.next_call_id,
            tool,
            arguments_hash,
            mutating,
        };
        self.next_call_id = self.next_call_id.wrapping_add(1);
        self.active = Some(call);
        self.steps += 1;
        self.phase = if mutating && self.policy.require_mutation_approval {
            Phase::AwaitingApproval
        } else {
            Phase::ReadyToExecute
        };
        Ok(call)
    }

    pub fn approve(&mut self, call_id: u32, approved: bool) -> Result<()> {
        self.require_call(call_id, Phase::AwaitingApproval)?;
        if approved {
            self.phase = Phase::ReadyToExecute;
        } else {
            self.active = None;
            self.phase = Phase::Thinking;
        }
        Ok(())
    }

    pub fn execution_result(
        &mut self,
        call_id: u32,
        result_hash: u64,
        success: bool,
    ) -> Result<()> {
        self.require_call(call_id, Phase::ReadyToExecute)?;
        if !success {
            self.active = None;
            self.phase = Phase::RollbackRequired;
            return Ok(());
        }
        self.pending_result = result_hash;
        if self.policy.require_verification {
            self.phase = Phase::AwaitingVerification;
        } else {
            self.record_checkpoint(call_id, result_hash, true)?;
            self.world_hash = result_hash;
            self.active = None;
            self.phase = Phase::Thinking;
        }
        Ok(())
    }

    pub fn verify(&mut self, call_id: u32, verdict: Verification) -> Result<()> {
        self.require_call(call_id, Phase::AwaitingVerification)?;
        let accepted = verdict == Verification::Accepted;
        self.record_checkpoint(call_id, self.pending_result, accepted)?;
        self.active = None;
        if accepted {
            self.world_hash = self.pending_result;
            self.phase = Phase::Thinking;
        } else {
            self.phase = Phase::RollbackRequired;
        }
        Ok(())
    }

    pub fn rollback(&mut self, restored_world_hash: u64) -> Result<()> {
        if self.phase != Phase::RollbackRequired {
            return Err(Error::InvalidTransition);
        }
        self.world_hash = restored_world_hash;
        self.pending_result = 0;
        self.active = None;
        self.phase = Phase::Thinking;
        Ok(())
    }

    pub fn complete(&mut self, final_world_hash: u64) -> Result<()> {
        if self.phase != Phase::Thinking || final_world_hash != self.world_hash {
            return Err(Error::InvalidTransition);
        }
        if self.checkpoints[..self.checkpoint_count]
            .iter()
            .any(|c| !c.verified)
        {
            return Err(Error::InvalidTransition);
        }
        self.phase = Phase::Completed;
        Ok(())
    }

    fn require_call(&self, call_id: u32, phase: Phase) -> Result<ToolCall> {
        if self.phase != phase {
            return Err(Error::InvalidTransition);
        }
        let call = self.active.ok_or(Error::InvalidTransition)?;
        if call.id != call_id {
            return Err(Error::CallMismatch);
        }
        Ok(call)
    }
    fn record_checkpoint(&mut self, call_id: u32, result_hash: u64, verified: bool) -> Result<()> {
        if self.checkpoint_count >= MAX_CHECKPOINTS {
            self.phase = Phase::Failed;
            return Err(Error::CheckpointLimit);
        }
        self.checkpoints[self.checkpoint_count] = Checkpoint {
            sequence: self.checkpoint_count as u32 + 1,
            call_id,
            before_hash: self.world_hash,
            result_hash,
            verified,
        };
        self.checkpoint_count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            tools: ToolSet::one(Tool::ReadFile).union(ToolSet::one(Tool::WriteFile)),
            require_mutation_approval: true,
            require_verification: true,
        }
    }
    #[test]
    fn verified_mutation_happy_path() {
        let mut agent = AgentLoop::<4, 4>::new(policy());
        agent.start(10, 100).unwrap();
        let call = agent.propose(Tool::WriteFile, 20, true).unwrap();
        assert_eq!(agent.phase(), Phase::AwaitingApproval);
        agent.approve(call.id, true).unwrap();
        agent.execution_result(call.id, 101, true).unwrap();
        assert_eq!(agent.phase(), Phase::AwaitingVerification);
        agent.verify(call.id, Verification::Accepted).unwrap();
        assert_eq!(agent.world_hash(), 101);
        assert_eq!(agent.checkpoints()[0].before_hash, 100);
        agent.complete(101).unwrap();
        assert_eq!(agent.phase(), Phase::Completed);
    }
    #[test]
    fn denied_tool_never_executes() {
        let mut agent = AgentLoop::<4, 4>::new(policy());
        agent.start(1, 2).unwrap();
        assert_eq!(
            agent.propose(Tool::SystemControl, 3, true),
            Err(Error::PermissionDenied)
        );
        assert_eq!(agent.steps(), 0);
    }
    #[test]
    fn rejection_requires_rollback() {
        let mut agent = AgentLoop::<4, 4>::new(policy());
        agent.start(1, 50).unwrap();
        let call = agent.propose(Tool::ReadFile, 2, false).unwrap();
        agent.execution_result(call.id, 60, true).unwrap();
        agent.verify(call.id, Verification::Rejected).unwrap();
        assert_eq!(agent.phase(), Phase::RollbackRequired);
        agent.rollback(50).unwrap();
        assert_eq!(agent.phase(), Phase::Thinking);
        assert_eq!(agent.world_hash(), 50);
    }
    #[test]
    fn approval_rejection_returns_to_thinking() {
        let mut agent = AgentLoop::<4, 4>::new(policy());
        agent.start(1, 2).unwrap();
        let call = agent.propose(Tool::WriteFile, 3, true).unwrap();
        agent.approve(call.id, false).unwrap();
        assert_eq!(agent.phase(), Phase::Thinking);
    }
    #[test]
    fn call_ids_prevent_confused_deputy() {
        let mut agent = AgentLoop::<4, 4>::new(policy());
        agent.start(1, 2).unwrap();
        let call = agent.propose(Tool::ReadFile, 3, false).unwrap();
        assert_eq!(
            agent.execution_result(call.id + 1, 4, true),
            Err(Error::CallMismatch)
        );
    }
    #[test]
    fn bounded_steps_fail_closed() {
        let mut agent = AgentLoop::<1, 2>::new(policy());
        agent.start(1, 2).unwrap();
        let call = agent.propose(Tool::ReadFile, 3, false).unwrap();
        agent.execution_result(call.id, 4, true).unwrap();
        agent.verify(call.id, Verification::Accepted).unwrap();
        assert_eq!(
            agent.propose(Tool::ReadFile, 5, false),
            Err(Error::StepLimit)
        );
        assert_eq!(agent.phase(), Phase::Failed);
    }
}
