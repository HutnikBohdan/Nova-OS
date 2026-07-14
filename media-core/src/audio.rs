use crate::MediaError;

pub const UNITY_GAIN: i16 = i16::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StereoFrame {
    pub left: i16,
    pub right: i16,
}

impl StereoFrame {
    pub const SILENCE: Self = Self { left: 0, right: 0 };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Voice<'a> {
    samples: &'a [StereoFrame],
    phase_q16: u64,
    step_q16: u64,
    volume: i16,
    pan: i16,
    looping: bool,
}

impl<'a> Voice<'a> {
    pub fn new(
        samples: &'a [StereoFrame],
        source_rate: u32,
        output_rate: u32,
        volume: i16,
        pan: i16,
        looping: bool,
    ) -> Result<Self, MediaError> {
        if samples.is_empty() {
            return Err(MediaError::Empty);
        }
        if source_rate == 0 || output_rate == 0 || volume < 0 {
            return Err(MediaError::InvalidValue);
        }
        let step_q16 = ((source_rate as u64) << 16) / output_rate as u64;
        if step_q16 == 0 {
            return Err(MediaError::Unsupported);
        }
        Ok(Self {
            samples,
            phase_q16: 0,
            step_q16,
            volume,
            pan: pan.max(-i16::MAX),
            looping,
        })
    }

    pub const fn volume(&self) -> i16 {
        self.volume
    }
    pub fn set_volume(&mut self, volume: i16) -> Result<(), MediaError> {
        if volume < 0 {
            return Err(MediaError::InvalidValue);
        }
        self.volume = volume;
        Ok(())
    }
    pub const fn set_pan(&mut self, pan: i16) {
        self.pan = if pan == i16::MIN { -i16::MAX } else { pan };
    }

    fn next(&mut self) -> Option<StereoFrame> {
        let index = (self.phase_q16 >> 16) as usize;
        if index >= self.samples.len() {
            if !self.looping {
                return None;
            }
            self.phase_q16 %= (self.samples.len() as u64) << 16;
        }
        let index = (self.phase_q16 >> 16) as usize;
        let fraction = (self.phase_q16 & 0xffff) as i64;
        let next_index = if index + 1 < self.samples.len() {
            index + 1
        } else if self.looping {
            0
        } else {
            index
        };
        let a = self.samples[index];
        let b = self.samples[next_index];
        self.phase_q16 = self.phase_q16.saturating_add(self.step_q16);
        let interpolated = StereoFrame {
            left: lerp(a.left, b.left, fraction),
            right: lerp(a.right, b.right, fraction),
        };
        let pan = self.pan as i32;
        let left_gain = (self.volume as i32 * (i16::MAX as i32 - pan.max(0))) / i16::MAX as i32;
        let right_gain = (self.volume as i32 * (i16::MAX as i32 + pan.min(0))) / i16::MAX as i32;
        Some(StereoFrame {
            left: gain(interpolated.left, left_gain),
            right: gain(interpolated.right, right_gain),
        })
    }
}

fn lerp(a: i16, b: i16, fraction: i64) -> i16 {
    let value = a as i64 * (65_536 - fraction) + b as i64 * fraction;
    (value / 65_536) as i16
}

fn gain(sample: i16, gain_q15: i32) -> i16 {
    ((sample as i32 * gain_q15) / i16::MAX as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoiceId(pub usize);

pub struct Mixer<'a, const VOICES: usize> {
    voices: [Option<Voice<'a>>; VOICES],
}

impl<'a, const VOICES: usize> Mixer<'a, VOICES> {
    pub const fn new() -> Self {
        Self {
            voices: [None; VOICES],
        }
    }

    pub fn play(&mut self, voice: Voice<'a>) -> Result<VoiceId, MediaError> {
        let mut index = 0;
        while index < VOICES {
            if self.voices[index].is_none() {
                self.voices[index] = Some(voice);
                return Ok(VoiceId(index));
            }
            index += 1;
        }
        Err(MediaError::Capacity)
    }

    pub fn stop(&mut self, id: VoiceId) -> bool {
        if id.0 >= VOICES {
            return false;
        }
        self.voices[id.0].take().is_some()
    }

    pub fn voice_mut(&mut self, id: VoiceId) -> Option<&mut Voice<'a>> {
        self.voices.get_mut(id.0)?.as_mut()
    }

    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|voice| voice.is_some()).count()
    }

    pub fn mix(&mut self, output: &mut [StereoFrame]) {
        for frame in output {
            let mut left = 0i64;
            let mut right = 0i64;
            let mut index = 0;
            while index < VOICES {
                if let Some(mut voice) = self.voices[index] {
                    if let Some(sample) = voice.next() {
                        left += sample.left as i64;
                        right += sample.right as i64;
                        self.voices[index] = Some(voice);
                    } else {
                        self.voices[index] = None;
                    }
                }
                index += 1;
            }
            *frame = StereoFrame {
                left: left.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
                right: right.clamp(i16::MIN as i64, i16::MAX as i64) as i16,
            };
        }
    }
}

impl<const VOICES: usize> Default for Mixer<'_, VOICES> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RingBuffer<const N: usize> {
    frames: [StereoFrame; N],
    read: usize,
    write: usize,
    len: usize,
}

impl<const N: usize> RingBuffer<N> {
    pub const fn new() -> Self {
        Self {
            frames: [StereoFrame::SILENCE; N],
            read: 0,
            write: 0,
            len: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn capacity(&self) -> usize {
        N
    }
    pub const fn available_write(&self) -> usize {
        N - self.len
    }

    pub fn push(&mut self, frame: StereoFrame) -> Result<(), MediaError> {
        if self.len == N {
            return Err(MediaError::Capacity);
        }
        if N == 0 {
            return Err(MediaError::Capacity);
        }
        self.frames[self.write] = frame;
        self.write = (self.write + 1) % N;
        self.len += 1;
        Ok(())
    }

    pub fn push_slice(&mut self, frames: &[StereoFrame]) -> usize {
        let mut written = 0;
        while written < frames.len() && self.push(frames[written]).is_ok() {
            written += 1;
        }
        written
    }

    pub fn pop(&mut self) -> Option<StereoFrame> {
        if self.len == 0 || N == 0 {
            return None;
        }
        let frame = self.frames[self.read];
        self.read = (self.read + 1) % N;
        self.len -= 1;
        Some(frame)
    }

    pub fn drain_or_silence(&mut self, output: &mut [StereoFrame]) -> usize {
        let mut missing = 0;
        for frame in output {
            if let Some(value) = self.pop() {
                *frame = value;
            } else {
                *frame = StereoFrame::SILENCE;
                missing += 1;
            }
        }
        missing
    }
}

impl<const N: usize> Default for RingBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    Signed16,
    Signed24In32,
    Signed32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamContract {
    pub sample_rate: u32,
    pub channels: u8,
    pub format: SampleFormat,
    pub period_frames: u16,
    pub period_count: u8,
}

impl StreamContract {
    pub const fn validate(self) -> Result<Self, MediaError> {
        if self.sample_rate < 8_000
            || self.sample_rate > 192_000
            || self.channels == 0
            || self.channels > 8
            || self.period_frames == 0
            || self.period_count < 2
        {
            Err(MediaError::InvalidValue)
        } else {
            Ok(self)
        }
    }

    pub const fn bytes_per_sample(self) -> usize {
        match self.format {
            SampleFormat::Signed16 => 2,
            _ => 4,
        }
    }

    pub const fn period_bytes(self) -> usize {
        self.period_frames as usize * self.channels as usize * self.bytes_per_sample()
    }
}

/// Hardware-neutral period accounting used by HDA and future audio drivers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamScheduler {
    contract: StreamContract,
    queued_periods: u8,
    completed_periods: u64,
    underrun_frames: u64,
}

impl StreamScheduler {
    pub fn new(contract: StreamContract) -> Result<Self, MediaError> {
        contract.validate()?;
        Ok(Self {
            contract,
            queued_periods: 0,
            completed_periods: 0,
            underrun_frames: 0,
        })
    }

    pub fn submit_period(&mut self) -> Result<u8, MediaError> {
        if self.queued_periods >= self.contract.period_count {
            return Err(MediaError::Capacity);
        }
        let index =
            (self.completed_periods as u8 + self.queued_periods) % self.contract.period_count;
        self.queued_periods += 1;
        Ok(index)
    }

    pub fn hardware_period_complete(&mut self) {
        if self.queued_periods == 0 {
            self.underrun_frames = self
                .underrun_frames
                .saturating_add(self.contract.period_frames as u64);
        } else {
            self.queued_periods -= 1;
        }
        self.completed_periods = self.completed_periods.saturating_add(1);
    }

    pub const fn queued_periods(&self) -> u8 {
        self.queued_periods
    }
    pub const fn completed_periods(&self) -> u64 {
        self.completed_periods
    }
    pub const fn underrun_frames(&self) -> u64 {
        self.underrun_frames
    }
    pub const fn contract(&self) -> StreamContract {
        self.contract
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixer_saturates_multiple_voices() {
        let samples = [StereoFrame {
            left: 30_000,
            right: 30_000,
        }; 2];
        let voice = Voice::new(&samples, 48_000, 48_000, UNITY_GAIN, 0, false).unwrap();
        let mut mixer = Mixer::<2>::new();
        mixer.play(voice).unwrap();
        mixer.play(voice).unwrap();
        let mut output = [StereoFrame::SILENCE; 1];
        mixer.mix(&mut output);
        assert_eq!(
            output[0],
            StereoFrame {
                left: i16::MAX,
                right: i16::MAX
            }
        );
    }

    #[test]
    fn pan_and_volume_are_applied() {
        let samples = [StereoFrame {
            left: 10_000,
            right: 10_000,
        }];
        let mut mixer = Mixer::<1>::new();
        mixer
            .play(Voice::new(&samples, 48_000, 48_000, UNITY_GAIN, i16::MAX, false).unwrap())
            .unwrap();
        let mut out = [StereoFrame::SILENCE];
        mixer.mix(&mut out);
        assert_eq!(
            out[0],
            StereoFrame {
                left: 0,
                right: 10_000
            }
        );
    }

    #[test]
    fn linear_resampler_generates_midpoint() {
        let samples = [
            StereoFrame { left: 0, right: 0 },
            StereoFrame {
                left: 10_000,
                right: -10_000,
            },
        ];
        let mut mixer = Mixer::<1>::new();
        mixer
            .play(Voice::new(&samples, 24_000, 48_000, UNITY_GAIN, 0, false).unwrap())
            .unwrap();
        let mut out = [StereoFrame::SILENCE; 3];
        mixer.mix(&mut out);
        assert_eq!(
            out[1],
            StereoFrame {
                left: 5_000,
                right: -5_000
            }
        );
    }

    #[test]
    fn ended_voice_is_reclaimed() {
        let samples = [StereoFrame { left: 1, right: 1 }];
        let mut mixer = Mixer::<1>::new();
        mixer
            .play(Voice::new(&samples, 48_000, 48_000, UNITY_GAIN, 0, false).unwrap())
            .unwrap();
        let mut out = [StereoFrame::SILENCE; 2];
        mixer.mix(&mut out);
        assert_eq!(mixer.active_voices(), 0);
    }

    #[test]
    fn ring_buffer_wraps_without_reordering() {
        let mut ring = RingBuffer::<2>::new();
        ring.push(StereoFrame { left: 1, right: 1 }).unwrap();
        ring.push(StereoFrame { left: 2, right: 2 }).unwrap();
        assert_eq!(ring.pop().unwrap().left, 1);
        ring.push(StereoFrame { left: 3, right: 3 }).unwrap();
        assert_eq!(ring.pop().unwrap().left, 2);
        assert_eq!(ring.pop().unwrap().left, 3);
    }

    #[test]
    fn ring_buffer_reports_missing_frames() {
        let mut ring = RingBuffer::<2>::new();
        ring.push(StereoFrame { left: 7, right: 8 }).unwrap();
        let mut out = [StereoFrame { left: 1, right: 1 }; 3];
        assert_eq!(ring.drain_or_silence(&mut out), 2);
        assert_eq!(out[2], StereoFrame::SILENCE);
    }

    fn contract() -> StreamContract {
        StreamContract {
            sample_rate: 48_000,
            channels: 2,
            format: SampleFormat::Signed16,
            period_frames: 256,
            period_count: 3,
        }
    }

    #[test]
    fn hda_neutral_contract_calculates_period_bytes() {
        assert_eq!(contract().period_bytes(), 1024);
        assert!(contract().validate().is_ok());
    }

    #[test]
    fn scheduler_tracks_periods_and_underruns() {
        let mut scheduler = StreamScheduler::new(contract()).unwrap();
        assert_eq!(scheduler.submit_period(), Ok(0));
        assert_eq!(scheduler.submit_period(), Ok(1));
        scheduler.hardware_period_complete();
        scheduler.hardware_period_complete();
        scheduler.hardware_period_complete();
        assert_eq!(scheduler.completed_periods(), 3);
        assert_eq!(scheduler.underrun_frames(), 256);
    }
}
