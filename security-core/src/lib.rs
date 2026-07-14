#![no_std]
#![forbid(unsafe_code)]

//! Allocation-free security primitives and policy state machines for Nova OS.
//! Cryptographic trust roots are injected by the platform; this crate never
//! reads host entropy or talks to firmware by itself.

mod hash;

pub use hash::{Sha256, sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityError {
    НедостатньоЕнтропії,
    НесправнеДжерелоЕнтропії,
    ПотрібнеПовторнеЗасівання,
    БуферЗаповнений,
    НекоректнийІндекс,
    ДоступЗаборонено,
    ПеревищеноЛіміт,
    ПідписНедійсний,
    ХешНеЗбігається,
    ВідкатВерсії,
    НеправильнийСтан,
    ЛанцюгАудитуПошкоджений,
}

impl SecurityError {
    pub const fn message_uk(self) -> &'static str {
        match self {
            Self::НедостатньоЕнтропії => {
                "Недостатньо ентропії для безпечного запуску"
            }
            Self::НесправнеДжерелоЕнтропії => {
                "Джерело ентропії не пройшло перевірку здоров'я"
            }
            Self::ПотрібнеПовторнеЗасівання => {
                "Потрібне повторне засівання генератора"
            }
            Self::БуферЗаповнений => "Захищений буфер заповнений",
            Self::НекоректнийІндекс => "Некоректний індекс",
            Self::ДоступЗаборонено => "Політика безпеки заборонила операцію",
            Self::ПеревищеноЛіміт => "Перевищено дозволену частоту операцій",
            Self::ПідписНедійсний => "Цифровий підпис недійсний",
            Self::ХешНеЗбігається => "Контрольний хеш не збігається",
            Self::ВідкатВерсії => "Заборонено відкат до старої версії",
            Self::НеправильнийСтан => "Операція недоступна у поточному стані",
            Self::ЛанцюгАудитуПошкоджений => {
                "Ланцюг журналу аудиту пошкоджений"
            }
        }
    }
}

/// Minimal start-up and continuous health checks for injected entropy.
#[derive(Debug, Clone)]
pub struct EntropyHealth {
    previous: [u8; 32],
    has_previous: bool,
}

impl EntropyHealth {
    pub const fn new() -> Self {
        Self {
            previous: [0; 32],
            has_previous: false,
        }
    }

    pub fn examine(&mut self, sample: &[u8]) -> Result<(), SecurityError> {
        if sample.len() < 32 {
            return Err(SecurityError::НедостатньоЕнтропії);
        }
        let mut any_nonzero = false;
        let mut any_change = false;
        for (index, byte) in sample[..32].iter().copied().enumerate() {
            any_nonzero |= byte != 0;
            any_change |= index > 0 && byte != sample[index - 1];
        }
        if !any_nonzero || !any_change {
            return Err(SecurityError::НесправнеДжерелоЕнтропії);
        }
        if self.has_previous && constant_time_eq(&self.previous, &sample[..32]) {
            return Err(SecurityError::НесправнеДжерелоЕнтропії);
        }
        self.previous.copy_from_slice(&sample[..32]);
        self.has_previous = true;
        Ok(())
    }
}

impl Default for EntropyHealth {
    fn default() -> Self {
        Self::new()
    }
}

/// Deterministic generator whose seed material must come from a platform CSPRNG.
/// The construction uses domain-separated SHA-256 state evolution.
pub struct HashDrbg {
    state: [u8; 32],
    generation: u64,
    last_block: [u8; 32],
    has_last: bool,
    health: EntropyHealth,
}

impl HashDrbg {
    pub const RESEED_INTERVAL: u64 = 1 << 20;

    pub fn instantiate(
        entropy: &[u8],
        nonce: &[u8],
        personalization: &[u8],
    ) -> Result<Self, SecurityError> {
        let mut health = EntropyHealth::new();
        health.examine(entropy)?;
        let state = hash_parts(&[b"NovaOS HashDRBG v1", entropy, nonce, personalization]);
        Ok(Self {
            state,
            generation: 0,
            last_block: [0; 32],
            has_last: false,
            health,
        })
    }

    pub fn reseed(&mut self, entropy: &[u8], additional: &[u8]) -> Result<(), SecurityError> {
        self.health.examine(entropy)?;
        self.state = hash_parts(&[b"NovaOS reseed", &self.state, entropy, additional]);
        self.generation = 0;
        self.has_last = false;
        Ok(())
    }

    pub fn fill(&mut self, output: &mut [u8], additional: &[u8]) -> Result<(), SecurityError> {
        if self.generation >= Self::RESEED_INTERVAL {
            return Err(SecurityError::ПотрібнеПовторнеЗасівання);
        }
        if !additional.is_empty() {
            self.state = hash_parts(&[b"NovaOS additional", &self.state, additional]);
        }
        let mut written = 0;
        let mut block_index = 0u64;
        while written < output.len() {
            let block = hash_parts(&[
                b"NovaOS generate",
                &self.state,
                &self.generation.to_be_bytes(),
                &block_index.to_be_bytes(),
            ]);
            if self.has_last && constant_time_eq(&block, &self.last_block) {
                output.fill(0);
                return Err(SecurityError::НесправнеДжерелоЕнтропії);
            }
            let take = (output.len() - written).min(32);
            output[written..written + take].copy_from_slice(&block[..take]);
            self.last_block = block;
            self.has_last = true;
            written += take;
            block_index = block_index.wrapping_add(1);
        }
        self.state = hash_parts(&[b"NovaOS update", &self.state, &self.last_block]);
        self.generation += 1;
        Ok(())
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootEventKind {
    Прошивка,
    Завантажувач,
    Ядро,
    Політика,
    Драйвер,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootEvent {
    pub sequence: u32,
    pub pcr: u8,
    pub kind: BootEventKind,
    pub digest: [u8; 32],
}

const EMPTY_EVENT: BootEvent = BootEvent {
    sequence: 0,
    pcr: 0,
    kind: BootEventKind::Прошивка,
    digest: [0; 32],
};

/// Fixed-capacity measured-boot event log with TPM-compatible PCR semantics:
/// `PCR := SHA256(PCR || event_digest)`.
pub struct MeasuredBoot<const EVENTS: usize, const PCRS: usize> {
    pcrs: [[u8; 32]; PCRS],
    events: [BootEvent; EVENTS],
    len: usize,
}

impl<const EVENTS: usize, const PCRS: usize> MeasuredBoot<EVENTS, PCRS> {
    pub const fn new() -> Self {
        Self {
            pcrs: [[0; 32]; PCRS],
            events: [EMPTY_EVENT; EVENTS],
            len: 0,
        }
    }

    pub fn extend(
        &mut self,
        pcr: usize,
        kind: BootEventKind,
        digest: [u8; 32],
    ) -> Result<[u8; 32], SecurityError> {
        if pcr >= PCRS {
            return Err(SecurityError::НекоректнийІндекс);
        }
        if self.len == EVENTS {
            return Err(SecurityError::БуферЗаповнений);
        }
        self.pcrs[pcr] = hash_parts(&[&self.pcrs[pcr], &digest]);
        self.events[self.len] = BootEvent {
            sequence: self.len as u32,
            pcr: pcr as u8,
            kind,
            digest,
        };
        self.len += 1;
        Ok(self.pcrs[pcr])
    }

    pub fn measure(
        &mut self,
        pcr: usize,
        kind: BootEventKind,
        bytes: &[u8],
    ) -> Result<[u8; 32], SecurityError> {
        self.extend(pcr, kind, sha256(bytes))
    }

    pub const fn pcr(&self, index: usize) -> Option<&[u8; 32]> {
        if index < PCRS {
            Some(&self.pcrs[index])
        } else {
            None
        }
    }
    pub const fn events(&self) -> &[BootEvent] {
        self.events.split_at(self.len).0
    }
}

impl<const EVENTS: usize, const PCRS: usize> Default for MeasuredBoot<EVENTS, PCRS> {
    fn default() -> Self {
        Self::new()
    }
}

pub trait SignatureVerifier {
    fn verify(&self, key_id: u32, digest: &[u8; 32], signature: &[u8]) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecureManifest<'a> {
    pub version: u64,
    pub minimum_version: u64,
    pub key_id: u32,
    pub image_digest: [u8; 32],
    pub signature: &'a [u8],
}

impl SecureManifest<'_> {
    pub fn signed_digest(&self) -> [u8; 32] {
        hash_parts(&[
            b"NovaOS manifest v1",
            &self.version.to_be_bytes(),
            &self.minimum_version.to_be_bytes(),
            &self.key_id.to_be_bytes(),
            &self.image_digest,
        ])
    }
}

pub fn verify_manifest(
    manifest: &SecureManifest<'_>,
    image: &[u8],
    installed_floor: u64,
    verifier: &impl SignatureVerifier,
) -> Result<(), SecurityError> {
    if manifest.version < installed_floor || manifest.version < manifest.minimum_version {
        return Err(SecurityError::ВідкатВерсії);
    }
    if !constant_time_eq(&sha256(image), &manifest.image_digest) {
        return Err(SecurityError::ХешНеЗбігається);
    }
    if !verifier.verify(
        manifest.key_id,
        &manifest.signed_digest(),
        manifest.signature,
    ) {
        return Err(SecurityError::ПідписНедійсний);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleAction {
    Дозволити,
    Заборонити,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyscallRule {
    pub syscall: u16,
    pub action: RuleAction,
}

const EMPTY_RULE: SyscallRule = SyscallRule {
    syscall: 0,
    action: RuleAction::Заборонити,
};

pub struct SandboxPolicy<const N: usize> {
    rules: [SyscallRule; N],
    len: usize,
    default: RuleAction,
    locked: bool,
}

impl<const N: usize> SandboxPolicy<N> {
    pub const fn deny_by_default() -> Self {
        Self {
            rules: [EMPTY_RULE; N],
            len: 0,
            default: RuleAction::Заборонити,
            locked: false,
        }
    }
    pub fn add(&mut self, rule: SyscallRule) -> Result<(), SecurityError> {
        if self.locked {
            return Err(SecurityError::НеправильнийСтан);
        }
        if self.len == N {
            return Err(SecurityError::БуферЗаповнений);
        }
        self.rules[self.len] = rule;
        self.len += 1;
        Ok(())
    }
    pub fn lock(&mut self) {
        self.locked = true;
    }
    pub fn check(&self, syscall: u16) -> Result<(), SecurityError> {
        let action = self.rules[..self.len]
            .iter()
            .rev()
            .find(|rule| rule.syscall == syscall)
            .map_or(self.default, |rule| rule.action);
        match action {
            RuleAction::Дозволити => Ok(()),
            RuleAction::Заборонити => Err(SecurityError::ДоступЗаборонено),
        }
    }
    pub const fn is_locked(&self) -> bool {
        self.locked
    }
}

/// Integer token bucket. The caller supplies a monotonic tick, keeping the core
/// independent from timers and host APIs.
pub struct RateLimiter {
    capacity: u32,
    tokens: u32,
    refill_per_tick: u32,
    last_tick: u64,
}

impl RateLimiter {
    pub const fn new(capacity: u32, refill_per_tick: u32, now: u64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_per_tick,
            last_tick: now,
        }
    }
    pub fn allow(&mut self, cost: u32, now: u64) -> Result<(), SecurityError> {
        let elapsed = now.saturating_sub(self.last_tick);
        let refill = elapsed
            .saturating_mul(self.refill_per_tick as u64)
            .min(u32::MAX as u64) as u32;
        self.tokens = self.tokens.saturating_add(refill).min(self.capacity);
        self.last_tick = self.last_tick.max(now);
        if cost > self.tokens {
            Err(SecurityError::ПеревищеноЛіміт)
        } else {
            self.tokens -= cost;
            Ok(())
        }
    }
    pub const fn available(&self) -> u32 {
        self.tokens
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRecord {
    pub sequence: u64,
    pub tick: u64,
    pub actor: u32,
    pub action: u16,
    pub outcome: u8,
    pub payload_digest: [u8; 32],
    pub chain_digest: [u8; 32],
}

const EMPTY_AUDIT: AuditRecord = AuditRecord {
    sequence: 0,
    tick: 0,
    actor: 0,
    action: 0,
    outcome: 0,
    payload_digest: [0; 32],
    chain_digest: [0; 32],
};

/// Append-only fixed-capacity log. Records are exposed read-only and chained.
pub struct AuditLog<const N: usize> {
    records: [AuditRecord; N],
    len: usize,
    root: [u8; 32],
}

impl<const N: usize> AuditLog<N> {
    pub const fn new() -> Self {
        Self {
            records: [EMPTY_AUDIT; N],
            len: 0,
            root: [0; 32],
        }
    }
    pub fn append(
        &mut self,
        tick: u64,
        actor: u32,
        action: u16,
        outcome: u8,
        payload: &[u8],
    ) -> Result<[u8; 32], SecurityError> {
        if self.len == N {
            return Err(SecurityError::БуферЗаповнений);
        }
        let sequence = self.len as u64;
        let payload_digest = sha256(payload);
        let chain_digest = audit_digest(
            self.root,
            sequence,
            tick,
            actor,
            action,
            outcome,
            payload_digest,
        );
        self.records[self.len] = AuditRecord {
            sequence,
            tick,
            actor,
            action,
            outcome,
            payload_digest,
            chain_digest,
        };
        self.root = chain_digest;
        self.len += 1;
        Ok(chain_digest)
    }
    pub fn verify(&self) -> Result<[u8; 32], SecurityError> {
        let mut previous = [0; 32];
        for (index, record) in self.records[..self.len].iter().enumerate() {
            if record.sequence != index as u64
                || audit_digest(
                    previous,
                    record.sequence,
                    record.tick,
                    record.actor,
                    record.action,
                    record.outcome,
                    record.payload_digest,
                ) != record.chain_digest
            {
                return Err(SecurityError::ЛанцюгАудитуПошкоджений);
            }
            previous = record.chain_digest;
        }
        if previous != self.root {
            return Err(SecurityError::ЛанцюгАудитуПошкоджений);
        }
        Ok(previous)
    }
    pub const fn records(&self) -> &[AuditRecord] {
        self.records.split_at(self.len).0
    }
    pub const fn root(&self) -> [u8; 32] {
        self.root
    }
}

impl<const N: usize> Default for AuditLog<N> {
    fn default() -> Self {
        Self::new()
    }
}

fn audit_digest(
    previous: [u8; 32],
    sequence: u64,
    tick: u64,
    actor: u32,
    action: u16,
    outcome: u8,
    payload: [u8; 32],
) -> [u8; 32] {
    hash_parts(&[
        b"NovaOS audit v1",
        &previous,
        &sequence.to_be_bytes(),
        &tick.to_be_bytes(),
        &actor.to_be_bytes(),
        &action.to_be_bytes(),
        &[outcome],
        &payload,
    ])
}

/// Fixed-size secret with explicit clearing and safe clearing on drop.
/// Platform integration may additionally use protected/locked pages.
pub struct Zeroizing<const N: usize> {
    bytes: [u8; N],
}

impl<const N: usize> Zeroizing<N> {
    pub const fn new(bytes: [u8; N]) -> Self {
        Self { bytes }
    }
    pub const fn expose(&self) -> &[u8; N] {
        &self.bytes
    }
    pub fn expose_mut(&mut self) -> &mut [u8; N] {
        &mut self.bytes
    }
    pub fn clear(&mut self) {
        self.bytes.fill(0);
    }
}

impl<const N: usize> Drop for Zeroizing<N> {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateState {
    Очікування,
    Перевірено,
    Підготовлено,
    Активовано,
    Підтверджено,
    Відкочено,
}

pub struct UpdateAuthorization {
    state: UpdateState,
    installed_floor: u64,
    candidate: u64,
    digest: [u8; 32],
}

impl UpdateAuthorization {
    pub const fn new(installed_floor: u64) -> Self {
        Self {
            state: UpdateState::Очікування,
            installed_floor,
            candidate: 0,
            digest: [0; 32],
        }
    }
    pub fn authorize(&mut self, version: u64, digest: [u8; 32]) -> Result<(), SecurityError> {
        if self.state != UpdateState::Очікування {
            return Err(SecurityError::НеправильнийСтан);
        }
        if version < self.installed_floor {
            return Err(SecurityError::ВідкатВерсії);
        }
        self.candidate = version;
        self.digest = digest;
        self.state = UpdateState::Перевірено;
        Ok(())
    }
    pub fn stage(&mut self, observed_digest: &[u8; 32]) -> Result<(), SecurityError> {
        if self.state != UpdateState::Перевірено {
            return Err(SecurityError::НеправильнийСтан);
        }
        if !constant_time_eq(&self.digest, observed_digest) {
            return Err(SecurityError::ХешНеЗбігається);
        }
        self.state = UpdateState::Підготовлено;
        Ok(())
    }
    pub fn activate(&mut self) -> Result<(), SecurityError> {
        self.transition(UpdateState::Підготовлено, UpdateState::Активовано)
    }
    pub fn confirm(&mut self) -> Result<(), SecurityError> {
        self.transition(UpdateState::Активовано, UpdateState::Підтверджено)?;
        self.installed_floor = self.installed_floor.max(self.candidate);
        Ok(())
    }
    pub fn rollback(&mut self) -> Result<(), SecurityError> {
        if self.state != UpdateState::Підготовлено
            && self.state != UpdateState::Активовано
        {
            return Err(SecurityError::НеправильнийСтан);
        }
        self.state = UpdateState::Відкочено;
        Ok(())
    }
    fn transition(&mut self, from: UpdateState, to: UpdateState) -> Result<(), SecurityError> {
        if self.state != from {
            Err(SecurityError::НеправильнийСтан)
        } else {
            self.state = to;
            Ok(())
        }
    }
    pub const fn state(&self) -> UpdateState {
        self.state
    }
    pub const fn installed_floor(&self) -> u64 {
        self.installed_floor
    }
}

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for index in 0..left.len() {
        difference |= left[index] ^ right[index];
    }
    difference == 0
}

fn hash_parts(parts: &[&[u8]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(&((*part).len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entropy(seed: u8) -> [u8; 32] {
        core::array::from_fn(|index| seed.wrapping_add(index as u8).wrapping_add(1))
    }

    #[test]
    fn diagnostics_are_ukrainian() {
        assert!(
            SecurityError::ДоступЗаборонено
                .message_uk()
                .contains("заборонила")
        );
    }
    #[test]
    fn entropy_health_rejects_short_zero_stuck_and_repeated() {
        let mut h = EntropyHealth::new();
        assert_eq!(h.examine(&[1; 12]), Err(SecurityError::НедостатньоЕнтропії));
        assert_eq!(
            h.examine(&[0; 32]),
            Err(SecurityError::НесправнеДжерелоЕнтропії)
        );
        assert_eq!(
            h.examine(&[7; 32]),
            Err(SecurityError::НесправнеДжерелоЕнтропії)
        );
        let e = entropy(3);
        assert_eq!(h.examine(&e), Ok(()));
        assert_eq!(h.examine(&e), Err(SecurityError::НесправнеДжерелоЕнтропії));
    }
    #[test]
    fn drbg_is_deterministic_but_evolves() {
        let e = entropy(9);
        let mut a = HashDrbg::instantiate(&e, b"nonce", b"nova").unwrap();
        let mut b = HashDrbg::instantiate(&e, b"nonce", b"nova").unwrap();
        let mut first = [0; 70];
        let mut same = [0; 70];
        a.fill(&mut first, b"").unwrap();
        b.fill(&mut same, b"").unwrap();
        assert_eq!(first, same);
        let mut second = [0; 70];
        a.fill(&mut second, b"").unwrap();
        assert_ne!(first, second);
        assert_eq!(a.generation(), 2);
    }
    #[test]
    fn drbg_reseed_changes_stream() {
        let mut d = HashDrbg::instantiate(&entropy(1), b"n", b"").unwrap();
        let mut before = [0; 32];
        d.fill(&mut before, b"").unwrap();
        d.reseed(&entropy(100), b"event").unwrap();
        let mut after = [0; 32];
        d.fill(&mut after, b"").unwrap();
        assert_ne!(before, after);
    }
    #[test]
    fn measured_boot_extends_pcr_and_logs() {
        let mut boot = MeasuredBoot::<4, 8>::new();
        let first = boot.measure(4, BootEventKind::Ядро, b"kernel").unwrap();
        let second = boot.measure(4, BootEventKind::Політика, b"policy").unwrap();
        assert_ne!(first, second);
        assert_eq!(boot.events().len(), 2);
        assert_eq!(boot.pcr(4), Some(&second));
    }
    #[test]
    fn measured_boot_checks_bounds_and_capacity() {
        let mut boot = MeasuredBoot::<1, 1>::new();
        assert_eq!(
            boot.measure(2, BootEventKind::Ядро, b"x"),
            Err(SecurityError::НекоректнийІндекс)
        );
        boot.measure(0, BootEventKind::Ядро, b"x").unwrap();
        assert_eq!(
            boot.measure(0, BootEventKind::Ядро, b"y"),
            Err(SecurityError::БуферЗаповнений)
        );
    }

    struct TestVerifier;
    impl SignatureVerifier for TestVerifier {
        fn verify(&self, key: u32, digest: &[u8; 32], signature: &[u8]) -> bool {
            key == 7 && signature == &digest[..8]
        }
    }
    #[test]
    fn manifest_verifies_image_signature_and_floor() {
        let image = b"signed Nova image";
        let mut manifest = SecureManifest {
            version: 12,
            minimum_version: 10,
            key_id: 7,
            image_digest: sha256(image),
            signature: &[],
        };
        let digest = manifest.signed_digest();
        manifest.signature = &digest[..8];
        assert_eq!(verify_manifest(&manifest, image, 11, &TestVerifier), Ok(()));
        assert_eq!(
            verify_manifest(&manifest, b"tampered", 11, &TestVerifier),
            Err(SecurityError::ХешНеЗбігається)
        );
        assert_eq!(
            verify_manifest(&manifest, image, 13, &TestVerifier),
            Err(SecurityError::ВідкатВерсії)
        );
    }
    #[test]
    fn sandbox_is_deny_by_default_and_irreversibly_locks() {
        let mut policy = SandboxPolicy::<3>::deny_by_default();
        policy
            .add(SyscallRule {
                syscall: 1,
                action: RuleAction::Дозволити,
            })
            .unwrap();
        assert_eq!(policy.check(1), Ok(()));
        assert_eq!(policy.check(2), Err(SecurityError::ДоступЗаборонено));
        policy.lock();
        assert_eq!(
            policy.add(SyscallRule {
                syscall: 2,
                action: RuleAction::Дозволити
            }),
            Err(SecurityError::НеправильнийСтан)
        );
    }
    #[test]
    fn last_sandbox_rule_wins() {
        let mut policy = SandboxPolicy::<2>::deny_by_default();
        policy
            .add(SyscallRule {
                syscall: 1,
                action: RuleAction::Дозволити,
            })
            .unwrap();
        policy
            .add(SyscallRule {
                syscall: 1,
                action: RuleAction::Заборонити,
            })
            .unwrap();
        assert_eq!(policy.check(1), Err(SecurityError::ДоступЗаборонено));
    }
    #[test]
    fn token_bucket_refills_from_monotonic_ticks() {
        let mut rate = RateLimiter::new(5, 2, 10);
        assert_eq!(rate.allow(5, 10), Ok(()));
        assert_eq!(rate.allow(1, 10), Err(SecurityError::ПеревищеноЛіміт));
        assert_eq!(rate.allow(4, 12), Ok(()));
        assert_eq!(rate.available(), 0);
    }
    #[test]
    fn audit_log_is_deterministic_and_verifiable() {
        let mut log = AuditLog::<4>::new();
        let a = log.append(10, 42, 3, 1, b"open").unwrap();
        let b = log.append(11, 42, 4, 0, b"deny").unwrap();
        assert_ne!(a, b);
        assert_eq!(log.verify(), Ok(b));
        assert_eq!(log.records().len(), 2);
    }
    #[test]
    fn audit_capacity_is_enforced() {
        let mut log = AuditLog::<1>::new();
        log.append(1, 1, 1, 1, b"x").unwrap();
        assert_eq!(
            log.append(2, 1, 1, 1, b"y"),
            Err(SecurityError::БуферЗаповнений)
        );
    }
    #[test]
    fn zeroizing_can_be_explicitly_cleared() {
        let mut secret = Zeroizing::new([0xAA; 16]);
        assert_eq!(secret.expose()[0], 0xAA);
        secret.clear();
        assert_eq!(*secret.expose(), [0; 16]);
    }
    #[test]
    fn update_happy_path_advances_antirollback_floor() {
        let digest = sha256(b"v9");
        let mut update = UpdateAuthorization::new(7);
        update.authorize(9, digest).unwrap();
        update.stage(&digest).unwrap();
        update.activate().unwrap();
        update.confirm().unwrap();
        assert_eq!(update.state(), UpdateState::Підтверджено);
        assert_eq!(update.installed_floor(), 9);
    }
    #[test]
    fn update_rejects_rollback_and_wrong_order() {
        let mut update = UpdateAuthorization::new(7);
        assert_eq!(
            update.authorize(6, sha256(b"v6")),
            Err(SecurityError::ВідкатВерсії)
        );
        assert_eq!(update.activate(), Err(SecurityError::НеправильнийСтан));
    }
    #[test]
    fn staged_update_can_be_rolled_back() {
        let digest = sha256(b"v8");
        let mut update = UpdateAuthorization::new(7);
        update.authorize(8, digest).unwrap();
        update.stage(&digest).unwrap();
        update.rollback().unwrap();
        assert_eq!(update.state(), UpdateState::Відкочено);
    }
    #[test]
    fn constant_time_comparison_checks_content_and_length() {
        assert!(constant_time_eq(b"nova", b"nova"));
        assert!(!constant_time_eq(b"nova", b"Nova"));
        assert!(!constant_time_eq(b"a", b"aa"));
    }
}
