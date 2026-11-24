// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! ADPCM block decoder, decoded-sample cache, and 4-tap Gaussian sample reader.

use alloc::boxed::Box;
use alloc::vec;

use crate::spu_base::{clamp16, SpuIrqWatcher};

pub(crate) const BUFFER_SAMPLES: usize = 28;
const PITCH_BASE: u32 = 0x1000;
const GAUSS_HISTORY: usize = 3;

const PREDICTOR_TABLE: [[i32; 2]; 16] = [
    [0, 0],
    [60, 0],
    [115, -52],
    [98, -55],
    [122, -60],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
    [0, 0],
];

/// PlayStation 1 / PlayStation 2 SPU 512-entry 4-tap Gaussian interpolation ROM table.
#[rustfmt::skip]
const GAUSS_TABLE: [i32; 512] = [
    -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001,
    -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0002, 0x0002, 0x0002, 0x0003, 0x0003,
    0x0003, 0x0004, 0x0004, 0x0005, 0x0005, 0x0006, 0x0007, 0x0007,
    0x0008, 0x0009, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E,
    0x000F, 0x0010, 0x0011, 0x0012, 0x0013, 0x0015, 0x0016, 0x0018,
    0x0019, 0x001B, 0x001C, 0x001E, 0x0020, 0x0021, 0x0023, 0x0025,
    0x0027, 0x0029, 0x002C, 0x002E, 0x0030, 0x0033, 0x0035, 0x0038,
    0x003A, 0x003D, 0x0040, 0x0043, 0x0046, 0x0049, 0x004D, 0x0050,
    0x0054, 0x0057, 0x005B, 0x005F, 0x0063, 0x0067, 0x006B, 0x006F,
    0x0074, 0x0078, 0x007D, 0x0082, 0x0087, 0x008C, 0x0091, 0x0096,
    0x009C, 0x00A1, 0x00A7, 0x00AD, 0x00B3, 0x00BA, 0x00C0, 0x00C7,
    0x00CD, 0x00D4, 0x00DB, 0x00E3, 0x00EA, 0x00F2, 0x00FA, 0x0101,
    0x010A, 0x0112, 0x011B, 0x0123, 0x012C, 0x0135, 0x013F, 0x0148,
    0x0152, 0x015C, 0x0166, 0x0171, 0x017B, 0x0186, 0x0191, 0x019C,
    0x01A8, 0x01B4, 0x01C0, 0x01CC, 0x01D9, 0x01E5, 0x01F2, 0x0200,
    0x020D, 0x021B, 0x0229, 0x0237, 0x0246, 0x0255, 0x0264, 0x0273,
    0x0283, 0x0293, 0x02A3, 0x02B4, 0x02C4, 0x02D6, 0x02E7, 0x02F9,
    0x030B, 0x031D, 0x0330, 0x0343, 0x0356, 0x036A, 0x037E, 0x0392,
    0x03A7, 0x03BC, 0x03D1, 0x03E7, 0x03FC, 0x0413, 0x042A, 0x0441,
    0x0458, 0x0470, 0x0488, 0x04A0, 0x04B9, 0x04D2, 0x04EC, 0x0506,
    0x0520, 0x053B, 0x0556, 0x0572, 0x058E, 0x05AA, 0x05C7, 0x05E4,
    0x0601, 0x061F, 0x063E, 0x065C, 0x067C, 0x069B, 0x06BB, 0x06DC,
    0x06FD, 0x071E, 0x0740, 0x0762, 0x0784, 0x07A7, 0x07CB, 0x07EF,
    0x0813, 0x0838, 0x085D, 0x0883, 0x08A9, 0x08D0, 0x08F7, 0x091E,
    0x0946, 0x096F, 0x0998, 0x09C1, 0x09EB, 0x0A16, 0x0A40, 0x0A6C,
    0x0A98, 0x0AC4, 0x0AF1, 0x0B1E, 0x0B4C, 0x0B7A, 0x0BA9, 0x0BD8,
    0x0C07, 0x0C38, 0x0C68, 0x0C99, 0x0CCB, 0x0CFD, 0x0D30, 0x0D63,
    0x0D97, 0x0DCB, 0x0E00, 0x0E35, 0x0E6B, 0x0EA1, 0x0ED7, 0x0F0F,
    0x0F46, 0x0F7F, 0x0FB7, 0x0FF1, 0x102A, 0x1065, 0x109F, 0x10DB,
    0x1116, 0x1153, 0x118F, 0x11CD, 0x120B, 0x1249, 0x1288, 0x12C7,
    0x1307, 0x1347, 0x1388, 0x13C9, 0x140B, 0x144D, 0x1490, 0x14D4,
    0x1517, 0x155C, 0x15A0, 0x15E6, 0x162C, 0x1672, 0x16B9, 0x1700,
    0x1747, 0x1790, 0x17D8, 0x1821, 0x186B, 0x18B5, 0x1900, 0x194B,
    0x1996, 0x19E2, 0x1A2E, 0x1A7B, 0x1AC8, 0x1B16, 0x1B64, 0x1BB3,
    0x1C02, 0x1C51, 0x1CA1, 0x1CF1, 0x1D42, 0x1D93, 0x1DE5, 0x1E37,
    0x1E89, 0x1EDC, 0x1F2F, 0x1F82, 0x1FD6, 0x202A, 0x207F, 0x20D4,
    0x2129, 0x217F, 0x21D5, 0x222C, 0x2282, 0x22DA, 0x2331, 0x2389,
    0x23E1, 0x2439, 0x2492, 0x24EB, 0x2545, 0x259E, 0x25F8, 0x2653,
    0x26AD, 0x2708, 0x2763, 0x27BE, 0x281A, 0x2876, 0x28D2, 0x292E,
    0x298B, 0x29E7, 0x2A44, 0x2AA1, 0x2AFF, 0x2B5C, 0x2BBA, 0x2C18,
    0x2C76, 0x2CD4, 0x2D33, 0x2D91, 0x2DF0, 0x2E4F, 0x2EAE, 0x2F0D,
    0x2F6C, 0x2FCC, 0x302B, 0x308B, 0x30EA, 0x314A, 0x31AA, 0x3209,
    0x3269, 0x32C9, 0x3329, 0x3389, 0x33E9, 0x3449, 0x34A9, 0x3509,
    0x3569, 0x35C9, 0x3629, 0x3689, 0x36E8, 0x3748, 0x37A8, 0x3807,
    0x3867, 0x38C6, 0x3926, 0x3985, 0x39E4, 0x3A43, 0x3AA2, 0x3B00,
    0x3B5F, 0x3BBD, 0x3C1B, 0x3C79, 0x3CD7, 0x3D35, 0x3D92, 0x3DEF,
    0x3E4C, 0x3EA9, 0x3F05, 0x3F62, 0x3FBD, 0x4019, 0x4074, 0x40D0,
    0x412A, 0x4185, 0x41DF, 0x4239, 0x4292, 0x42EB, 0x4344, 0x439C,
    0x43F4, 0x444C, 0x44A3, 0x44FA, 0x4550, 0x45A6, 0x45FC, 0x4651,
    0x46A6, 0x46FA, 0x474E, 0x47A1, 0x47F4, 0x4846, 0x4898, 0x48E9,
    0x493A, 0x498A, 0x49D9, 0x4A29, 0x4A77, 0x4AC5, 0x4B13, 0x4B5F,
    0x4BAC, 0x4BF7, 0x4C42, 0x4C8D, 0x4CD7, 0x4D20, 0x4D68, 0x4DB0,
    0x4DF7, 0x4E3E, 0x4E84, 0x4EC9, 0x4F0E, 0x4F52, 0x4F95, 0x4FD7,
    0x5019, 0x505A, 0x509A, 0x50DA, 0x5118, 0x5156, 0x5194, 0x51D0,
    0x520C, 0x5247, 0x5281, 0x52BA, 0x52F3, 0x532A, 0x5361, 0x5397,
    0x53CC, 0x5401, 0x5434, 0x5467, 0x5499, 0x54CA, 0x54FA, 0x5529,
    0x5558, 0x5585, 0x55B2, 0x55DE, 0x5609, 0x5632, 0x565B, 0x5684,
    0x56AB, 0x56D1, 0x56F6, 0x571B, 0x573E, 0x5761, 0x5782, 0x57A3,
    0x57C3, 0x57E2, 0x57FF, 0x581C, 0x5838, 0x5853, 0x586D, 0x5886,
    0x589E, 0x58B5, 0x58CB, 0x58E0, 0x58F4, 0x5907, 0x5919, 0x592A,
    0x593A, 0x5949, 0x5958, 0x5965, 0x5971, 0x597C, 0x5986, 0x598F,
    0x5997, 0x599E, 0x59A4, 0x59A9, 0x59AD, 0x59B0, 0x59B2, 0x59B3,
];

const CACHE_SLOTS: usize = 1024;
const CACHE_SLOT_MASK: usize = CACHE_SLOTS - 1;

#[derive(Clone, Copy)]
struct CacheKey {
    address: u32,
    s1: i16,
    s2: i16,
}

#[derive(Clone, Copy)]
struct CacheSlot {
    valid: bool,
    address: u32,
    in_s1: i16,
    in_s2: i16,
    out_s1: i16,
    out_s2: i16,
    samples: [i16; BUFFER_SAMPLES],
}

impl Default for CacheSlot {
    fn default() -> Self {
        Self {
            valid: false,
            address: 0,
            in_s1: 0,
            in_s2: 0,
            out_s1: 0,
            out_s2: 0,
            samples: [0; BUFFER_SAMPLES],
        }
    }
}

/// Flat, zero-allocation decoded ADPCM block cache keyed by `(address, s1, s2)`.
pub(crate) struct SpuSampleCache {
    ram_size: u32,
    slots: Box<[CacheSlot]>,
}

impl SpuSampleCache {
    pub(crate) fn new(ram_size: u32) -> Self {
        debug_assert!(ram_size.is_power_of_two());
        Self {
            ram_size,
            slots: vec![CacheSlot::default(); CACHE_SLOTS].into_boxed_slice(),
        }
    }

    #[inline]
    fn slot_index(address: u32) -> usize {
        ((address >> 4) as usize) & CACHE_SLOT_MASK
    }

    #[inline]
    fn get_item(&self, key: CacheKey) -> Option<&CacheSlot> {
        let slot = &self.slots[Self::slot_index(key.address)];
        if slot.valid && slot.address == key.address && slot.in_s1 == key.s1 && slot.in_s2 == key.s2
        {
            Some(slot)
        } else {
            None
        }
    }

    #[inline]
    fn register_item(&mut self, key: CacheKey, samples: &[i16], out_s1: i16, out_s2: i16) {
        let slot = &mut self.slots[Self::slot_index(key.address)];
        slot.valid = true;
        slot.address = key.address;
        slot.in_s1 = key.s1;
        slot.in_s2 = key.s2;
        slot.out_s1 = out_s1;
        slot.out_s2 = out_s2;
        slot.samples.copy_from_slice(samples);
    }

    pub(crate) fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            slot.valid = false;
        }
    }

    /// Fast direct-slot invalidation for a 2-byte aligned write (e.g. PS1 capture buffers).
    /// Handles both aligned and unaligned overlapping blocks with zero loops.
    #[inline]
    pub(crate) fn invalidate_aligned_word(&mut self, address: u32) {
        let mask = self.ram_size - 1;
        let addr = address & mask;
        let idx = Self::slot_index(addr);
        let slot = &mut self.slots[idx];
        if slot.valid && ((addr.wrapping_add(1).wrapping_sub(slot.address)) & mask) <= 16 {
            slot.valid = false;
        }
        let prev_idx = idx.wrapping_sub(1) & CACHE_SLOT_MASK;
        let prev = &mut self.slots[prev_idx];
        if prev.valid && ((addr.wrapping_add(1).wrapping_sub(prev.address)) & mask) <= 16 {
            prev.valid = false;
        }
    }

    /// Invalidates all cached blocks that overlap `[address, address + size)`,
    /// including unaligned blocks up to 15 bytes before `address` and wrap-around
    /// across `ram_size`.
    pub(crate) fn clear_range(&mut self, address: u32, size: u32) {
        if size == 0 {
            return;
        }
        if size >= (CACHE_SLOTS as u32) * 16 || size >= self.ram_size {
            self.clear();
            return;
        }
        let mask = self.ram_size - 1;
        let addr = address & mask;
        let start = addr.saturating_sub(15);
        let end = addr.saturating_add(size).min(mask);
        self.clear_linear_range(start, end);
        // Unaligned blocks starting in [ram_size - 15, ram_size) wrap into [0, 15).
        if addr < 15 {
            self.clear_linear_range(self.ram_size - (15 - addr), mask);
        }
        // Writes straddling ram_size wrap into [0, wrap_len).
        if let Some(wrap_end) = addr
            .checked_add(size)
            .and_then(|e| e.checked_sub(self.ram_size))
        {
            if wrap_end > 0 {
                self.clear_linear_range(0, wrap_end.min(mask));
            }
        }
    }

    fn clear_linear_range(&mut self, start: u32, end: u32) {
        let first_block = start >> 4;
        let last_block = end >> 4;
        if last_block.saturating_sub(first_block) >= CACHE_SLOTS as u32 {
            self.clear();
            return;
        }
        for block in first_block..=last_block {
            let slot = &mut self.slots[(block as usize) & CACHE_SLOT_MASK];
            if slot.valid && slot.address >= start && slot.address <= end {
                slot.valid = false;
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct HalfBlockMeta {
    addr: u32,
    end_flag: bool,
    done: bool,
    repeat_set: bool,
    repeat_addr: u32,
}

#[derive(Clone)]
pub(crate) struct SampleReader {
    ram_size: u32,
    src_sample_idx: u32,
    sample_step: u32,
    src_sampling_rate: u32,
    dst_sampling_rate: u32,
    next_sample_addr: u32,
    repeat_addr: u32,
    latched_repeat_addr: u32,
    buffer: [i16; GAUSS_HISTORY + BUFFER_SAMPLES * 2],
    cur_meta: HalfBlockMeta,
    next_meta: HalfBlockMeta,
    s1: i32,
    s2: i32,
    done: bool,
    next_valid: bool,
    end_flag: bool,
    did_change_repeat: bool,
}

impl SampleReader {
    pub(crate) fn new(ram_size: u32) -> Self {
        debug_assert!(ram_size.is_power_of_two());
        Self {
            ram_size,
            src_sample_idx: 0,
            sample_step: 0,
            src_sampling_rate: 0,
            dst_sampling_rate: 0,
            next_sample_addr: 0,
            repeat_addr: 0,
            latched_repeat_addr: 0,
            buffer: [0; GAUSS_HISTORY + BUFFER_SAMPLES * 2],
            cur_meta: HalfBlockMeta::default(),
            next_meta: HalfBlockMeta::default(),
            s1: 0,
            s2: 0,
            done: false,
            next_valid: false,
            end_flag: false,
            did_change_repeat: false,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.next_sample_addr = 0;
        self.repeat_addr = 0;
        self.latched_repeat_addr = 0;
        self.buffer = [0; GAUSS_HISTORY + BUFFER_SAMPLES * 2];
        self.cur_meta = HalfBlockMeta::default();
        self.next_meta = HalfBlockMeta::default();
        self.src_sample_idx = 0;
        self.src_sampling_rate = 0;
        self.dst_sampling_rate = 0;
        self.sample_step = 0;
        self.s1 = 0;
        self.s2 = 0;
        self.done = false;
        self.did_change_repeat = false;
        self.next_valid = false;
        self.end_flag = false;
    }

    #[inline]
    pub(crate) fn is_inert(&self) -> bool {
        self.sample_step == 0
            && (self.src_sample_idx / PITCH_BASE) < BUFFER_SAMPLES as u32
            && !self.done
            && !self.did_change_repeat
    }

    pub(crate) fn set_destination_sampling_rate(&mut self, sampling_rate: u32) {
        self.dst_sampling_rate = sampling_rate;
        self.update_sample_step();
    }

    fn set_params(&mut self, address: u32, repeat: u32) {
        let mask = self.ram_size - 1;
        self.src_sample_idx = 0;
        self.next_sample_addr = address & mask;
        self.repeat_addr = repeat & mask;
        self.latched_repeat_addr = self.repeat_addr;
        self.cur_meta = HalfBlockMeta {
            addr: self.next_sample_addr,
            ..HalfBlockMeta::default()
        };
        self.next_meta = HalfBlockMeta::default();
        self.s1 = 0;
        self.s2 = 0;
        self.next_valid = false;
        self.done = false;
        self.did_change_repeat = false;
    }

    pub(crate) fn set_params_read(
        &mut self,
        address: u32,
        repeat: u32,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        self.set_params(address, repeat);
        self.advance_buffer(ram, cache, irq_watcher);
    }

    pub(crate) fn set_params_no_read(&mut self, address: u32, repeat: u32) {
        self.set_params(address, repeat);
        self.buffer = [0; GAUSS_HISTORY + BUFFER_SAMPLES * 2];
    }

    pub(crate) fn set_pitch(&mut self, base_sampling_rate: u32, pitch: u16) {
        self.src_sampling_rate = base_sampling_rate.wrapping_mul(pitch.min(0x3FFF) as u32);
        self.update_sample_step();
    }

    fn update_sample_step(&mut self) {
        if let Some(step) = self.src_sampling_rate.checked_div(self.dst_sampling_rate) {
            self.sample_step = step;
        }
    }

    #[inline]
    pub(crate) fn step_for_pitch(&self, base_sampling_rate: u32, pitch: u16) -> u32 {
        let clamped = pitch.min(0x3FFF) as u32;
        if base_sampling_rate == self.dst_sampling_rate {
            if self.dst_sampling_rate == 0 {
                0
            } else {
                clamped
            }
        } else {
            base_sampling_rate
                .wrapping_mul(clamped)
                .checked_div(self.dst_sampling_rate)
                .unwrap_or(0)
        }
    }

    #[inline]
    pub(crate) fn get_sample(
        &mut self,
        step_override: Option<u32>,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) -> i32 {
        let src_sample_idx = (self.src_sample_idx / PITCH_BASE) as usize;
        let gauss_idx = ((self.src_sample_idx >> 4) & 0xFF) as usize;
        let window = &self.buffer[src_sample_idx..src_sample_idx + 4];
        let s0 = window[0] as i32; // oldest
        let s1 = window[1] as i32; // older
        let s2 = window[2] as i32; // old
        let s3 = window[3] as i32; // new
        let g0 = GAUSS_TABLE[0x0FF - gauss_idx];
        let g1 = GAUSS_TABLE[0x1FF - gauss_idx];
        let g2 = GAUSS_TABLE[0x100 + gauss_idx];
        let g3 = GAUSS_TABLE[gauss_idx];
        // Hardware Gaussian interpolation: full-precision 4-tap MAC then >> 15.
        let out = clamp16((g0 * s0 + g1 * s1 + g2 * s2 + g3 * s3) >> 15);

        let step = step_override.unwrap_or(self.sample_step);
        let mut next_idx = (self.src_sample_idx as u64) + (step as u64);
        let block_span = (BUFFER_SAMPLES as u64) * (PITCH_BASE as u64);
        if src_sample_idx >= BUFFER_SAMPLES {
            next_idx = next_idx.saturating_sub(block_span);
            self.advance_buffer(ram, cache, irq_watcher);
        }
        let max_valid_idx = ((BUFFER_SAMPLES * 2 - 1) as u64) * (PITCH_BASE as u64);
        while next_idx >= max_valid_idx {
            next_idx -= block_span;
            self.advance_buffer(ram, cache, irq_watcher);
        }
        self.src_sample_idx = next_idx as u32;
        out
    }

    fn advance_buffer(
        &mut self,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        if self.next_valid {
            // Commit end/done flags from the block that just finished playing.
            if self.cur_meta.end_flag {
                self.end_flag = true;
            }
            if self.cur_meta.done {
                self.done = true;
            }
            self.buffer
                .copy_within(BUFFER_SAMPLES..(GAUSS_HISTORY + BUFFER_SAMPLES * 2), 0);
            self.cur_meta = self.next_meta;
            if self.cur_meta.repeat_set {
                self.latched_repeat_addr = self.cur_meta.repeat_addr;
                self.did_change_repeat = true;
            }
            self.next_meta =
                self.unpack_into_half(GAUSS_HISTORY + BUFFER_SAMPLES, ram, cache, irq_watcher);
        } else {
            self.buffer[..GAUSS_HISTORY].fill(0);
            self.cur_meta = self.unpack_into_half(GAUSS_HISTORY, ram, cache, irq_watcher);
            if self.cur_meta.repeat_set {
                self.latched_repeat_addr = self.cur_meta.repeat_addr;
                self.did_change_repeat = true;
            }
            self.next_meta =
                self.unpack_into_half(GAUSS_HISTORY + BUFFER_SAMPLES, ram, cache, irq_watcher);
            self.next_valid = true;
        }
    }

    fn unpack_into_half(
        &mut self,
        dst_offset: usize,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) -> HalfBlockMeta {
        let mask = (self.ram_size - 1) as usize;
        let block_addr = self.next_sample_addr;
        let base_addr = (block_addr as usize) & mask;

        irq_watcher.check_irq(block_addr);

        let header = ram[base_addr];
        let flags = ram[(base_addr + 1) & mask];
        let mut shift_factor = (header & 0x0F) as u32;
        if shift_factor > 12 {
            shift_factor = 9;
        }
        let predict_number = (header >> 4) as usize;

        let mut cache_key = CacheKey {
            address: block_addr,
            s1: self.s1 as i16,
            s2: self.s2 as i16,
        };
        if predict_number == 0 || predict_number >= 5 {
            cache_key.s1 = 0;
            cache_key.s2 = 0;
        }

        let dst = &mut self.buffer[dst_offset..dst_offset + BUFFER_SAMPLES];
        if let Some(item) = cache.get_item(cache_key) {
            dst.copy_from_slice(&item.samples);
            self.s1 = item.out_s1 as i32;
            self.s2 = item.out_s2 as i32;
        } else {
            let k0 = PREDICTOR_TABLE[predict_number][0];
            let k1 = PREDICTOR_TABLE[predict_number][1];
            let mut s1 = self.s1;
            let mut s2 = self.s2;

            let mut decode_pair = |sample_byte: u8, out: &mut [i16]| {
                let first_sample = (((sample_byte & 0x0F) as u16) << 12) as i16;
                let second_sample = (((sample_byte & 0xF0) as u16) << 8) as i16;

                let sample0 = clamp16(
                    ((first_sample as i32) >> shift_factor) + ((s1 * k0) >> 6) + ((s2 * k1) >> 6),
                );
                s2 = s1;
                s1 = sample0;
                out[0] = sample0 as i16;

                let sample1 = clamp16(
                    ((second_sample as i32) >> shift_factor) + ((s1 * k0) >> 6) + ((s2 * k1) >> 6),
                );
                s2 = s1;
                s1 = sample1;
                out[1] = sample1 as i16;
            };

            if base_addr + 16 <= ram.len() {
                let payload = &ram[base_addr + 2..base_addr + 16];
                for (out, &sample_byte) in dst.chunks_exact_mut(2).zip(payload.iter()) {
                    decode_pair(sample_byte, out);
                }
            } else {
                for (i, out) in dst.chunks_exact_mut(2).enumerate() {
                    let sample_byte = ram[(base_addr + 2 + i) & mask];
                    decode_pair(sample_byte, out);
                }
            }

            self.s1 = s1;
            self.s2 = s2;
            cache.register_item(cache_key, dst, s1 as i16, s2 as i16);
        }

        let mut meta = HalfBlockMeta {
            addr: block_addr,
            end_flag: false,
            done: false,
            repeat_set: false,
            repeat_addr: self.repeat_addr,
        };

        if (flags & 0x04) != 0 {
            self.repeat_addr = block_addr;
            meta.repeat_set = true;
            meta.repeat_addr = block_addr;
        }

        self.next_sample_addr = block_addr.wrapping_add(0x10) & (self.ram_size - 1);

        if (flags & 0x01) != 0 {
            meta.end_flag = true;
            self.next_sample_addr = self.repeat_addr;
            if (flags & 0x03) != 0x03 {
                meta.done = true;
            }
        }

        meta
    }

    #[inline]
    pub(crate) fn repeat(&self) -> u32 {
        self.latched_repeat_addr
    }

    #[inline]
    pub(crate) fn set_repeat(&mut self, repeat_addr: u32) {
        self.repeat_addr = repeat_addr & (self.ram_size - 1);
    }

    #[inline]
    pub(crate) fn current(&self) -> u32 {
        let intra_sample_idx = ((self.src_sample_idx / PITCH_BASE) / 2).min(0x0E);
        self.cur_meta.addr.wrapping_add(intra_sample_idx)
    }

    #[inline]
    pub(crate) fn is_done(&self) -> bool {
        self.done
    }

    #[inline]
    pub(crate) fn clear_is_done(&mut self) {
        self.done = false;
    }

    #[inline]
    pub(crate) fn end_flag(&self) -> bool {
        self.end_flag
    }

    #[inline]
    pub(crate) fn clear_end_flag(&mut self) {
        self.end_flag = false;
    }

    #[inline]
    pub(crate) fn did_change_repeat(&self) -> bool {
        self.did_change_repeat
    }

    #[inline]
    pub(crate) fn clear_did_change_repeat(&mut self) {
        self.did_change_repeat = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauss_table_unity_dc_gain_and_no_overflow() {
        for i in 0..256usize {
            let g0 = GAUSS_TABLE[0x0FF - i];
            let g1 = GAUSS_TABLE[0x1FF - i];
            let g2 = GAUSS_TABLE[0x100 + i];
            let g3 = GAUSS_TABLE[i];
            let sum = g0 + g1 + g2 + g3;
            assert!(
                (0x7F7F..=0x7F81).contains(&sum),
                "gauss sum at i={i:#x} was {sum:#x}"
            );

            // Verify zero overflow for all extreme full-scale i16::MIN/MAX combinations.
            let extremes = [i16::MIN as i32, i16::MAX as i32];
            for &s0 in &extremes {
                for &s1 in &extremes {
                    for &s2 in &extremes {
                        for &s3 in &extremes {
                            let acc = g0
                                .checked_mul(s0)
                                .and_then(|v| v.checked_add(g1.checked_mul(s1)?))
                                .and_then(|v| v.checked_add(g2.checked_mul(s2)?))
                                .and_then(|v| v.checked_add(g3.checked_mul(s3)?))
                                .expect("gaussian accumulation must not overflow i32");
                            let out = acc >> 15;
                            assert!(
                                (i16::MIN as i32..=i16::MAX as i32).contains(&out),
                                "gaussian output {out} out of i16 range at i={i:#x}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn gauss_impulse_response_is_smooth_and_symmetric() {
        // Place a single impulse at sample index 10 in a silent buffer and sweep frac across 4 samples.
        let mut buf = [0i16; 16];
        buf[10] = 16384;
        let eval = |pos_q12: usize| -> i32 {
            let idx = pos_q12 >> 12;
            let frac = (pos_q12 >> 4) & 0xFF;
            let s0 = buf[idx] as i32;
            let s1 = buf[idx + 1] as i32;
            let s2 = buf[idx + 2] as i32;
            let s3 = buf[idx + 3] as i32;
            (GAUSS_TABLE[0x0FF - frac] * s0
                + GAUSS_TABLE[0x1FF - frac] * s1
                + GAUSS_TABLE[0x100 + frac] * s2
                + GAUSS_TABLE[frac] * s3)
                >> 15
        };

        // Peak occurs when the impulse sits on tap s1 (idx = 9, frac = 0) with weight gauss[0x1FF].
        let peak = eval(9 << 12);
        assert_eq!(peak, (16384 * GAUSS_TABLE[0x1FF]) >> 15);
        assert!(peak > 11_000);

        // Exact half-sample symmetry around the center between (8, frac=255) and (9, frac=0),
        // covering both the inner bell (s2 vs s1) and outer tails (s3 vs s0).
        for step in 0..256usize {
            let inner_l = eval((8 << 12) + ((255 - step) << 4));
            let inner_r = eval((9 << 12) + (step << 4));
            assert_eq!(inner_l, inner_r, "inner symmetry mismatch at step={step}");
            assert!(inner_l <= peak);

            let outer_l = eval((7 << 12) + ((255 - step) << 4));
            let outer_r = eval((10 << 12) + (step << 4));
            assert_eq!(outer_l, outer_r, "outer symmetry mismatch at step={step}");
            assert!(outer_l <= inner_l);
        }
    }

    #[test]
    fn cxd2922q_gauss_per_term_shift_wrap_and_saturate() {
        // Verify hardware-capture Gaussian MAC: full-precision 4-tap sum, >> 15, clamp16.
        let mut reader = SampleReader::new(0x80000);
        reader.set_destination_sampling_rate(44_100);
        reader.set_pitch(44_100, 0x1000);
        reader.buffer[0] = -32768;
        reader.buffer[1] = -32768;
        reader.buffer[2] = -32768;
        reader.buffer[3] = -32768;
        reader.src_sample_idx = 0x0080; // gauss_idx = 8
        let ram = vec![0u8; 0x80000];
        let mut cache = SpuSampleCache::new(0x80000);
        let mut irq = SpuIrqWatcher::new();
        let got = reader.get_sample(None, &ram, &mut cache, &mut irq);

        let gi = 8usize;
        let s = -32768i32;
        let expected = clamp16(
            (GAUSS_TABLE[0x0FF - gi] * s
                + GAUSS_TABLE[0x1FF - gi] * s
                + GAUSS_TABLE[0x100 + gi] * s
                + GAUSS_TABLE[gi] * s)
                >> 15,
        );
        assert_eq!(got, expected);
    }
}
