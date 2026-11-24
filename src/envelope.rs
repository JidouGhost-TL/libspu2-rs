// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! ADSR envelope generator, volume sweep, and per-voice channel state.

use crate::inspect::EnvelopePhase;

pub(crate) const MAX_ADSR_LEVEL: i32 = 0x7FFF;

const LINEAR_INCREASE_SWEEP_DELTAS: [u32; 0x80] = [
    0x3A00_0000,
    0x3000_0000,
    0x2800_0000,
    0x2000_0000,
    0x1D00_0000,
    0x1800_0000,
    0x1400_0000,
    0x1000_0000,
    0x0E80_0000,
    0x0C00_0000,
    0x0A00_0000,
    0x0800_0000,
    0x0740_0000,
    0x0600_0000,
    0x0500_0000,
    0x0400_0000,
    0x03A0_0000,
    0x0300_0000,
    0x0280_0000,
    0x0200_0000,
    0x01D0_0000,
    0x0180_0000,
    0x0140_0000,
    0x0100_0000,
    0x00E8_0000,
    0x00C0_0000,
    0x00A0_0000,
    0x0080_0000,
    0x0074_0000,
    0x0060_0000,
    0x0050_0000,
    0x0040_0000,
    0x003A_0000,
    0x0030_0000,
    0x0028_0000,
    0x0020_0000,
    0x001D_0000,
    0x0018_0000,
    0x0014_0000,
    0x0010_0000,
    0x000E_8000,
    0x000C_0000,
    0x000A_0000,
    0x0008_0000,
    0x0007_4000,
    0x0006_0000,
    0x0005_0000,
    0x0004_0000,
    0x0003_A000,
    0x0003_0000,
    0x0002_8000,
    0x0002_0000,
    0x0001_D000,
    0x0001_8000,
    0x0001_4000,
    0x0001_0000,
    0x0000_E800,
    0x0000_C000,
    0x0000_A000,
    0x0000_8000,
    0x0000_7400,
    0x0000_6000,
    0x0000_5000,
    0x0000_4000,
    0x0000_3A00,
    0x0000_3000,
    0x0000_2800,
    0x0000_2000,
    0x0000_1D00,
    0x0000_1800,
    0x0000_1400,
    0x0000_1000,
    0x0000_0E80,
    0x0000_0C00,
    0x0000_0A00,
    0x0000_0800,
    0x0000_0740,
    0x0000_0600,
    0x0000_0500,
    0x0000_0400,
    0x0000_03A0,
    0x0000_0300,
    0x0000_0280,
    0x0000_0200,
    0x0000_01D0,
    0x0000_0180,
    0x0000_0140,
    0x0000_0100,
    0x0000_00E8,
    0x0000_00C0,
    0x0000_00A0,
    0x0000_0080,
    0x0000_0074,
    0x0000_0060,
    0x0000_0050,
    0x0000_0040,
    0x0000_003A,
    0x0000_0030,
    0x0000_0028,
    0x0000_0020,
    0x0000_001D,
    0x0000_0018,
    0x0000_0014,
    0x0000_0010,
    0x0000_000E,
    0x0000_000C,
    0x0000_000A,
    0x0000_0008,
    0x0000_0007,
    0x0000_0006,
    0x0000_0005,
    0x0000_0004,
    0x0000_0003,
    0x0000_0003,
    0x0000_0002,
    0x0000_0002,
    0x0000_0001,
    0x0000_0001,
    0x0000_0001,
    0x0000_0001,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
];

const LINEAR_DECREASE_SWEEP_DELTAS: [u32; 0x80] = [
    0x4800_0000,
    0x4000_0000,
    0x3800_0000,
    0x3000_0000,
    0x2400_0000,
    0x2000_0000,
    0x1C00_0000,
    0x1800_0000,
    0x1200_0000,
    0x1000_0000,
    0x0E00_0000,
    0x0C00_0000,
    0x0900_0000,
    0x0800_0000,
    0x0700_0000,
    0x0600_0000,
    0x0480_0000,
    0x0400_0000,
    0x0380_0000,
    0x0300_0000,
    0x0240_0000,
    0x0200_0000,
    0x01C0_0000,
    0x0180_0000,
    0x0120_0000,
    0x0100_0000,
    0x00E0_0000,
    0x00C0_0000,
    0x0090_0000,
    0x0080_0000,
    0x0070_0000,
    0x0060_0000,
    0x0048_0000,
    0x0040_0000,
    0x0038_0000,
    0x0030_0000,
    0x0024_0000,
    0x0020_0000,
    0x001C_0000,
    0x0018_0000,
    0x0012_0000,
    0x0010_0000,
    0x000E_0000,
    0x000C_0000,
    0x0009_0000,
    0x0008_0000,
    0x0007_0000,
    0x0006_0000,
    0x0004_8000,
    0x0004_0000,
    0x0003_8000,
    0x0003_0000,
    0x0002_4000,
    0x0002_0000,
    0x0001_C000,
    0x0001_8000,
    0x0001_2000,
    0x0001_0000,
    0x0000_E000,
    0x0000_C000,
    0x0000_9000,
    0x0000_8000,
    0x0000_7000,
    0x0000_6000,
    0x0000_4800,
    0x0000_4000,
    0x0000_3800,
    0x0000_3000,
    0x0000_2400,
    0x0000_2000,
    0x0000_1C00,
    0x0000_1800,
    0x0000_1200,
    0x0000_1000,
    0x0000_0E00,
    0x0000_0C00,
    0x0000_0900,
    0x0000_0800,
    0x0000_0700,
    0x0000_0600,
    0x0000_0480,
    0x0000_0400,
    0x0000_0380,
    0x0000_0300,
    0x0000_0240,
    0x0000_0200,
    0x0000_01C0,
    0x0000_0180,
    0x0000_0120,
    0x0000_0100,
    0x0000_00E0,
    0x0000_00C0,
    0x0000_0090,
    0x0000_0080,
    0x0000_0070,
    0x0000_0060,
    0x0000_0048,
    0x0000_0040,
    0x0000_0038,
    0x0000_0030,
    0x0000_0024,
    0x0000_0020,
    0x0000_001C,
    0x0000_0018,
    0x0000_0012,
    0x0000_0010,
    0x0000_000E,
    0x0000_000C,
    0x0000_0009,
    0x0000_0008,
    0x0000_0007,
    0x0000_0006,
    0x0000_0004,
    0x0000_0004,
    0x0000_0003,
    0x0000_0003,
    0x0000_0002,
    0x0000_0002,
    0x0000_0001,
    0x0000_0001,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
];

/// Packed `VP_ADSR1` register.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdsrLevel(pub(crate) u16);

impl AdsrLevel {
    #[inline]
    pub(crate) const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub(crate) const fn sustain_level(self) -> u32 {
        (self.0 & 0x000F) as u32
    }
    #[inline]
    pub(crate) const fn decay_rate(self) -> u32 {
        ((self.0 >> 4) & 0x000F) as u32
    }
    #[inline]
    pub(crate) const fn attack_rate(self) -> u32 {
        ((self.0 >> 8) & 0x007F) as u32
    }
    #[inline]
    pub(crate) const fn attack_exponential(self) -> bool {
        (self.0 & 0x8000) != 0
    }
}

/// Packed `VP_ADSR2` register.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdsrRate(pub(crate) u16);

impl AdsrRate {
    #[inline]
    pub(crate) const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub(crate) const fn release_rate(self) -> u32 {
        (self.0 & 0x001F) as u32
    }
    #[inline]
    pub(crate) const fn release_exponential(self) -> bool {
        (self.0 & 0x0020) != 0
    }
    #[inline]
    pub(crate) const fn sustain_rate(self) -> u32 {
        ((self.0 >> 6) & 0x007F) as u32
    }
    #[inline]
    pub(crate) const fn sustain_decrease(self) -> bool {
        (self.0 & 0x4000) != 0
    }
    #[inline]
    pub(crate) const fn sustain_exponential(self) -> bool {
        (self.0 & 0x8000) != 0
    }
}

/// Packed `VP_VOLL` / `VP_VOLR` / `P_MVOLL` / `P_MVOLR` register.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChannelVolume(pub(crate) u16);

impl ChannelVolume {
    #[inline]
    pub(crate) const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub(crate) const fn is_sweep(self) -> bool {
        (self.0 & 0x8000) != 0
    }
    #[inline]
    pub(crate) const fn fixed_volume_i32(self) -> i32 {
        (((self.0 << 1) as i16) as i32) << 16
    }
    #[inline]
    pub(crate) const fn sweep_volume(self) -> usize {
        (self.0 & 0x007F) as usize
    }
    #[inline]
    pub(crate) const fn sweep_decrease(self) -> bool {
        (self.0 & 0x2000) != 0
    }
    #[inline]
    pub(crate) const fn sweep_slope(self) -> bool {
        (self.0 & 0x4000) != 0
    }
}

/// Per-voice register and synthesis state.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Channel {
    pub(crate) volume_left: ChannelVolume,
    pub(crate) volume_right: ChannelVolume,
    pub(crate) volume_left_abs: i32,
    pub(crate) volume_right_abs: i32,
    pub(crate) pitch: u16,
    pub(crate) address: u32,
    pub(crate) adsr_level: AdsrLevel,
    pub(crate) adsr_rate: AdsrRate,
    pub(crate) adsr_envx: i32,
    pub(crate) adsr_cycles: i32,
    pub(crate) repeat: u32,
    pub(crate) repeat_set: bool,
    pub(crate) status: EnvelopePhase,
    pub(crate) current: u32,
}

impl Channel {
    #[inline]
    pub(crate) fn set_volume_left(&mut self, value: u16) {
        self.volume_left = ChannelVolume(value);
        if !self.volume_left.is_sweep() {
            self.volume_left_abs = self.volume_left.fixed_volume_i32();
        }
    }

    #[inline]
    pub(crate) fn set_volume_right(&mut self, value: u16) {
        self.volume_right = ChannelVolume(value);
        if !self.volume_right.is_sweep() {
            self.volume_right_abs = self.volume_right.fixed_volume_i32();
        }
    }
}

/// Steps a fixed or sweeping channel/master volume by one sample tick.
#[inline]
pub(crate) fn compute_channel_volume(volume: ChannelVolume, current_volume: i32) -> i32 {
    if !volume.is_sweep() {
        volume.fixed_volume_i32()
    } else {
        let volume_level = if !volume.sweep_slope() {
            if volume.sweep_decrease() {
                let delta = LINEAR_DECREASE_SWEEP_DELTAS[volume.sweep_volume()];
                current_volume.wrapping_sub(delta as i32)
            } else {
                let delta = LINEAR_INCREASE_SWEEP_DELTAS[volume.sweep_volume()];
                current_volume.wrapping_add(delta as i32)
            }
        } else if volume.sweep_decrease() {
            let sweep_delta = (current_volume as i64 * volume.sweep_volume() as i64) / 0x7F;
            let base_volume = current_volume.max(1);
            let sweep_delta_clamped = sweep_delta.clamp(1, base_volume as i64) as i32;
            base_volume.wrapping_sub(sweep_delta_clamped)
        } else {
            // Exponential increase: linear ramp up to 0x6000 (0x6000_0000 in Q31),
            // then 4x slowdown above the hardware knee.
            let mut delta = LINEAR_INCREASE_SWEEP_DELTAS[volume.sweep_volume()];
            if current_volume >= 0x6000_0000 && delta > 0 {
                delta = (delta >> 2).max(1);
            }
            current_volume.saturating_add(delta as i32)
        };
        volume_level.clamp(0, 0x7FFF_FFFF)
    }
}

/// Exact hardware 15-bit discrete cycle-counter ADSR envelope generator
/// (per Nocash PSX-SPX specification).
#[inline]
pub(crate) fn update_adsr(ch: &mut Channel) {
    if ch.adsr_cycles > 1 {
        ch.adsr_cycles -= 1;
        return;
    }
    update_adsr_slow(ch);
}

fn update_adsr_slow(ch: &mut Channel) {
    let (shift, base_step, decreasing, exponential) = match ch.status {
        EnvelopePhase::Stopped | EnvelopePhase::KeyOn => return,
        EnvelopePhase::Attack => {
            let rate = ch.adsr_level.attack_rate();
            let shift = rate >> 2;
            let step_idx = (rate & 3) as i32;
            (
                shift,
                7 - step_idx,
                false,
                ch.adsr_level.attack_exponential(),
            )
        }
        EnvelopePhase::Decay => (ch.adsr_level.decay_rate(), -8, true, true),
        EnvelopePhase::Sustain => {
            let rate = ch.adsr_rate.sustain_rate();
            let shift = rate >> 2;
            let step_idx = (rate & 3) as i32;
            let dec = ch.adsr_rate.sustain_decrease();
            let base = if dec { -(8 - step_idx) } else { 7 - step_idx };
            (shift, base, dec, ch.adsr_rate.sustain_exponential())
        }
        EnvelopePhase::Release => (
            ch.adsr_rate.release_rate(),
            -8,
            true,
            ch.adsr_rate.release_exponential(),
        ),
    };

    let mut adsr_cycles: i32 = 1i32 << shift.saturating_sub(11);
    let mut adsr_step: i32 = base_step << 11u32.saturating_sub(shift);

    if exponential {
        if !decreasing {
            if ch.adsr_envx >= 0x6000 {
                if shift < 10 {
                    adsr_step >>= 2;
                } else if shift == 10 {
                    adsr_step >>= 1;
                    adsr_cycles = adsr_cycles.saturating_mul(2);
                } else {
                    adsr_cycles = adsr_cycles.saturating_mul(4);
                }
            }
        } else {
            adsr_step = (adsr_step * ch.adsr_envx) >> 15;
        }
    }

    if ch.adsr_cycles <= 0 {
        ch.adsr_cycles = adsr_cycles;
    }
    ch.adsr_cycles = ch.adsr_cycles.wrapping_sub(1);
    if ch.adsr_cycles > 0 {
        return;
    }

    let next = (ch.adsr_envx + adsr_step).clamp(0, MAX_ADSR_LEVEL);
    ch.adsr_envx = next;

    match ch.status {
        EnvelopePhase::Attack => {
            if next >= MAX_ADSR_LEVEL {
                ch.status = EnvelopePhase::Decay;
                ch.adsr_cycles = 0;
            }
        }
        EnvelopePhase::Decay => {
            let sustain_target = ((ch.adsr_level.sustain_level() + 1) * 0x800) as i32;
            if next < sustain_target {
                ch.status = EnvelopePhase::Sustain;
                ch.adsr_cycles = 0;
            }
        }
        EnvelopePhase::Release => {
            if next <= 0 {
                ch.status = EnvelopePhase::Stopped;
                ch.adsr_cycles = 0;
            }
        }
        EnvelopePhase::Sustain | EnvelopePhase::Stopped | EnvelopePhase::KeyOn => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_increase_sweep_ramps_smoothly_with_knee() {
        // 0xC000 = sweep (0x8000) | exponential (0x4000) | increase (0x0000) | rate 0x20
        let vol = ChannelVolume(0xC020);
        let mut level = 0i32;

        // First step from 0 should use full linear-increase delta (below 0x6000_0000 knee).
        let first = compute_channel_volume(vol, level);
        let expected_fast = LINEAR_INCREASE_SWEEP_DELTAS[0x20] as i32;
        assert_eq!(first, expected_fast);
        assert!(first > 0);

        // Ramp until we cross the 0x6000_0000 knee and verify strict monotonicity.
        let mut prev = level;
        let mut crossed_knee = false;
        let mut knee_step = 0i32;
        for _ in 0..10_000 {
            level = compute_channel_volume(vol, prev);
            assert!(level >= prev, "sweep must be monotonically non-decreasing");
            if prev >= 0x6000_0000 && !crossed_knee && level < 0x7FFF_FFFF {
                crossed_knee = true;
                knee_step = level - prev;
            }
            prev = level;
            if level == 0x7FFF_FFFF {
                break;
            }
        }
        assert!(crossed_knee, "must cross 0x6000_0000 knee");
        assert_eq!(
            knee_step,
            expected_fast >> 2,
            "step above 0x6000_0000 must slow down by 4x"
        );
        assert_eq!(level, 0x7FFF_FFFF, "must saturate at max volume");
        // One more step stays saturated (never wraps to 0).
        assert_eq!(compute_channel_volume(vol, level), 0x7FFF_FFFF);
    }
}
