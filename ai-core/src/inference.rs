//! Bounded decoder-only transformer inference.
//!
//! Model storage is borrowed and all session state lives in fixed-size arrays.
//! This makes inference usable before Nova has a general-purpose allocator.

use crate::quant::{matvec_q8, Q8Block, QK8};
use crate::sampling::greedy;
use crate::{Error, Result};

pub const MAX_DIM: usize = 64;
pub const MAX_HIDDEN: usize = 128;
pub const MAX_LAYERS: usize = 4;
pub const MAX_CONTEXT: usize = 32;
pub const MAX_VOCAB: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelConfig {
    pub dimension: usize,
    pub hidden_dimension: usize,
    pub layers: usize,
    pub heads: usize,
    pub context: usize,
    pub vocabulary: usize,
}

impl ModelConfig {
    pub fn validate(self) -> Result<()> {
        if self.dimension == 0
            || self.dimension > MAX_DIM
            || !self.dimension.is_multiple_of(QK8)
            || self.hidden_dimension == 0
            || self.hidden_dimension > MAX_HIDDEN
            || !self.hidden_dimension.is_multiple_of(QK8)
            || self.layers == 0
            || self.layers > MAX_LAYERS
            || self.heads == 0
            || !self.dimension.is_multiple_of(self.heads)
            || self.context == 0
            || self.context > MAX_CONTEXT
            || self.vocabulary == 0
            || self.vocabulary > MAX_VOCAB
        {
            return Err(Error::InvalidConfiguration);
        }
        let head_dim = self.dimension / self.heads;
        if !head_dim.is_multiple_of(2) {
            return Err(Error::InvalidConfiguration);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Q8Matrix<'a> {
    pub blocks: &'a [Q8Block],
    pub rows: usize,
    pub columns: usize,
}

impl Q8Matrix<'_> {
    pub fn validate(self, rows: usize, columns: usize) -> Result<()> {
        if self.rows != rows || self.columns != columns || !columns.is_multiple_of(QK8) {
            return Err(Error::ShapeMismatch);
        }
        let needed = rows
            .checked_mul(columns / QK8)
            .ok_or(Error::IntegerOverflow)?;
        if self.blocks.len() != needed {
            return Err(Error::ShapeMismatch);
        }
        Ok(())
    }

    fn project(self, input: &[f32], output: &mut [f32], budget: &mut WorkBudget) -> Result<()> {
        budget.consume(self.rows.saturating_mul(self.columns / QK8))?;
        matvec_q8(self.blocks, self.rows, self.columns, input, output)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LayerWeights<'a> {
    pub attention_norm: &'a [f32],
    pub query: Q8Matrix<'a>,
    pub key: Q8Matrix<'a>,
    pub value: Q8Matrix<'a>,
    pub attention_output: Q8Matrix<'a>,
    pub ffn_norm: &'a [f32],
    pub ffn_gate: Q8Matrix<'a>,
    pub ffn_up: Q8Matrix<'a>,
    pub ffn_down: Q8Matrix<'a>,
}

#[derive(Clone, Copy, Debug)]
pub struct Transformer<'a> {
    pub config: ModelConfig,
    /// Row-major F32 token embedding table `[vocabulary, dimension]`.
    pub embeddings: &'a [f32],
    pub layers: &'a [LayerWeights<'a>],
    pub output_norm: &'a [f32],
    pub output: Q8Matrix<'a>,
    pub rms_epsilon: f32,
    pub rope_base: f32,
}

impl Transformer<'_> {
    pub fn validate(&self) -> Result<()> {
        let c = self.config;
        c.validate()?;
        if self.embeddings.len()
            != c.vocabulary
                .checked_mul(c.dimension)
                .ok_or(Error::IntegerOverflow)?
            || self.layers.len() != c.layers
            || self.output_norm.len() != c.dimension
            || self.rms_epsilon <= 0.0
            || self.rope_base <= 1.0
        {
            return Err(Error::ShapeMismatch);
        }
        self.output.validate(c.vocabulary, c.dimension)?;
        for layer in self.layers {
            if layer.attention_norm.len() != c.dimension || layer.ffn_norm.len() != c.dimension {
                return Err(Error::ShapeMismatch);
            }
            layer.query.validate(c.dimension, c.dimension)?;
            layer.key.validate(c.dimension, c.dimension)?;
            layer.value.validate(c.dimension, c.dimension)?;
            layer.attention_output.validate(c.dimension, c.dimension)?;
            layer.ffn_gate.validate(c.hidden_dimension, c.dimension)?;
            layer.ffn_up.validate(c.hidden_dimension, c.dimension)?;
            layer.ffn_down.validate(c.dimension, c.hidden_dimension)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkBudget {
    remaining: usize,
    cancelled: bool,
}

impl WorkBudget {
    pub const fn new(units: usize) -> Self {
        Self {
            remaining: units,
            cancelled: false,
        }
    }
    pub const fn remaining(&self) -> usize {
        self.remaining
    }
    pub fn cancel(&mut self) {
        self.cancelled = true;
    }
    pub fn consume(&mut self, units: usize) -> Result<()> {
        if self.cancelled {
            return Err(Error::Cancelled);
        }
        self.remaining = self
            .remaining
            .checked_sub(units)
            .ok_or(Error::WorkBudgetExceeded)?;
        Ok(())
    }
}

pub struct DecoderSession {
    position: usize,
    keys: [f32; MAX_LAYERS * MAX_CONTEXT * MAX_DIM],
    values: [f32; MAX_LAYERS * MAX_CONTEXT * MAX_DIM],
    x: [f32; MAX_DIM],
    norm: [f32; MAX_DIM],
    q: [f32; MAX_DIM],
    k: [f32; MAX_DIM],
    v: [f32; MAX_DIM],
    attention: [f32; MAX_DIM],
    projected: [f32; MAX_DIM],
    gate: [f32; MAX_HIDDEN],
    up: [f32; MAX_HIDDEN],
    hidden: [f32; MAX_HIDDEN],
    scores: [f32; MAX_CONTEXT],
    logits: [f32; MAX_VOCAB],
}

impl Default for DecoderSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DecoderSession {
    pub const fn new() -> Self {
        Self {
            position: 0,
            keys: [0.0; MAX_LAYERS * MAX_CONTEXT * MAX_DIM],
            values: [0.0; MAX_LAYERS * MAX_CONTEXT * MAX_DIM],
            x: [0.0; MAX_DIM],
            norm: [0.0; MAX_DIM],
            q: [0.0; MAX_DIM],
            k: [0.0; MAX_DIM],
            v: [0.0; MAX_DIM],
            attention: [0.0; MAX_DIM],
            projected: [0.0; MAX_DIM],
            gate: [0.0; MAX_HIDDEN],
            up: [0.0; MAX_HIDDEN],
            hidden: [0.0; MAX_HIDDEN],
            scores: [0.0; MAX_CONTEXT],
            logits: [0.0; MAX_VOCAB],
        }
    }

    pub const fn position(&self) -> usize {
        self.position
    }
    pub fn reset(&mut self) {
        self.position = 0;
        self.keys.fill(0.0);
        self.values.fill(0.0);
    }

    /// Runs a complete transformer forward pass and returns model logits.
    pub fn forward<'s>(
        &'s mut self,
        model: &Transformer<'_>,
        token: u32,
        budget: &mut WorkBudget,
    ) -> Result<&'s [f32]> {
        model.validate()?;
        budget.consume(1)?;
        let c = model.config;
        let token = usize::try_from(token).map_err(|_| Error::UnknownToken)?;
        if token >= c.vocabulary {
            return Err(Error::UnknownToken);
        }
        if self.position >= c.context {
            return Err(Error::ContextFull);
        }
        let d = c.dimension;
        self.x[..d].copy_from_slice(&model.embeddings[token * d..(token + 1) * d]);

        for (layer_index, layer) in model.layers.iter().enumerate() {
            budget.consume(d)?;
            rms_norm(
                &self.x[..d],
                layer.attention_norm,
                model.rms_epsilon,
                &mut self.norm[..d],
            )?;
            layer
                .query
                .project(&self.norm[..d], &mut self.q[..d], budget)?;
            layer
                .key
                .project(&self.norm[..d], &mut self.k[..d], budget)?;
            layer
                .value
                .project(&self.norm[..d], &mut self.v[..d], budget)?;
            apply_rope(&mut self.q[..d], c.heads, self.position, model.rope_base)?;
            apply_rope(&mut self.k[..d], c.heads, self.position, model.rope_base)?;
            let cache = (layer_index * MAX_CONTEXT + self.position) * MAX_DIM;
            self.keys[cache..cache + d].copy_from_slice(&self.k[..d]);
            self.values[cache..cache + d].copy_from_slice(&self.v[..d]);
            self.attention[..d].fill(0.0);
            let head_dim = d / c.heads;
            let scale = inverse_sqrt(head_dim as f32);
            for head in 0..c.heads {
                let hs = head * head_dim;
                for past in 0..=self.position {
                    budget.consume(head_dim)?;
                    let past_cache = (layer_index * MAX_CONTEXT + past) * MAX_DIM + hs;
                    self.scores[past] = dot(
                        &self.q[hs..hs + head_dim],
                        &self.keys[past_cache..past_cache + head_dim],
                    ) * scale;
                }
                softmax(&mut self.scores[..=self.position]);
                for past in 0..=self.position {
                    let past_cache = (layer_index * MAX_CONTEXT + past) * MAX_DIM + hs;
                    let weight = self.scores[past];
                    for i in 0..head_dim {
                        self.attention[hs + i] += weight * self.values[past_cache + i];
                    }
                }
            }
            layer.attention_output.project(
                &self.attention[..d],
                &mut self.projected[..d],
                budget,
            )?;
            for i in 0..d {
                self.x[i] += self.projected[i];
            }
            rms_norm(
                &self.x[..d],
                layer.ffn_norm,
                model.rms_epsilon,
                &mut self.norm[..d],
            )?;
            let h = c.hidden_dimension;
            layer
                .ffn_gate
                .project(&self.norm[..d], &mut self.gate[..h], budget)?;
            layer
                .ffn_up
                .project(&self.norm[..d], &mut self.up[..h], budget)?;
            for i in 0..h {
                self.hidden[i] = silu(self.gate[i]) * self.up[i];
            }
            layer
                .ffn_down
                .project(&self.hidden[..h], &mut self.projected[..d], budget)?;
            for i in 0..d {
                self.x[i] += self.projected[i];
            }
        }
        rms_norm(
            &self.x[..d],
            model.output_norm,
            model.rms_epsilon,
            &mut self.norm[..d],
        )?;
        model
            .output
            .project(&self.norm[..d], &mut self.logits[..c.vocabulary], budget)?;
        self.position += 1;
        Ok(&self.logits[..c.vocabulary])
    }

    /// One deterministic autoregressive step (forward + greedy token choice).
    pub fn generate(
        &mut self,
        model: &Transformer<'_>,
        token: u32,
        budget: &mut WorkBudget,
    ) -> Result<u32> {
        greedy(self.forward(model, token, budget)?)
    }
}

pub fn rms_norm(input: &[f32], weights: &[f32], epsilon: f32, output: &mut [f32]) -> Result<()> {
    if input.is_empty()
        || input.len() != weights.len()
        || output.len() < input.len()
        || epsilon <= 0.0
    {
        return Err(Error::InvalidConfiguration);
    }
    let mean = input.iter().map(|x| x * x).sum::<f32>() / input.len() as f32;
    let scale = inverse_sqrt(mean + epsilon);
    for i in 0..input.len() {
        output[i] = input[i] * scale * weights[i];
    }
    Ok(())
}

pub fn apply_rope(vector: &mut [f32], heads: usize, position: usize, base: f32) -> Result<()> {
    if heads == 0 || !vector.len().is_multiple_of(heads) || base <= 1.0 {
        return Err(Error::InvalidConfiguration);
    }
    let head_dim = vector.len() / heads;
    if !head_dim.is_multiple_of(2) {
        return Err(Error::InvalidConfiguration);
    }
    for head in 0..heads {
        for pair in (0..head_dim).step_by(2) {
            // Repeated square roots approximate base^(pair/head_dim) without libm.
            let exponent = pair as f32 / head_dim as f32;
            let frequency = pow_approx(base, -exponent);
            let angle = position as f32 * frequency;
            let (sin, cos) = sin_cos(angle);
            let i = head * head_dim + pair;
            let a = vector[i];
            let b = vector[i + 1];
            vector[i] = a * cos - b * sin;
            vector[i + 1] = a * sin + b * cos;
        }
    }
    Ok(())
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn softmax(values: &mut [f32]) {
    let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0;
    for value in values.iter_mut() {
        *value = fast_exp(*value - max);
        sum += *value;
    }
    if sum > 0.0 {
        for value in values {
            *value /= sum;
        }
    }
}

fn silu(x: f32) -> f32 {
    x / (1.0 + fast_exp(-x))
}

fn inverse_sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut y = f32::from_bits(0x5f37_5a86 - (x.to_bits() >> 1));
    y *= 1.5 - 0.5 * x * y * y;
    y *= 1.5 - 0.5 * x * y * y;
    y
}

fn sqrt(x: f32) -> f32 {
    if x <= 0.0 {
        0.0
    } else {
        x * inverse_sqrt(x)
    }
}

fn pow_approx(mut base: f32, exponent: f32) -> f32 {
    // RoPE only requests exponents in [-1, 0]. Binary fractional powers suffice.
    let negative = exponent < 0.0;
    let mut fraction = if negative { -exponent } else { exponent };
    let mut result = 1.0;
    for _ in 0..16 {
        base = sqrt(base);
        fraction *= 2.0;
        if fraction >= 1.0 {
            result *= base;
            fraction -= 1.0;
        }
    }
    if negative {
        1.0 / result
    } else {
        result
    }
}

fn sin_cos(mut x: f32) -> (f32, f32) {
    const TAU: f32 = core::f32::consts::TAU;
    while x > core::f32::consts::PI {
        x -= TAU;
    }
    while x < -core::f32::consts::PI {
        x += TAU;
    }
    let x2 = x * x;
    let sin = x * (1.0 - x2 / 6.0 + x2 * x2 / 120.0 - x2 * x2 * x2 / 5040.0);
    let cos = 1.0 - x2 / 2.0 + x2 * x2 / 24.0 - x2 * x2 * x2 / 720.0;
    (sin, cos)
}

fn fast_exp(x: f32) -> f32 {
    if x < -16.0 {
        return 0.0;
    }
    if x > 16.0 {
        return 8_886_110.0;
    }
    let mut r = x;
    let mut halves = 0;
    while r.abs() > 0.5 {
        r *= 0.5;
        halves += 1;
    }
    let r2 = r * r;
    let mut out = 1.0 + r + r2 * 0.5 + r2 * r / 6.0 + r2 * r2 / 24.0 + r2 * r2 * r / 120.0;
    for _ in 0..halves {
        out *= out;
    }
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;

    fn matrix(rows: usize, columns: usize, diagonal: bool) -> Vec<Q8Block> {
        let mut result = Vec::new();
        for row in 0..rows {
            for block in 0..columns / QK8 {
                let mut q = Q8Block::ZERO;
                q.scale = 1.0;
                let column = row;
                if diagonal && column / QK8 == block && column < columns {
                    q.values[column % QK8] = 1;
                }
                result.push(q);
            }
        }
        result
    }

    #[test]
    fn rmsnorm_and_rope_have_expected_invariants() {
        let mut out = [0.0; 4];
        rms_norm(&[2.0; 4], &[1.0; 4], 1e-5, &mut out).unwrap();
        for x in out {
            assert!((x - 1.0).abs() < 0.01);
        }
        let mut v = [1.0, 0.0, 0.0, 1.0];
        apply_rope(&mut v, 1, 0, 10_000.0).unwrap();
        assert_eq!(v, [1.0, 0.0, 0.0, 1.0]);
        apply_rope(&mut v, 1, 1, 10_000.0).unwrap();
        assert!((v[0] * v[0] + v[1] * v[1] - 1.0).abs() < 0.01);
    }

    #[test]
    fn deterministic_tiny_model_runs_end_to_end_with_kv_cache() {
        let identity = matrix(32, 32, true);
        let zero = matrix(32, 32, false);
        let output = matrix(4, 32, true);
        let norm = [1.0; 32];
        let mut embeddings = [0.0; 4 * 32];
        embeddings[2 * 32 + 2] = 3.0;
        let layer = LayerWeights {
            attention_norm: &norm,
            query: Q8Matrix {
                blocks: &identity,
                rows: 32,
                columns: 32,
            },
            key: Q8Matrix {
                blocks: &identity,
                rows: 32,
                columns: 32,
            },
            value: Q8Matrix {
                blocks: &identity,
                rows: 32,
                columns: 32,
            },
            attention_output: Q8Matrix {
                blocks: &zero,
                rows: 32,
                columns: 32,
            },
            ffn_norm: &norm,
            ffn_gate: Q8Matrix {
                blocks: &zero,
                rows: 32,
                columns: 32,
            },
            ffn_up: Q8Matrix {
                blocks: &zero,
                rows: 32,
                columns: 32,
            },
            ffn_down: Q8Matrix {
                blocks: &zero,
                rows: 32,
                columns: 32,
            },
        };
        let model = Transformer {
            config: ModelConfig {
                dimension: 32,
                hidden_dimension: 32,
                layers: 1,
                heads: 4,
                context: 4,
                vocabulary: 4,
            },
            embeddings: &embeddings,
            layers: &[layer],
            output_norm: &norm,
            output: Q8Matrix {
                blocks: &output,
                rows: 4,
                columns: 32,
            },
            rms_epsilon: 1e-5,
            rope_base: 10_000.0,
        };
        let mut session = DecoderSession::new();
        let mut budget = WorkBudget::new(10_000);
        assert_eq!(session.generate(&model, 2, &mut budget).unwrap(), 2);
        assert_eq!(session.generate(&model, 2, &mut budget).unwrap(), 2);
        assert_eq!(session.position(), 2);
        assert!(budget.remaining() < 10_000);
    }

    #[test]
    fn cancellation_budget_and_shapes_are_enforced() {
        let mut b = WorkBudget::new(1);
        assert_eq!(b.consume(2), Err(Error::WorkBudgetExceeded));
        let mut cancelled = WorkBudget::new(100);
        cancelled.cancel();
        assert_eq!(cancelled.consume(0), Err(Error::Cancelled));
        let bad = Q8Matrix {
            blocks: &[],
            rows: 1,
            columns: 32,
        };
        assert_eq!(bad.validate(1, 32), Err(Error::ShapeMismatch));
    }
}
