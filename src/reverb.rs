// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! Hardware 16-bit saturating fixed-point reverb engine with 39-tap half-band FIR.

use crate::spu_base::clamp16;

pub(crate) const REVERB_PARAM_COUNT: usize = 32;

/// Typed reverb parameter register indices (`0..32`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum ReverbParam {
    FbSrcA = 0,
    FbSrcB = 1,
    IirAlpha = 2,
    AccCoefA = 3,
    AccCoefB = 4,
    AccCoefC = 5,
    AccCoefD = 6,
    IirCoef = 7,
    FbAlpha = 8,
    FbX = 9,
    IirDestA0 = 10,
    IirDestA1 = 11,
    AccSrcA0 = 12,
    AccSrcA1 = 13,
    AccSrcB0 = 14,
    AccSrcB1 = 15,
    IirSrcA0 = 16,
    IirSrcA1 = 17,
    IirDestB0 = 18,
    IirDestB1 = 19,
    AccSrcC0 = 20,
    AccSrcC1 = 21,
    AccSrcD0 = 22,
    AccSrcD1 = 23,
    IirSrcB0 = 24,
    IirSrcB1 = 25,
    MixDestA0 = 26,
    MixDestA1 = 27,
    MixDestB0 = 28,
    MixDestB1 = 29,
    InCoefL = 30,
    InCoefR = 31,
}

impl ReverbParam {
    const ALL: [Self; REVERB_PARAM_COUNT] = [
        Self::FbSrcA,
        Self::FbSrcB,
        Self::IirAlpha,
        Self::AccCoefA,
        Self::AccCoefB,
        Self::AccCoefC,
        Self::AccCoefD,
        Self::IirCoef,
        Self::FbAlpha,
        Self::FbX,
        Self::IirDestA0,
        Self::IirDestA1,
        Self::AccSrcA0,
        Self::AccSrcA1,
        Self::AccSrcB0,
        Self::AccSrcB1,
        Self::IirSrcA0,
        Self::IirSrcA1,
        Self::IirDestB0,
        Self::IirDestB1,
        Self::AccSrcC0,
        Self::AccSrcC1,
        Self::AccSrcD0,
        Self::AccSrcD1,
        Self::IirSrcB0,
        Self::IirSrcB1,
        Self::MixDestA0,
        Self::MixDestA1,
        Self::MixDestB0,
        Self::MixDestB1,
        Self::InCoefL,
        Self::InCoefR,
    ];

    #[inline]
    pub(crate) const fn from_index(idx: usize) -> Option<Self> {
        if idx < REVERB_PARAM_COUNT {
            Some(Self::ALL[idx])
        } else {
            None
        }
    }

    #[inline]
    pub(crate) const fn is_address(self) -> bool {
        !matches!(
            self,
            Self::IirAlpha
                | Self::AccCoefA
                | Self::AccCoefB
                | Self::AccCoefC
                | Self::AccCoefD
                | Self::IirCoef
                | Self::FbAlpha
                | Self::FbX
                | Self::InCoefL
                | Self::InCoefR
        )
    }
}

/// Symmetric half-band 39-tap FIR filter odd-offset coefficients (offsets ±19, ±17, ..., ±1).
/// Center tap (offset 0) is `0x4000`; all other even-offset taps are `0`.
const REVERB_FIR_ODD_TAPS: [i32; 10] = [
    -0x0001, 0x0002, -0x000A, 0x0023, -0x0067, 0x010A, -0x0268, 0x0534, -0x0B90, 0x2806,
];
const REVERB_FIR_CENTER_TAP: i32 = 0x4000;

#[inline]
const fn sub_mod(a: u32, b: u32, m: u32) -> u32 {
    if a >= b {
        a - b
    } else {
        a + m - b
    }
}

#[inline]
const fn neg16(v: i32) -> i32 {
    clamp16(-v)
}

/// Hardware IIR `(1 - IIR_ALPHA) * sample` term in Q14 (pre-`>> 1`).
/// Handles `IIR_ALPHA == -32768` via saturating negation so `-(-32768) -> 0x7FFF`.
#[inline]
const fn reverb_iiasm(iir_alpha: i32, sample: i32) -> i32 {
    (sample << 1) + ((sample * neg16(iir_alpha)) >> 14)
}

/// Pre-reduced per-channel delay offsets modulo `work_len` (all in `0..work_len`).
#[derive(Debug, Default, Clone, Copy)]
struct ChannelOffsets {
    m_same: u32,
    m_same_m2: u32,
    d_same: u32,
    m_diff: u32,
    m_diff_m2: u32,
    d_diff: u32,
    acc_a: u32,
    acc_b: u32,
    acc_c: u32,
    acc_d: u32,
    m_apf1: u32,
    fb_apf1: u32,
    m_apf2: u32,
    fb_apf2: u32,
}

/// Per-core hardware reverb engine state.
pub(crate) struct ReverbState {
    ram_size: u32,
    work_addr_start: u32,
    work_addr_end: u32,
    curr_addr: u32,
    ticks: i32,
    params: [u32; REVERB_PARAM_COUNT],
    reduced: [ChannelOffsets; 2],
    offsets_dirty: bool,
    in_buf: [[i16; 64]; 2],
    in_pos: usize,
    out_buf: [[i16; 32]; 2],
    out_pos: [usize; 2],
}

impl ReverbState {
    pub(crate) fn new(ram_size: u32) -> Self {
        Self {
            ram_size,
            work_addr_start: 0,
            work_addr_end: ram_size.min(0x80000),
            curr_addr: 0,
            ticks: 0,
            params: [0; REVERB_PARAM_COUNT],
            reduced: [ChannelOffsets::default(); 2],
            offsets_dirty: true,
            in_buf: [[0; 64]; 2],
            in_pos: 0,
            out_buf: [[0; 32]; 2],
            out_pos: [0; 2],
        }
    }

    pub(crate) fn reset(&mut self) {
        self.work_addr_start = 0;
        self.work_addr_end = self.ram_size.min(0x80000);
        self.curr_addr = 0;
        self.ticks = 0;
        self.params = [0; REVERB_PARAM_COUNT];
        self.reduced = [ChannelOffsets::default(); 2];
        self.offsets_dirty = true;
        self.in_buf = [[0; 64]; 2];
        self.in_pos = 0;
        self.out_buf = [[0; 32]; 2];
        self.out_pos = [0; 2];
    }

    #[inline]
    pub(crate) fn work_address_start(&self) -> u32 {
        self.work_addr_start
    }

    #[inline]
    pub(crate) fn set_work_address_start(&mut self, address: u32) {
        self.work_addr_start = address;
        self.curr_addr = address;
        self.offsets_dirty = true;
    }

    #[inline]
    pub(crate) fn work_address_end(&self) -> u32 {
        self.work_addr_end
    }

    #[inline]
    pub(crate) fn set_work_address_end(&mut self, address: u32) {
        self.work_addr_end = address;
        self.offsets_dirty = true;
    }

    #[inline]
    pub(crate) fn has_valid_work_area(&self) -> bool {
        self.work_addr_start < self.work_addr_end
    }

    #[inline]
    pub(crate) fn param(&self, param: ReverbParam) -> u32 {
        self.params[param as usize]
    }

    #[inline]
    pub(crate) fn set_param(&mut self, param: ReverbParam, value: u32) {
        self.params[param as usize] = value;
        if param.is_address() {
            self.offsets_dirty = true;
        }
    }

    #[inline]
    fn coef(&self, param: ReverbParam) -> i32 {
        (self.params[param as usize] as i16) as i32
    }

    /// Refreshes pre-reduced modulo offsets if the work area or any address param changed.
    #[inline]
    pub(crate) fn prepare_render(&mut self) {
        if !self.offsets_dirty {
            return;
        }
        let work_len = self.work_addr_end - self.work_addr_start;
        let rem = |p: ReverbParam| self.params[p as usize] % work_len;
        let two = 2 % work_len;

        let d_apf1 = rem(ReverbParam::FbSrcA);
        let d_apf2 = rem(ReverbParam::FbSrcB);

        let m_same_0 = rem(ReverbParam::IirDestA0);
        let m_diff_0 = rem(ReverbParam::IirDestB0);
        let m_apf1_0 = rem(ReverbParam::MixDestA0);
        let m_apf2_0 = rem(ReverbParam::MixDestB0);
        self.reduced[0] = ChannelOffsets {
            m_same: m_same_0,
            m_same_m2: sub_mod(m_same_0, two, work_len),
            d_same: rem(ReverbParam::IirSrcA0),
            m_diff: m_diff_0,
            m_diff_m2: sub_mod(m_diff_0, two, work_len),
            d_diff: rem(ReverbParam::IirSrcB1),
            acc_a: rem(ReverbParam::AccSrcA0),
            acc_b: rem(ReverbParam::AccSrcB0),
            acc_c: rem(ReverbParam::AccSrcC0),
            acc_d: rem(ReverbParam::AccSrcD0),
            m_apf1: m_apf1_0,
            fb_apf1: sub_mod(m_apf1_0, d_apf1, work_len),
            m_apf2: m_apf2_0,
            fb_apf2: sub_mod(m_apf2_0, d_apf2, work_len),
        };

        let m_same_1 = rem(ReverbParam::IirDestA1);
        let m_diff_1 = rem(ReverbParam::IirDestB1);
        let m_apf1_1 = rem(ReverbParam::MixDestA1);
        let m_apf2_1 = rem(ReverbParam::MixDestB1);
        self.reduced[1] = ChannelOffsets {
            m_same: m_same_1,
            m_same_m2: sub_mod(m_same_1, two, work_len),
            d_same: rem(ReverbParam::IirSrcA1),
            m_diff: m_diff_1,
            m_diff_m2: sub_mod(m_diff_1, two, work_len),
            d_diff: rem(ReverbParam::IirSrcB0),
            acc_a: rem(ReverbParam::AccSrcA1),
            acc_b: rem(ReverbParam::AccSrcB1),
            acc_c: rem(ReverbParam::AccSrcC1),
            acc_d: rem(ReverbParam::AccSrcD1),
            m_apf1: m_apf1_1,
            fb_apf1: sub_mod(m_apf1_1, d_apf1, work_len),
            m_apf2: m_apf2_1,
            fb_apf2: sub_mod(m_apf2_1, d_apf2, work_len),
        };

        self.offsets_dirty = false;
    }

    #[inline]
    fn current_rel(&self, work_len: u32) -> u32 {
        if self.curr_addr >= self.work_addr_start && self.curr_addr < self.work_addr_end {
            self.curr_addr - self.work_addr_start
        } else {
            (self.curr_addr as i64 - self.work_addr_start as i64).rem_euclid(work_len as i64) as u32
        }
    }

    #[inline]
    fn ram_addr(&self, rel: u32, work_len: u32, reduced_offset: u32) -> usize {
        let sum = rel + reduced_offset;
        let wrapped = if sum >= work_len { sum - work_len } else { sum };
        ((self.work_addr_start + wrapped) & (self.ram_size - 1)) as usize
    }

    #[inline]
    fn get_sample(&self, rel: u32, work_len: u32, reduced_offset: u32, ram: &[u8]) -> i32 {
        let mask = (self.ram_size - 1) as usize;
        let off = self.ram_addr(rel, work_len, reduced_offset);
        i16::from_le_bytes([ram[off], ram[(off + 1) & mask]]) as i32
    }

    #[inline]
    fn set_sample(&self, rel: u32, work_len: u32, reduced_offset: u32, value: i32, ram: &mut [u8]) {
        let mask = (self.ram_size - 1) as usize;
        let off = self.ram_addr(rel, work_len, reduced_offset);
        let bytes = (clamp16(value) as i16).to_le_bytes();
        ram[off] = bytes[0];
        ram[(off + 1) & mask] = bytes[1];
    }

    /// 39-tap symmetric half-band FIR downsample (44.1/48 kHz -> 22.05/24 kHz).
    #[inline]
    fn fir_downsample(&self, ch: usize) -> i32 {
        let buf = &self.in_buf[ch];
        let pos = self.in_pos;
        let mut acc = REVERB_FIR_CENTER_TAP * (buf[pos.wrapping_sub(19) & 63] as i32);
        for (j, &tap) in REVERB_FIR_ODD_TAPS.iter().enumerate() {
            let a = buf[pos.wrapping_sub(38 - j * 2) & 63] as i32;
            let b = buf[pos.wrapping_sub(j * 2) & 63] as i32;
            acc += tap * (a + b);
        }
        clamp16(acc >> 15)
    }

    /// 39-tap symmetric half-band zero-stuffed FIR upsample (22.05/24 kHz -> 44.1/48 kHz).
    #[inline]
    fn fir_upsample(&self, ch: usize, is_active: bool) -> i32 {
        let buf = &self.out_buf[ch];
        let pos = self.out_pos[ch];
        if is_active {
            let mut acc = 0i32;
            for (j, &tap) in REVERB_FIR_ODD_TAPS.iter().enumerate() {
                let a = buf[pos.wrapping_sub(19 - j) & 31] as i32;
                let b = buf[pos.wrapping_sub(j) & 31] as i32;
                acc += tap * (a + b);
            }
            clamp16(acc >> 14)
        } else {
            buf[pos.wrapping_sub(9) & 31] as i32
        }
    }

    /// Runs one channel (0 = Left on odd cycles, 1 = Right on even cycles) of the
    /// hardware 16-bit fixed-point reverb engine.
    fn step_channel(&self, ch: usize, ram: &mut [u8]) -> i16 {
        let work_len = self.work_addr_end - self.work_addr_start;
        let rel = self.current_rel(work_len);
        let offs = self.reduced[ch];

        let in_s = self.fir_downsample(ch);

        let v_in = self.coef(if ch == 0 {
            ReverbParam::InCoefL
        } else {
            ReverbParam::InCoefR
        });
        let v_wall = self.coef(ReverbParam::IirCoef);
        let v_iir = self.coef(ReverbParam::IirAlpha);

        let in_term = (in_s * v_in) >> 14;

        // Same-side reflection (IIR).
        let iir_in_same = clamp16(
            (((self.get_sample(rel, work_len, offs.d_same, ram) * v_wall) >> 14) + in_term) >> 1,
        );
        let prev_same = self.get_sample(rel, work_len, offs.m_same_m2, ram);
        let iir_same =
            clamp16((((iir_in_same * v_iir) >> 14) + reverb_iiasm(v_iir, prev_same)) >> 1);
        self.set_sample(rel, work_len, offs.m_same, iir_same, ram);

        // Cross-coupled different-side reflection (IIR).
        let iir_in_diff = clamp16(
            (((self.get_sample(rel, work_len, offs.d_diff, ram) * v_wall) >> 14) + in_term) >> 1,
        );
        let prev_diff = self.get_sample(rel, work_len, offs.m_diff_m2, ram);
        let iir_diff =
            clamp16((((iir_in_diff * v_iir) >> 14) + reverb_iiasm(v_iir, prev_diff)) >> 1);
        self.set_sample(rel, work_len, offs.m_diff, iir_diff, ram);

        // Early echo (4-tap comb filter): unclamped Q14 accumulator folded directly into APF1.
        let v_comb1 = self.coef(ReverbParam::AccCoefA);
        let v_comb2 = self.coef(ReverbParam::AccCoefB);
        let v_comb3 = self.coef(ReverbParam::AccCoefC);
        let v_comb4 = self.coef(ReverbParam::AccCoefD);

        let comb_acc = ((v_comb1 * self.get_sample(rel, work_len, offs.acc_a, ram)) >> 14)
            + ((v_comb2 * self.get_sample(rel, work_len, offs.acc_b, ram)) >> 14)
            + ((v_comb3 * self.get_sample(rel, work_len, offs.acc_c, ram)) >> 14)
            + ((v_comb4 * self.get_sample(rel, work_len, offs.acc_d, ram)) >> 14);

        // Late reverb APF1 (cascaded into APF2).
        let v_apf1 = self.coef(ReverbParam::FbAlpha);
        let v_apf2 = self.coef(ReverbParam::FbX);

        let fb_a = self.get_sample(rel, work_len, offs.fb_apf1, ram);
        let mda = clamp16((comb_acc + ((fb_a * neg16(v_apf1)) >> 14)) >> 1);
        self.set_sample(rel, work_len, offs.m_apf1, mda, ram);

        let apf1_out_q14 = (fb_a << 1) + ((mda * v_apf1) >> 14);

        // Late reverb APF2.
        let fb_b = self.get_sample(rel, work_len, offs.fb_apf2, ram);
        let mdb = clamp16((apf1_out_q14 + ((fb_b * neg16(v_apf2)) >> 14)) >> 1);
        self.set_sample(rel, work_len, offs.m_apf2, mdb, ram);

        clamp16(((fb_b << 1) + ((mdb * v_apf2) >> 14)) >> 1) as i16
    }

    /// Hardware 16-bit saturating fixed-point reverb engine with alternating L/R
    /// 44.1 kHz cycles and 39-tap half-band FIR downsample/upsample.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update(
        &mut self,
        reverb_sample: [i16; 2],
        unmuted: bool,
        reverb_vol_l: i16,
        reverb_vol_r: i16,
        out_l: &mut i16,
        out_r: &mut i16,
        ram: &mut [u8],
    ) {
        self.in_pos = (self.in_pos + 1) & 63;
        self.in_buf[0][self.in_pos] = reverb_sample[0];
        self.in_buf[1][self.in_pos] = reverb_sample[1];

        let is_odd = (self.ticks & 1) != 0;
        if is_odd {
            let wet_22k_l = self.step_channel(0, ram);
            self.out_pos[0] = (self.out_pos[0] + 1) & 31;
            self.out_buf[0][self.out_pos[0]] = wet_22k_l;
        } else {
            let wet_22k_r = self.step_channel(1, ram);
            self.out_pos[1] = (self.out_pos[1] + 1) & 31;
            self.out_buf[1][self.out_pos[1]] = wet_22k_r;

            self.curr_addr = self.curr_addr.wrapping_add(2);
            if self.curr_addr >= self.work_addr_end {
                self.curr_addr = self.work_addr_start;
            }
        }

        if unmuted {
            if reverb_vol_l != 0 {
                let up_l = self.fir_upsample(0, is_odd);
                let wet_l = (up_l * (reverb_vol_l as i32)) >> 15;
                *out_l = clamp16((*out_l as i32) + wet_l) as i16;
            }
            if reverb_vol_r != 0 {
                let up_r = self.fir_upsample(1, !is_odd);
                let wet_r = (up_r * (reverb_vol_r as i32)) >> 15;
                *out_r = clamp16((*out_r as i32) + wet_r) as i16;
            }
        }

        self.ticks = self.ticks.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverb_39tap_halfband_fir_unity_dc_gain() {
        let mut rvb = ReverbState::new(0x80000);
        // Feed constant DC 16000 into the downsample ring buffer.
        for _ in 0..64 {
            rvb.in_pos = (rvb.in_pos + 1) & 63;
            rvb.in_buf[0][rvb.in_pos] = 16_000;
        }
        let down = rvb.fir_downsample(0);
        // Sum of all 39 taps = 0x7FFE / 0x8000 => 16000 * 32766 >> 15 = 15999.
        assert_eq!(down, (16_000 * 0x7FFE) >> 15);

        // Feed constant DC 16000 into the upsample ring buffer.
        for _ in 0..32 {
            rvb.out_pos[0] = (rvb.out_pos[0] + 1) & 31;
            rvb.out_buf[0][rvb.out_pos[0]] = 16_000;
        }
        let up_even = rvb.fir_upsample(0, false);
        let up_odd = rvb.fir_upsample(0, true);
        assert_eq!(up_even, 16_000);
        assert_eq!(up_odd, (16_000 * 2 * 0x1FFF) >> 14);
    }
}
