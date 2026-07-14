//! Reproducible sampling using caller-owned candidate scratch space.

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub token: u32,
    pub probability: f32,
}
impl Candidate {
    pub const ZERO: Self = Self {
        token: 0,
        probability: 0.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplingConfig {
    pub temperature: f32,
    pub top_k: usize,
    pub top_p: f32,
}
impl Default for SamplingConfig {
    fn default() -> Self {
        Self {
            temperature: 0.8,
            top_k: 40,
            top_p: 0.95,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeterministicRng {
    state: u64,
}
impl DeterministicRng {
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    pub fn next_unit_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as u32) as f32 / 16_777_216.0
    }
}

pub fn greedy(logits: &[f32]) -> Result<u32> {
    let mut best: Option<(usize, f32)> = None;
    for (index, value) in logits.iter().copied().enumerate() {
        if value.is_nan() {
            continue;
        }
        if best.is_none_or(|(best_index, current)| {
            value > current || (value == current && index < best_index)
        }) {
            best = Some((index, value));
        }
    }
    best.map(|(index, _)| index as u32)
        .ok_or(Error::InvalidConfiguration)
}

pub fn sample(
    logits: &[f32],
    config: SamplingConfig,
    rng: &mut DeterministicRng,
    scratch: &mut [Candidate],
) -> Result<u32> {
    if logits.is_empty() || config.temperature < 0.0 || config.top_p <= 0.0 || config.top_p > 1.0 {
        return Err(Error::InvalidConfiguration);
    }
    if config.temperature == 0.0 {
        return greedy(logits);
    }
    let keep = if config.top_k == 0 {
        logits.len()
    } else {
        config.top_k.min(logits.len())
    };
    if scratch.len() < keep {
        return Err(Error::OutputTooSmall);
    }
    // Keep the strongest K candidates in descending order with deterministic tie breaks.
    let mut used = 0usize;
    for (token, logit) in logits.iter().copied().enumerate() {
        if logit.is_nan() {
            continue;
        }
        let mut position = 0usize;
        while position < used
            && (scratch[position].probability > logit
                || (scratch[position].probability == logit
                    && scratch[position].token < token as u32))
        {
            position += 1;
        }
        if position < keep {
            let new_used = (used + 1).min(keep);
            if position < new_used - 1 {
                scratch.copy_within(position..new_used - 1, position + 1);
            }
            scratch[position] = Candidate {
                token: token as u32,
                probability: logit,
            };
            used = new_used;
        }
    }
    if used == 0 {
        return Err(Error::InvalidConfiguration);
    }
    let max = scratch[0].probability;
    let mut total = 0.0f32;
    for candidate in &mut scratch[..used] {
        candidate.probability = fast_exp((candidate.probability - max) / config.temperature);
        total += candidate.probability;
    }
    if total <= 0.0 {
        return Ok(scratch[0].token);
    }
    for candidate in &mut scratch[..used] {
        candidate.probability /= total;
    }
    let mut cumulative = 0.0f32;
    let mut nucleus = used;
    for (index, candidate) in scratch[..used].iter().enumerate() {
        cumulative += candidate.probability;
        if cumulative >= config.top_p {
            nucleus = index + 1;
            break;
        }
    }
    let nucleus_total: f32 = scratch[..nucleus].iter().map(|c| c.probability).sum();
    let needle = rng.next_unit_f32() * nucleus_total;
    let mut seen = 0.0;
    for candidate in &scratch[..nucleus] {
        seen += candidate.probability;
        if needle < seen {
            return Ok(candidate.token);
        }
    }
    Ok(scratch[nucleus - 1].token)
}

// Range reduction followed by a fifth-order Taylor series. Sampling only uses
// non-positive inputs, and values below -16 have negligible probability.
fn fast_exp(mut x: f32) -> f32 {
    if x <= -16.0 {
        return 0.0;
    }
    if x >= 0.0 {
        return 1.0;
    }
    const LN2: f32 = core::f32::consts::LN_2;
    let mut halves = 0u32;
    while x < -LN2 {
        x += LN2;
        halves += 1;
    }
    let x2 = x * x;
    let x3 = x2 * x;
    let x4 = x3 * x;
    let x5 = x4 * x;
    let mut value = 1.0 + x + x2 * 0.5 + x3 * (1.0 / 6.0) + x4 * (1.0 / 24.0) + x5 * (1.0 / 120.0);
    for _ in 0..halves {
        value *= 0.5;
    }
    value.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn greedy_ties_choose_lowest_token() {
        assert_eq!(greedy(&[1.0, 5.0, 5.0]).unwrap(), 1);
    }
    #[test]
    fn seeded_sampling_is_reproducible() {
        let logits = [1.0, 2.0, 3.0, 4.0];
        let cfg = SamplingConfig {
            temperature: 0.9,
            top_k: 3,
            top_p: 0.9,
        };
        let mut a = DeterministicRng::new(42);
        let mut b = DeterministicRng::new(42);
        let mut sa = [Candidate::ZERO; 4];
        let mut sb = [Candidate::ZERO; 4];
        for _ in 0..20 {
            assert_eq!(
                sample(&logits, cfg, &mut a, &mut sa).unwrap(),
                sample(&logits, cfg, &mut b, &mut sb).unwrap()
            );
        }
    }
    #[test]
    fn top_one_and_zero_temperature_are_greedy() {
        let mut rng = DeterministicRng::new(1);
        let mut scratch = [Candidate::ZERO; 1];
        assert_eq!(
            sample(
                &[2.0, 7.0, 1.0],
                SamplingConfig {
                    temperature: 1.0,
                    top_k: 1,
                    top_p: 1.0
                },
                &mut rng,
                &mut scratch
            )
            .unwrap(),
            1
        );
        assert_eq!(
            sample(
                &[2.0, 7.0, 1.0],
                SamplingConfig {
                    temperature: 0.0,
                    top_k: 0,
                    top_p: 1.0
                },
                &mut rng,
                &mut []
            )
            .unwrap(),
            1
        );
    }
    #[test]
    fn validates_scratch_and_config() {
        let mut rng = DeterministicRng::new(1);
        assert_eq!(
            sample(
                &[1.0, 2.0],
                SamplingConfig::default(),
                &mut rng,
                &mut [Candidate::ZERO; 1]
            ),
            Err(Error::OutputTooSmall)
        );
    }
}
