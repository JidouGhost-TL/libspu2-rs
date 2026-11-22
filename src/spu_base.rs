// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! Core SPU/SPU2 synthesis engine (`Iop::CSpuBase` port).

use crate::inspect::EnvelopePhase;
use crate::registers::{
    CONTROL_DMA_READ, CONTROL_DMA_STOP, CONTROL_ENABLE, CONTROL_IRQ, CONTROL_REVERB,
    TRANSFER_MODE_BLOCK_CORE0IN, TRANSFER_MODE_BLOCK_CORE1IN, TRANSFER_MODE_VOICE,
};

pub const MAX_CHANNEL: usize = 24;
pub const REVERB_PARAM_COUNT: usize = 32;
pub const BUFFER_SAMPLES: usize = 28;
pub const CONTROL_DMA: u16 = 0x0030;

const INIT_SAMPLE_RATE: u32 = 44_100;
const PITCH_BASE: u32 = 0x1000;
const TIME_SCALE: u32 = 0x1000;
const RESET_IRQ_ADDR: u32 = !0u32;
const MAX_ADSR_VOLUME: u32 = 0x7FFF_FFFF;

const CORE0_SIN_LEFT: u32 = 0x000;
const CORE1_SIN_LEFT: u32 = 0x800;
const CORE1_SIN_RIGHT: u32 = 0xA00;
const CORE0_OUTPUT_SIZE: u32 = 0x200;
const SOUND_INPUT_DATA_CORE0_BASE: u32 = 0x2000;
const SOUND_INPUT_DATA_CORE1_BASE: u32 = 0x2400;
const SOUND_INPUT_DATA_SIZE: usize = 0x400;
const SOUND_INPUT_DATA_SAMPLES: u32 = 0x100;

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
    pub const fn from_index(idx: usize) -> Option<Self> {
        if idx < REVERB_PARAM_COUNT {
            Some(Self::ALL[idx])
        } else {
            None
        }
    }

    #[inline]
    pub const fn is_address(self) -> bool {
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

/// Compile-time 256-entry ADSR log delta table (indices 0..160 populated, 160..256 zero).
const ADSR_LOG_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut value = 3u32;
    let mut column_increment = 1u32;
    let mut column = 0u32;
    let mut i = 32usize;
    while i < 160 {
        if value < 0x3FFF_FFFF {
            value += column_increment;
            column += 1;
            if column == 5 {
                column = 1;
                column_increment *= 2;
            }
        } else {
            value = 0x3FFF_FFFF;
        }
        table[i] = value;
        i += 1;
    }
    table
};

#[inline]
const fn get_adsr_delta(index: u32) -> u32 {
    ADSR_LOG_TABLE[(index.wrapping_add(32) & 0xFF) as usize]
}

const LINEAR_INCREASE_SWEEP_DELTAS: [u32; 0x80] = [
    0x3A00_0000, 0x3000_0000, 0x2800_0000, 0x2000_0000, 0x1D00_0000, 0x1800_0000, 0x1400_0000,
    0x1000_0000, 0x0E80_0000, 0x0C00_0000, 0x0A00_0000, 0x0800_0000, 0x0740_0000, 0x0600_0000,
    0x0500_0000, 0x0400_0000, 0x03A0_0000, 0x0300_0000, 0x0280_0000, 0x0200_0000, 0x01D0_0000,
    0x0180_0000, 0x0140_0000, 0x0100_0000, 0x00E8_0000, 0x00C0_0000, 0x00A0_0000, 0x0080_0000,
    0x0074_0000, 0x0060_0000, 0x0050_0000, 0x0040_0000, 0x003A_0000, 0x0030_0000, 0x0028_0000,
    0x0020_0000, 0x001D_0000, 0x0018_0000, 0x0014_0000, 0x0010_0000, 0x000E_8000, 0x000C_0000,
    0x000A_0000, 0x0008_0000, 0x0007_4000, 0x0006_0000, 0x0005_0000, 0x0004_0000, 0x0003_A000,
    0x0003_0000, 0x0002_8000, 0x0002_0000, 0x0001_D000, 0x0001_8000, 0x0001_4000, 0x0001_0000,
    0x0000_E800, 0x0000_C000, 0x0000_A000, 0x0000_8000, 0x0000_7400, 0x0000_6000, 0x0000_5000,
    0x0000_4000, 0x0000_3A00, 0x0000_3000, 0x0000_2800, 0x0000_2000, 0x0000_1D00, 0x0000_1800,
    0x0000_1400, 0x0000_1000, 0x0000_0E80, 0x0000_0C00, 0x0000_0A00, 0x0000_0800, 0x0000_0740,
    0x0000_0600, 0x0000_0500, 0x0000_0400, 0x0000_03A0, 0x0000_0300, 0x0000_0280, 0x0000_0200,
    0x0000_01D0, 0x0000_0180, 0x0000_0140, 0x0000_0100, 0x0000_00E8, 0x0000_00C0, 0x0000_00A0,
    0x0000_0080, 0x0000_0074, 0x0000_0060, 0x0000_0050, 0x0000_0040, 0x0000_003A, 0x0000_0030,
    0x0000_0028, 0x0000_0020, 0x0000_001D, 0x0000_0018, 0x0000_0014, 0x0000_0010, 0x0000_000E,
    0x0000_000C, 0x0000_000A, 0x0000_0008, 0x0000_0007, 0x0000_0006, 0x0000_0005, 0x0000_0004,
    0x0000_0003, 0x0000_0003, 0x0000_0002, 0x0000_0002, 0x0000_0001, 0x0000_0001, 0x0000_0001,
    0x0000_0001, 0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000,
    0x0000_0000, 0x0000_0000,
];

const LINEAR_DECREASE_SWEEP_DELTAS: [u32; 0x80] = [
    0x4800_0000, 0x4000_0000, 0x3800_0000, 0x3000_0000, 0x2400_0000, 0x2000_0000, 0x1C00_0000,
    0x1800_0000, 0x1200_0000, 0x1000_0000, 0x0E00_0000, 0x0C00_0000, 0x0900_0000, 0x0800_0000,
    0x0700_0000, 0x0600_0000, 0x0480_0000, 0x0400_0000, 0x0380_0000, 0x0300_0000, 0x0240_0000,
    0x0200_0000, 0x01C0_0000, 0x0180_0000, 0x0120_0000, 0x0100_0000, 0x00E0_0000, 0x00C0_0000,
    0x0090_0000, 0x0080_0000, 0x0070_0000, 0x0060_0000, 0x0048_0000, 0x0040_0000, 0x0038_0000,
    0x0030_0000, 0x0024_0000, 0x0020_0000, 0x001C_0000, 0x0018_0000, 0x0012_0000, 0x0010_0000,
    0x000E_0000, 0x000C_0000, 0x0009_0000, 0x0008_0000, 0x0007_0000, 0x0006_0000, 0x0004_8000,
    0x0004_0000, 0x0003_8000, 0x0003_0000, 0x0002_4000, 0x0002_0000, 0x0001_C000, 0x0001_8000,
    0x0001_2000, 0x0001_0000, 0x0000_E000, 0x0000_C000, 0x0000_9000, 0x0000_8000, 0x0000_7000,
    0x0000_6000, 0x0000_4800, 0x0000_4000, 0x0000_3800, 0x0000_3000, 0x0000_2400, 0x0000_2000,
    0x0000_1C00, 0x0000_1800, 0x0000_1200, 0x0000_1000, 0x0000_0E00, 0x0000_0C00, 0x0000_0900,
    0x0000_0800, 0x0000_0700, 0x0000_0600, 0x0000_0480, 0x0000_0400, 0x0000_0380, 0x0000_0300,
    0x0000_0240, 0x0000_0200, 0x0000_01C0, 0x0000_0180, 0x0000_0120, 0x0000_0100, 0x0000_00E0,
    0x0000_00C0, 0x0000_0090, 0x0000_0080, 0x0000_0070, 0x0000_0060, 0x0000_0048, 0x0000_0040,
    0x0000_0038, 0x0000_0030, 0x0000_0024, 0x0000_0020, 0x0000_001C, 0x0000_0018, 0x0000_0012,
    0x0000_0010, 0x0000_000E, 0x0000_000C, 0x0000_0009, 0x0000_0008, 0x0000_0007, 0x0000_0006,
    0x0000_0004, 0x0000_0004, 0x0000_0003, 0x0000_0003, 0x0000_0002, 0x0000_0002, 0x0000_0001,
    0x0000_0001, 0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000,
    0x0000_0000, 0x0000_0000,
];

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

const CACHE_SLOTS: usize = 1024;
const CACHE_SLOT_MASK: usize = CACHE_SLOTS - 1;

#[derive(Clone, Copy)]
struct CacheKey {
    address: u32,
    s1: i32,
    s2: i32,
}

#[derive(Clone, Copy)]
struct CacheSlot {
    valid: bool,
    address: u32,
    in_s1: i32,
    in_s2: i32,
    out_s1: i32,
    out_s2: i32,
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
pub struct SpuSampleCache {
    ram_size: u32,
    slots: Box<[CacheSlot]>,
}

impl SpuSampleCache {
    pub fn new(ram_size: u32) -> Self {
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
    fn register_item(
        &mut self,
        key: CacheKey,
        samples: [i16; BUFFER_SAMPLES],
        out_s1: i32,
        out_s2: i32,
    ) {
        let slot = &mut self.slots[Self::slot_index(key.address)];
        slot.valid = true;
        slot.address = key.address;
        slot.in_s1 = key.s1;
        slot.in_s2 = key.s2;
        slot.out_s1 = out_s1;
        slot.out_s2 = out_s2;
        slot.samples = samples;
    }

    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            slot.valid = false;
        }
    }

    /// Invalidates all cached blocks that overlap `[address, address + size)`,
    /// including unaligned blocks up to 15 bytes before `address` and wrap-around
    /// across `ram_size`.
    pub fn clear_range(&mut self, address: u32, size: u32) {
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
        if let Some(wrap_end) = addr.checked_add(size).and_then(|e| e.checked_sub(self.ram_size)) {
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

/// Shared per-core IRQ address watcher.
pub struct SpuIrqWatcher {
    irq_addr: [u32; 2],
    irq_pending: [bool; 2],
}

impl SpuIrqWatcher {
    pub fn new() -> Self {
        Self {
            irq_addr: [RESET_IRQ_ADDR; 2],
            irq_pending: [false; 2],
        }
    }

    pub fn reset(&mut self) {
        self.irq_addr = [RESET_IRQ_ADDR; 2];
        self.irq_pending = [false; 2];
    }

    pub fn set_irq_address(&mut self, core: usize, address: u32) {
        self.irq_addr[core] = address;
    }

    #[inline]
    pub fn check_irq(&mut self, address: u32) {
        if address == self.irq_addr[0] {
            self.irq_pending[0] = true;
        }
        if address == self.irq_addr[1] {
            self.irq_pending[1] = true;
        }
    }

    pub fn clear_irq_pending(&mut self, core: usize) {
        self.irq_pending[core] = false;
    }

    pub fn has_pending_irq(&self, core: usize) -> bool {
        self.irq_pending[core]
    }
}

/// Packed `VP_ADSR1` register.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AdsrLevel(pub u16);

impl AdsrLevel {
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub const fn sustain_level(self) -> u32 {
        (self.0 & 0x000F) as u32
    }
    #[inline]
    pub const fn decay_rate(self) -> u32 {
        ((self.0 >> 4) & 0x000F) as u32
    }
    #[inline]
    pub const fn attack_rate(self) -> u32 {
        ((self.0 >> 8) & 0x007F) as u32
    }
    #[inline]
    pub const fn attack_exponential(self) -> bool {
        (self.0 & 0x8000) != 0
    }
}

/// Packed `VP_ADSR2` register.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AdsrRate(pub u16);

impl AdsrRate {
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub const fn release_rate(self) -> u32 {
        (self.0 & 0x001F) as u32
    }
    #[inline]
    pub const fn release_exponential(self) -> bool {
        (self.0 & 0x0020) != 0
    }
    #[inline]
    pub const fn sustain_rate(self) -> u32 {
        ((self.0 >> 6) & 0x007F) as u32
    }
    #[inline]
    pub const fn sustain_decrease(self) -> bool {
        (self.0 & 0x4000) != 0
    }
    #[inline]
    pub const fn sustain_exponential(self) -> bool {
        (self.0 & 0x8000) != 0
    }
}

/// Packed `VP_VOLL` / `VP_VOLR` register.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ChannelVolume(pub u16);

impl ChannelVolume {
    #[inline]
    pub const fn raw(self) -> u16 {
        self.0
    }
    #[inline]
    pub const fn is_sweep(self) -> bool {
        (self.0 & 0x8000) != 0
    }
    #[inline]
    pub const fn fixed_volume(self) -> u16 {
        self.0 & 0x3FFF
    }
    #[inline]
    pub const fn fixed_phase(self) -> bool {
        (self.0 & 0x4000) != 0
    }
    #[inline]
    pub const fn sweep_volume(self) -> usize {
        (self.0 & 0x007F) as usize
    }
    #[inline]
    pub const fn sweep_decrease(self) -> bool {
        (self.0 & 0x2000) != 0
    }
    #[inline]
    pub const fn sweep_slope(self) -> bool {
        (self.0 & 0x4000) != 0
    }
}

/// Per-voice register and synthesis state.
#[derive(Debug, Default, Clone, Copy)]
pub struct Channel {
    pub volume_left: ChannelVolume,
    pub volume_right: ChannelVolume,
    pub volume_left_abs: i32,
    pub volume_right_abs: i32,
    pub pitch: u16,
    pub address: u32,
    pub adsr_level: AdsrLevel,
    pub adsr_rate: AdsrRate,
    pub adsr_volume: u32,
    pub repeat: u32,
    pub repeat_set: bool,
    pub status: EnvelopePhase,
    pub current: u32,
}

impl Channel {
    #[inline]
    pub fn set_volume_left(&mut self, value: u16) {
        self.volume_left = ChannelVolume(value);
        if !self.volume_left.is_sweep() {
            self.volume_left_abs = (self.volume_left.fixed_volume() as i32) << 17;
        }
    }

    #[inline]
    pub fn set_volume_right(&mut self, value: u16) {
        self.volume_right = ChannelVolume(value);
        if !self.volume_right.is_sweep() {
            self.volume_right_abs = (self.volume_right.fixed_volume() as i32) << 17;
        }
    }
}

#[derive(Clone)]
struct SampleReader {
    ram_size: u32,
    src_sample_idx: u32,
    sample_step: u32,
    src_sampling_rate: u32,
    dst_sampling_rate: u32,
    next_sample_addr: u32,
    repeat_addr: u32,
    buffer: [i16; BUFFER_SAMPLES * 2],
    s1: i32,
    s2: i32,
    done: bool,
    next_valid: bool,
    end_flag: bool,
    did_change_repeat: bool,
}

impl SampleReader {
    fn new(ram_size: u32) -> Self {
        debug_assert!(ram_size.is_power_of_two());
        Self {
            ram_size,
            src_sample_idx: 0,
            sample_step: 0,
            src_sampling_rate: 0,
            dst_sampling_rate: 0,
            next_sample_addr: 0,
            repeat_addr: 0,
            buffer: [0; BUFFER_SAMPLES * 2],
            s1: 0,
            s2: 0,
            done: false,
            next_valid: false,
            end_flag: false,
            did_change_repeat: false,
        }
    }

    fn reset(&mut self) {
        self.next_sample_addr = 0;
        self.repeat_addr = 0;
        self.buffer = [0; BUFFER_SAMPLES * 2];
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
    fn is_inert(&self) -> bool {
        self.sample_step == 0
            && (self.src_sample_idx / PITCH_BASE) < BUFFER_SAMPLES as u32
            && !self.done
            && !self.did_change_repeat
    }

    fn set_destination_sampling_rate(&mut self, sampling_rate: u32) {
        self.dst_sampling_rate = sampling_rate;
        self.update_sample_step();
    }

    fn set_params(&mut self, address: u32, repeat: u32) {
        let mask = self.ram_size - 1;
        self.src_sample_idx = 0;
        self.next_sample_addr = address & mask;
        self.repeat_addr = repeat & mask;
        self.s1 = 0;
        self.s2 = 0;
        self.next_valid = false;
        self.done = false;
        self.did_change_repeat = false;
    }

    fn set_params_read(
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

    fn set_params_no_read(&mut self, address: u32, repeat: u32) {
        self.set_params(address, repeat);
        self.buffer = [0; BUFFER_SAMPLES * 2];
    }

    fn set_pitch(&mut self, base_sampling_rate: u32, pitch: u16) {
        self.src_sampling_rate = base_sampling_rate.wrapping_mul(pitch as u32);
        self.update_sample_step();
    }

    fn update_sample_step(&mut self) {
        if let Some(step) = self.src_sampling_rate.checked_div(self.dst_sampling_rate) {
            self.sample_step = step;
        }
    }

    #[inline]
    fn get_sample(
        &mut self,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) -> i32 {
        let src_sample_idx = (self.src_sample_idx / PITCH_BASE) as usize;
        let src_sample_alpha = (self.src_sample_idx % PITCH_BASE) as i32;
        let pair = &self.buffer[src_sample_idx..src_sample_idx + 2];
        let current_sample = pair[0] as i32;
        let next_sample = pair[1] as i32;
        let pitch_base = PITCH_BASE as i32;
        let result_sample = (current_sample * (pitch_base - src_sample_alpha) / pitch_base)
            + (next_sample * src_sample_alpha / pitch_base);
        let mut next_idx = (self.src_sample_idx as u64) + (self.sample_step as u64);
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
        result_sample
    }

    fn advance_buffer(
        &mut self,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        if self.next_valid {
            self.buffer.copy_within(BUFFER_SAMPLES..(BUFFER_SAMPLES * 2), 0);
            self.unpack_into_half(BUFFER_SAMPLES, ram, cache, irq_watcher);
        } else {
            self.unpack_into_half(0, ram, cache, irq_watcher);
            self.unpack_into_half(BUFFER_SAMPLES, ram, cache, irq_watcher);
            self.next_valid = true;
        }
    }

    fn unpack_into_half(
        &mut self,
        dst_offset: usize,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        let mask = (self.ram_size - 1) as usize;
        let base_addr = (self.next_sample_addr as usize) & mask;

        irq_watcher.check_irq(self.next_sample_addr);

        let header = ram[base_addr];
        let flags = ram[(base_addr + 1) & mask];
        let shift_factor = (header & 0x0F) as u32;
        let predict_number = (header >> 4) as usize;

        let mut cache_key = CacheKey {
            address: self.next_sample_addr,
            s1: self.s1,
            s2: self.s2,
        };
        if predict_number == 0 {
            cache_key.s1 = 0;
            cache_key.s2 = 0;
        }

        let dst = &mut self.buffer[dst_offset..dst_offset + BUFFER_SAMPLES];
        if let Some(item) = cache.get_item(cache_key) {
            dst.copy_from_slice(&item.samples);
            self.s1 = item.out_s1;
            self.s2 = item.out_s2;
        } else {
            let k0 = PREDICTOR_TABLE[predict_number][0];
            let k1 = PREDICTOR_TABLE[predict_number][1];
            let mut s1 = self.s1;
            let mut s2 = self.s2;

            let mut decode_pair = |sample_byte: u8, out: &mut [i16]| {
                let first_sample = (((sample_byte & 0x0F) as u16) << 12) as i16;
                let second_sample = (((sample_byte & 0xF0) as u16) << 8) as i16;

                let w0 = (first_sample as i32) >> shift_factor;
                let mut v0 = w0.wrapping_mul(64);
                v0 = v0.wrapping_add(s1.wrapping_mul(k0) / 64);
                v0 = v0.wrapping_add(s2.wrapping_mul(k1) / 64);
                s2 = s1;
                s1 = v0;
                out[0] = (v0.wrapping_add(32) / 64).clamp(i16::MIN as i32, i16::MAX as i32) as i16;

                let w1 = (second_sample as i32) >> shift_factor;
                let mut v1 = w1.wrapping_mul(64);
                v1 = v1.wrapping_add(s1.wrapping_mul(k0) / 64);
                v1 = v1.wrapping_add(s2.wrapping_mul(k1) / 64);
                s2 = s1;
                s1 = v1;
                out[1] = (v1.wrapping_add(32) / 64).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
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
            let mut cached = [0i16; BUFFER_SAMPLES];
            cached.copy_from_slice(dst);
            cache.register_item(cache_key, cached, s1, s2);
        }

        if (flags & 0x04) != 0 {
            self.repeat_addr = self.next_sample_addr;
            self.did_change_repeat = true;
        }

        self.next_sample_addr = self.next_sample_addr.wrapping_add(0x10) & (self.ram_size - 1);

        if (flags & 0x01) != 0 {
            self.end_flag = true;
            self.next_sample_addr = self.repeat_addr;
            if flags != 0x03 {
                self.done = true;
            }
        }
    }

    fn repeat(&self) -> u32 {
        self.repeat_addr
    }

    fn set_repeat(&mut self, repeat_addr: u32) {
        self.repeat_addr = repeat_addr & (self.ram_size - 1);
    }

    fn current(&self) -> u32 {
        let intra_sample_idx = ((self.src_sample_idx / PITCH_BASE) / 2).min(0x0E);
        self.next_sample_addr.wrapping_add(intra_sample_idx)
    }

    fn is_done(&self) -> bool {
        self.done
    }

    fn clear_is_done(&mut self) {
        self.done = false;
    }

    fn end_flag(&self) -> bool {
        self.end_flag
    }

    fn clear_end_flag(&mut self) {
        self.end_flag = false;
    }

    fn did_change_repeat(&self) -> bool {
        self.did_change_repeat
    }

    fn clear_did_change_repeat(&mut self) {
        self.did_change_repeat = false;
    }
}

struct BlockSampleReader {
    base_sampling_rate: u32,
    dst_sampling_rate: u32,
    src_sample_idx: u32,
    sample_step: u32,
    block_buffer: [u8; SOUND_INPUT_DATA_SIZE],
    spdif_bypass: bool,
}

impl BlockSampleReader {
    fn new() -> Self {
        Self {
            base_sampling_rate: INIT_SAMPLE_RATE,
            dst_sampling_rate: 0,
            src_sample_idx: SOUND_INPUT_DATA_SAMPLES * TIME_SCALE,
            sample_step: 0,
            block_buffer: [0; SOUND_INPUT_DATA_SIZE],
            spdif_bypass: false,
        }
    }

    fn reset(&mut self) {
        self.base_sampling_rate = INIT_SAMPLE_RATE;
        self.dst_sampling_rate = 0;
        self.src_sample_idx = SOUND_INPUT_DATA_SAMPLES * TIME_SCALE;
        self.sample_step = 0;
        self.block_buffer.fill(0);
        self.spdif_bypass = false;
    }

    fn set_base_sampling_rate(&mut self, rate: u32) {
        self.base_sampling_rate = rate;
        self.update_sample_step();
    }

    fn set_destination_sampling_rate(&mut self, rate: u32) {
        self.dst_sampling_rate = rate;
        self.update_sample_step();
    }

    fn spdif_bypass(&self) -> bool {
        self.spdif_bypass
    }

    fn set_spdif_bypass(&mut self, bypass: bool) {
        self.spdif_bypass = bypass;
    }

    #[inline]
    fn can_read_samples(&self) -> bool {
        let sample_idx = self.src_sample_idx / TIME_SCALE;
        if self.spdif_bypass {
            sample_idx < (SOUND_INPUT_DATA_SAMPLES / 2)
        } else {
            sample_idx < SOUND_INPUT_DATA_SAMPLES
        }
    }

    fn fill_block(&mut self, block: &[u8]) {
        self.block_buffer.copy_from_slice(&block[..SOUND_INPUT_DATA_SIZE]);
        self.src_sample_idx = 0;
    }

    fn get_samples(&mut self) -> [i32; 2] {
        let sample_idx = (self.src_sample_idx / TIME_SCALE) as usize;
        let out = if self.spdif_bypass {
            let base = sample_idx * 4;
            let l = i16::from_le_bytes([self.block_buffer[base], self.block_buffer[base + 1]]);
            let r = i16::from_le_bytes([self.block_buffer[base + 2], self.block_buffer[base + 3]]);
            [l as i32, r as i32]
        } else {
            let l_off = sample_idx * 2;
            let r_off = (SOUND_INPUT_DATA_SIZE / 2) + sample_idx * 2;
            let l = i16::from_le_bytes([self.block_buffer[l_off], self.block_buffer[l_off + 1]]);
            let r = i16::from_le_bytes([self.block_buffer[r_off], self.block_buffer[r_off + 1]]);
            [l as i32, r as i32]
        };
        self.src_sample_idx = self.src_sample_idx.wrapping_add(self.sample_step);
        out
    }

    fn update_sample_step(&mut self) {
        if let Some(step) = self
            .base_sampling_rate
            .wrapping_mul(TIME_SCALE)
            .checked_div(self.dst_sampling_rate)
        {
            self.sample_step = step;
        }
    }
}

/// Single SPU/SPU2 synthesis core (24 voices, reverb, ADSR, DMA/FIFO transfers).
pub struct SpuBase {
    ram_size: u32,
    spu_number: usize,
    base_sampling_rate: u32,

    irq_addr: u32,
    irq_pending: bool,
    transfer_mode: u16,
    transfer_addr: u32,
    core0_output_offset: u32,
    channel_reverb: u32,
    reverb_work_addr_start: u32,
    reverb_work_addr_end: u32,
    reverb_curr_addr: u32,
    ctrl: u16,
    reverb_ticks: i32,
    reverb: [u32; REVERB_PARAM_COUNT],
    reverb_coefs: [f32; REVERB_PARAM_COUNT],
    reverb_cached_out: [i32; 2],
    reverb_cached_valid: bool,

    channels: [Channel; MAX_CHANNEL],
    readers: [SampleReader; MAX_CHANNEL],

    block_reader: BlockSampleReader,
    sound_input_data_addr: u32,
    block_write_ptr: u32,

    pub input_vol_l: i32,
    pub input_vol_r: i32,
    pub ext_input_vol_l: i32,
    pub ext_input_vol_r: i32,
}

impl SpuBase {
    pub fn new(spu_number: usize, ram_size: u32) -> Self {
        assert!(ram_size.is_power_of_two() && ram_size >= 0x4000);
        Self {
            ram_size,
            spu_number,
            base_sampling_rate: INIT_SAMPLE_RATE,
            irq_addr: RESET_IRQ_ADDR,
            irq_pending: false,
            transfer_mode: 0,
            transfer_addr: 0,
            core0_output_offset: 0,
            channel_reverb: 0,
            reverb_work_addr_start: 0,
            reverb_work_addr_end: 0x80000,
            reverb_curr_addr: 0,
            ctrl: 0,
            reverb_ticks: 0,
            reverb: [0; REVERB_PARAM_COUNT],
            reverb_coefs: [0.0; REVERB_PARAM_COUNT],
            reverb_cached_out: [0, 0],
            reverb_cached_valid: false,
            channels: [Channel::default(); MAX_CHANNEL],
            readers: std::array::from_fn(|_| SampleReader::new(ram_size)),
            block_reader: BlockSampleReader::new(),
            sound_input_data_addr: if spu_number == 0 {
                SOUND_INPUT_DATA_CORE0_BASE
            } else {
                SOUND_INPUT_DATA_CORE1_BASE
            },
            block_write_ptr: 0,
            input_vol_l: 0x7FFF,
            input_vol_r: 0x7FFF,
            ext_input_vol_l: 0x7FFF,
            ext_input_vol_r: 0x7FFF,
        }
    }

    pub fn reset(&mut self) {
        self.ctrl = 0;
        self.channel_reverb = 0;
        self.reverb_ticks = 0;
        self.irq_addr = RESET_IRQ_ADDR;
        self.irq_pending = false;
        self.transfer_mode = 0;
        self.transfer_addr = 0;
        self.core0_output_offset = 0;
        self.reverb_curr_addr = 0;
        self.reverb_work_addr_start = 0;
        self.reverb_work_addr_end = 0x80000;
        self.base_sampling_rate = INIT_SAMPLE_RATE;

        self.input_vol_l = 0x7FFF;
        self.input_vol_r = 0x7FFF;
        self.ext_input_vol_l = 0x7FFF;
        self.ext_input_vol_r = 0x7FFF;

        self.channels = [Channel::default(); MAX_CHANNEL];
        self.reverb = [0; REVERB_PARAM_COUNT];
        self.reverb_coefs = [0.0; REVERB_PARAM_COUNT];
        self.reverb_cached_out = [0, 0];
        self.reverb_cached_valid = false;

        for reader in &mut self.readers {
            reader.reset();
        }

        self.block_reader.reset();
        self.sound_input_data_addr = if self.spu_number == 0 {
            SOUND_INPUT_DATA_CORE0_BASE
        } else {
            SOUND_INPUT_DATA_CORE1_BASE
        };
        self.block_write_ptr = 0;
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        (self.ctrl & CONTROL_ENABLE) != 0
    }

    /// Returns `true` if this core has no active/stepping voices, no active
    /// reverb, no queued AutoDMA block input, and no IRQ enabled.
    pub fn is_completely_idle(&self) -> bool {
        if (self.ctrl & (CONTROL_REVERB | CONTROL_IRQ)) != 0 {
            return false;
        }
        if self.block_reader.can_read_samples()
            || self.block_write_ptr as usize == SOUND_INPUT_DATA_SIZE
        {
            return false;
        }
        for i in 0..MAX_CHANNEL {
            let ch = &self.channels[i];
            if ch.status != EnvelopePhase::Stopped
                || (ch.adsr_volume >> 16) != 0
                || !self.readers[i].is_inert()
                || ch.volume_left.is_sweep()
                || ch.volume_right.is_sweep()
            {
                return false;
            }
        }
        true
    }

    pub fn control(&self) -> u16 {
        self.ctrl
    }

    pub fn set_control(&mut self, value: u16, irq_watcher: &mut SpuIrqWatcher) {
        self.ctrl = value;
        if (self.ctrl & CONTROL_IRQ) == 0 {
            self.clear_irq_pending();
            irq_watcher.clear_irq_pending(self.spu_number);
        }
    }

    pub fn set_base_sampling_rate(&mut self, sampling_rate: u32) {
        self.base_sampling_rate = sampling_rate;
        self.block_reader.set_base_sampling_rate(sampling_rate);
    }

    pub fn set_input_bypass(&mut self, input_bypass: bool) {
        self.block_reader.set_spdif_bypass(input_bypass);
    }

    pub fn set_destination_sampling_rate(&mut self, sampling_rate: u32) {
        self.block_reader
            .set_destination_sampling_rate(sampling_rate);
        for reader in &mut self.readers {
            reader.set_destination_sampling_rate(sampling_rate);
        }
    }

    pub fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    pub fn clear_irq_pending(&mut self) {
        self.irq_pending = false;
    }

    pub fn irq_address(&self) -> u32 {
        self.irq_addr
    }

    pub fn set_irq_address(&mut self, value: u32, irq_watcher: &mut SpuIrqWatcher) {
        self.irq_addr = value & (self.ram_size - 1);
        irq_watcher.set_irq_address(self.spu_number, self.irq_addr);
    }

    pub fn transfer_mode(&self) -> u16 {
        self.transfer_mode
    }

    pub fn set_transfer_mode(&mut self, transfer_mode: u16) {
        self.transfer_mode = transfer_mode;
    }

    pub fn transfer_address(&self) -> u32 {
        self.transfer_addr
    }

    pub fn set_transfer_address(&mut self, value: u32) {
        self.transfer_addr = value & (self.ram_size - 1);
    }

    pub fn channel_reverb(&self) -> u32 {
        self.channel_reverb
    }

    pub fn set_channel_reverb_lo(&mut self, value: u16) {
        self.channel_reverb = (self.channel_reverb & 0xFFFF_0000) | (value as u32);
    }

    pub fn set_channel_reverb_hi(&mut self, value: u16) {
        self.channel_reverb = (self.channel_reverb & 0x0000_FFFF) | ((value as u32) << 16);
    }

    #[inline]
    pub(crate) fn reverb_param(&self, param: ReverbParam) -> u32 {
        self.reverb[param as usize]
    }

    pub(crate) fn set_reverb_param(&mut self, param: ReverbParam, value: u32) {
        let idx = param as usize;
        self.reverb[idx] = value;
        self.reverb_coefs[idx] = ((value as i16) as f32) / (0x8000 as f32);
        self.reverb_cached_valid = false;
    }

    pub fn end_flags(&self) -> u32 {
        let mut result = 0u32;
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.end_flag() {
                result |= 1 << i;
            }
        }
        result
    }

    pub fn clear_end_flags(&mut self) {
        for reader in &mut self.readers {
            reader.clear_end_flag();
        }
    }

    pub fn channel(&self, channel_number: usize) -> &Channel {
        &self.channels[channel_number]
    }

    pub fn channel_mut(&mut self, channel_number: usize) -> &mut Channel {
        &mut self.channels[channel_number]
    }

    pub fn on_channel_pitch_changed(&mut self, channel_number: usize) {
        let pitch = self.channels[channel_number].pitch;
        self.readers[channel_number].set_pitch(self.base_sampling_rate, pitch);
    }

    pub fn send_key_on(&mut self, channels: u32) {
        let mut mask = channels & ((1u32 << MAX_CHANNEL) - 1);
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let ch = &mut self.channels[i];
            let attack_rate = ch.adsr_level.attack_rate();
            let preloaded = get_adsr_delta((attack_rate ^ 0x7F).wrapping_sub(0x10));
            ch.status = EnvelopePhase::KeyOn;
            ch.repeat_set = false;
            ch.current = ch.address;
            ch.adsr_volume = preloaded;
        }
    }

    pub fn send_key_off(&mut self, channels: u32) {
        let mut mask = channels & ((1u32 << MAX_CHANNEL) - 1);
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let ch = &mut self.channels[i];
            match ch.status {
                EnvelopePhase::Stopped => {}
                EnvelopePhase::KeyOn => {
                    ch.status = EnvelopePhase::Stopped;
                    let reader = &mut self.readers[i];
                    reader.set_params_no_read(ch.address, ch.repeat);
                    reader.clear_end_flag();
                    ch.current = reader.current();
                }
                _ => {
                    ch.status = EnvelopePhase::Release;
                }
            }
        }
    }

    pub fn reverb_work_address_start(&self) -> u32 {
        self.reverb_work_addr_start
    }

    pub fn set_reverb_work_address_start(&mut self, address: u32) {
        self.reverb_work_addr_start = address;
        self.reverb_curr_addr = address;
        self.reverb_cached_valid = false;
    }

    pub fn reverb_work_address_end(&self) -> u32 {
        self.reverb_work_addr_end
    }

    pub fn set_reverb_work_address_end(&mut self, address: u32) {
        self.reverb_work_addr_end = address;
        self.reverb_cached_valid = false;
    }

    pub fn write_word(&mut self, value: u16, ram: &mut [u8], cache: &mut SpuSampleCache) {
        let mask = self.ram_size - 1;
        self.transfer_addr &= mask;
        let bytes = value.to_le_bytes();
        let addr = self.transfer_addr as usize;
        ram[addr] = bytes[0];
        ram[(addr + 1) & (mask as usize)] = bytes[1];
        cache.clear_range(self.transfer_addr, 2);
        self.transfer_addr = self.transfer_addr.wrapping_add(2) & mask;
    }

    pub fn receive_dma(
        &mut self,
        buffer: &mut [u8],
        block_size: u32,
        mut block_amount: u32,
        ram: &mut [u8],
        cache: &mut SpuSampleCache,
    ) -> u32 {
        let mask = self.ram_size - 1;
        if self.transfer_mode == TRANSFER_MODE_VOICE {
            let dma_mode = self.ctrl & CONTROL_DMA;
            if dma_mode == CONTROL_DMA_STOP {
                return 0;
            }
            let bs = block_size as usize;
            let ram_len = self.ram_size as usize;
            if dma_mode == CONTROL_DMA_READ {
                block_amount = block_amount.min(0x10);
                let mut offset = 0usize;
                for _ in 0..block_amount {
                    let mut ta = self.transfer_addr as usize;
                    let mut copied = 0usize;
                    while copied < bs {
                        let chunk = (ram_len - ta).min(bs - copied);
                        buffer[offset + copied..offset + copied + chunk]
                            .copy_from_slice(&ram[ta..ta + chunk]);
                        ta = (ta + chunk) & (mask as usize);
                        copied += chunk;
                    }
                    self.transfer_addr = self.transfer_addr.wrapping_add(block_size) & mask;
                    offset += bs;
                }
                return block_amount;
            }
            block_amount = block_amount.min(0x100);
            let total_size = block_size.saturating_mul(block_amount);
            let tail = self.ram_size.saturating_sub(self.transfer_addr);
            if total_size <= tail {
                cache.clear_range(self.transfer_addr, total_size);
            } else {
                cache.clear_range(self.transfer_addr, tail);
                cache.clear_range(0, (total_size - tail).min(self.ram_size));
            }
            let mut offset = 0usize;
            for _ in 0..block_amount {
                let mut ta = self.transfer_addr as usize;
                let mut copied = 0usize;
                while copied < bs {
                    let chunk = (ram_len - ta).min(bs - copied);
                    ram[ta..ta + chunk]
                        .copy_from_slice(&buffer[offset + copied..offset + copied + chunk]);
                    ta = (ta + chunk) & (mask as usize);
                    copied += chunk;
                }
                self.transfer_addr = self.transfer_addr.wrapping_add(block_size) & mask;
                offset += bs;
            }
            block_amount
        } else if self.transfer_mode == TRANSFER_MODE_BLOCK_CORE0IN
            || self.transfer_mode == TRANSFER_MODE_BLOCK_CORE1IN
        {
            if block_size == 0 || self.block_write_ptr as usize > SOUND_INPUT_DATA_SIZE {
                return 0;
            }
            let available_bytes = (SOUND_INPUT_DATA_SIZE as u32) - self.block_write_ptr;
            let available_blocks = available_bytes / block_size;
            block_amount = block_amount.min(available_blocks);
            let total_bytes = (block_amount * block_size) as usize;
            let dst_addr = (self.sound_input_data_addr + self.block_write_ptr) as usize;
            ram[dst_addr..dst_addr + total_bytes].copy_from_slice(&buffer[..total_bytes]);
            self.block_write_ptr += total_bytes as u32;
            block_amount
        } else {
            0
        }
    }

    pub fn latch_irq(&mut self, irq_watcher: &mut SpuIrqWatcher) {
        if (self.ctrl & CONTROL_IRQ) != 0 && irq_watcher.has_pending_irq(self.spu_number) {
            self.irq_pending = true;
            irq_watcher.clear_irq_pending(self.spu_number);
        }
    }

    #[inline]
    pub fn mix_samples(input_sample: i32, volume_level: i32, output: &mut i16) {
        let scaled = input_sample.wrapping_mul(volume_level) / 0x7FFF;
        let result = scaled
            .wrapping_add(*output as i32)
            .clamp(i16::MIN as i32, i16::MAX as i32);
        *output = result as i16;
    }

    /// Renders `samples.len() / 2` stereo frames into `samples` (interleaved L, R).
    pub fn render(
        &mut self,
        samples: &mut [i16],
        ram: &mut [u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        debug_assert!((samples.len() & 1) == 0);
        let ticks = samples.len() / 2;
        if ticks == 0 {
            return;
        }

        let update_reverb = ((self.ctrl & CONTROL_REVERB) != 0)
            && (self.reverb_work_addr_start < self.reverb_work_addr_end);
        let irq_enabled = (self.ctrl & CONTROL_IRQ) != 0;
        self.reverb_cached_valid = false;

        // Build active-voice bitmask; sync inert voices once outside the hot loop.
        let mut active_mask = 0u32;
        for i in 0..MAX_CHANNEL {
            let ch = &mut self.channels[i];
            let reader = &mut self.readers[i];
            if ch.status == EnvelopePhase::Stopped
                && (ch.adsr_volume >> 16) == 0
                && reader.is_inert()
                && !ch.volume_left.is_sweep()
                && !ch.volume_right.is_sweep()
            {
                reader.set_repeat(ch.repeat);
                ch.current = reader.current();
                ch.volume_left_abs =
                    Self::compute_channel_volume(ch.volume_left, ch.volume_left_abs);
                ch.volume_right_abs =
                    Self::compute_channel_volume(ch.volume_right, ch.volume_right_abs);
            } else {
                active_mask |= 1u32 << i;
            }
        }

        let check_core0_irq = self.spu_number == 0
            && irq_enabled
            && (self.irq_addr < CORE0_OUTPUT_SIZE
                || (CORE1_SIN_LEFT..CORE1_SIN_LEFT + CORE0_OUTPUT_SIZE).contains(&self.irq_addr)
                || (CORE1_SIN_RIGHT..CORE1_SIN_RIGHT + CORE0_OUTPUT_SIZE).contains(&self.irq_addr));

        for frame in samples.chunks_exact_mut(2) {
            let mut out_l: i16 = 0;
            let mut out_r: i16 = 0;
            let mut reverb_sample = [0i16; 2];

            let mut mask = active_mask;
            while mask != 0 {
                let i = mask.trailing_zeros() as usize;
                mask &= mask - 1;

                let ch = &mut self.channels[i];
                let reader = &mut self.readers[i];

                if ch.status == EnvelopePhase::KeyOn {
                    reader.set_params_read(ch.address, ch.repeat, ram, cache, irq_watcher);
                    reader.clear_end_flag();
                    ch.status = EnvelopePhase::Attack;
                    ch.adsr_volume = 0;
                } else {
                    if reader.is_done() {
                        ch.status = EnvelopePhase::Stopped;
                        ch.adsr_volume = 0;
                        reader.clear_is_done();
                    }
                    if reader.did_change_repeat() && !ch.repeat_set {
                        ch.repeat = reader.repeat();
                        reader.clear_did_change_repeat();
                    }
                    reader.set_repeat(ch.repeat);
                }

                let read_sample = reader.get_sample(ram, cache, irq_watcher);
                ch.current = reader.current();

                Self::update_adsr(ch);
                let vol_l = Self::compute_channel_volume(ch.volume_left, ch.volume_left_abs);
                let vol_r = Self::compute_channel_volume(ch.volume_right, ch.volume_right_abs);
                ch.volume_left_abs = vol_l;
                ch.volume_right_abs = vol_r;

                if read_sample == 0 {
                    continue;
                }

                let adsr_top = (ch.adsr_volume >> 16) as i32;
                let max_adsr_top = (MAX_ADSR_VOLUME >> 16) as i32;
                let input_sample = read_sample.wrapping_mul(adsr_top) / max_adsr_top;

                if input_sample == 0 {
                    continue;
                }

                let volume_left = vol_l >> 16;
                let volume_right = vol_r >> 16;

                Self::mix_samples(input_sample, volume_left, &mut out_l);
                Self::mix_samples(input_sample, volume_right, &mut out_r);

                if update_reverb && ((self.channel_reverb & (1 << i)) != 0) {
                    Self::mix_samples(input_sample, volume_left, &mut reverb_sample[0]);
                    Self::mix_samples(input_sample, volume_right, &mut reverb_sample[1]);
                }
            }

            if !self.block_reader.can_read_samples()
                && (self.block_write_ptr as usize == SOUND_INPUT_DATA_SIZE)
            {
                let start = self.sound_input_data_addr as usize;
                self.block_reader
                    .fill_block(&ram[start..start + SOUND_INPUT_DATA_SIZE]);
                self.block_write_ptr = 0;
            }

            if self.block_reader.can_read_samples() {
                let block_samples = self.block_reader.get_samples();
                if self.spu_number == 0 && self.block_reader.spdif_bypass() {
                    Self::mix_samples(block_samples[0], 0x7FFF, &mut out_l);
                    Self::mix_samples(block_samples[1], 0x7FFF, &mut out_r);
                } else {
                    Self::mix_samples(block_samples[0], self.input_vol_l, &mut out_l);
                    Self::mix_samples(block_samples[1], self.input_vol_r, &mut out_r);
                }
            }

            if check_core0_irq {
                if self.irq_addr == (CORE0_SIN_LEFT + self.core0_output_offset)
                    || self.irq_addr == (CORE1_SIN_LEFT + self.core0_output_offset)
                    || self.irq_addr == (CORE1_SIN_RIGHT + self.core0_output_offset)
                {
                    self.irq_pending = true;
                }
                self.core0_output_offset =
                    self.core0_output_offset.wrapping_add(2) & (CORE0_OUTPUT_SIZE - 1);
            }

            if update_reverb {
                self.update_reverb(reverb_sample, &mut out_l, &mut out_r, ram);
            }

            frame[0] = out_l;
            frame[1] = out_r;
        }

        if self.spu_number == 0 && !check_core0_irq {
            self.core0_output_offset = self
                .core0_output_offset
                .wrapping_add((ticks as u32).wrapping_mul(2))
                & (CORE0_OUTPUT_SIZE - 1);
        }

        if irq_enabled && irq_watcher.has_pending_irq(self.spu_number) {
            self.irq_pending = true;
        }
        irq_watcher.clear_irq_pending(self.spu_number);
    }

    #[inline]
    fn compute_channel_volume(volume: ChannelVolume, current_volume: i32) -> i32 {
        if !volume.is_sweep() {
            let level = if volume.fixed_phase() {
                0x3FFF - volume.fixed_volume() as i32
            } else {
                volume.fixed_volume() as i32
            };
            level << 17
        } else {
            let mut volume_level = 0i32;
            if !volume.sweep_slope() {
                if volume.sweep_decrease() {
                    let delta = LINEAR_DECREASE_SWEEP_DELTAS[volume.sweep_volume()];
                    volume_level = current_volume.wrapping_sub(delta as i32);
                } else {
                    let delta = LINEAR_INCREASE_SWEEP_DELTAS[volume.sweep_volume()];
                    volume_level = current_volume.wrapping_add(delta as i32);
                }
            } else if volume.sweep_decrease() {
                let sweep_delta = (current_volume as i64 * volume.sweep_volume() as i64) / 0x7F;
                let base_volume = current_volume.max(1);
                let sweep_delta_clamped = sweep_delta.clamp(1, base_volume as i64) as i32;
                volume_level = base_volume.wrapping_sub(sweep_delta_clamped);
            }
            volume_level.clamp(0, 0x7FFF_FFFF)
        }
    }

    #[inline]
    fn update_adsr(ch: &mut Channel) {
        const LOG_INDEX: [u32; 8] = [0, 4, 6, 8, 9, 10, 11, 12];
        let mut current = ch.adsr_volume as i32;

        match ch.status {
            EnvelopePhase::Stopped | EnvelopePhase::KeyOn => return,
            EnvelopePhase::Attack => {
                let delta = if !ch.adsr_level.attack_exponential() || current < 0x6000_0000 {
                    get_adsr_delta((ch.adsr_level.attack_rate() ^ 0x7F).wrapping_sub(0x10))
                } else {
                    get_adsr_delta((ch.adsr_level.attack_rate() ^ 0x7F).wrapping_sub(0x18))
                };
                current = current.wrapping_add(delta as i32);
                if current < 0 {
                    current = MAX_ADSR_VOLUME as i32;
                    ch.status = EnvelopePhase::Decay;
                }
            }
            EnvelopePhase::Decay => {
                let decay_type = ((current as u32) >> 28) & 0x7;
                let idx = (4 * (ch.adsr_level.decay_rate() ^ 0x1F))
                    .wrapping_sub(0x18)
                    .wrapping_add(LOG_INDEX[decay_type as usize]);
                let delta = get_adsr_delta(idx);
                current = current.wrapping_sub(delta as i32);
                if (((current >> 27) & 0xF) as u32) <= ch.adsr_level.sustain_level() {
                    ch.status = EnvelopePhase::Sustain;
                }
            }
            EnvelopePhase::Sustain => {
                if !ch.adsr_rate.sustain_decrease() {
                    let delta = if !ch.adsr_rate.sustain_exponential() || current < 0x6000_0000 {
                        get_adsr_delta((ch.adsr_rate.sustain_rate() ^ 0x7F).wrapping_sub(0x10))
                    } else {
                        get_adsr_delta((ch.adsr_rate.sustain_rate() ^ 0x7F).wrapping_sub(0x18))
                    };
                    current = current.wrapping_add(delta as i32);
                    if current < 0 {
                        current = MAX_ADSR_VOLUME as i32;
                    }
                } else {
                    let delta = if !ch.adsr_rate.sustain_exponential() {
                        get_adsr_delta((ch.adsr_rate.sustain_rate() ^ 0x7F).wrapping_sub(0x0F))
                    } else {
                        let sustain_type = ((current as u32) >> 28) & 0x7;
                        let idx = (ch.adsr_rate.sustain_rate() ^ 0x7F)
                            .wrapping_sub(0x1B)
                            .wrapping_add(LOG_INDEX[sustain_type as usize]);
                        get_adsr_delta(idx)
                    };
                    current = current.wrapping_sub(delta as i32);
                    if current < 0 {
                        current = 0;
                    }
                }
            }
            EnvelopePhase::Release => {
                let delta = if !ch.adsr_rate.release_exponential() {
                    get_adsr_delta((4 * (ch.adsr_rate.release_rate() ^ 0x1F)).wrapping_sub(0x0C))
                } else {
                    let release_type = ((current as u32) >> 28) & 0x7;
                    let idx = (4 * (ch.adsr_rate.release_rate() ^ 0x1F))
                        .wrapping_sub(0x18)
                        .wrapping_add(LOG_INDEX[release_type as usize]);
                    get_adsr_delta(idx)
                };
                current = current.wrapping_sub(delta as i32);
                if current < 0 {
                    current = 0;
                    ch.status = EnvelopePhase::Stopped;
                }
            }
        }

        ch.adsr_volume = current as u32;
    }

    #[inline]
    fn reverb_ram_addr(&self, address: u32) -> usize {
        let mut absolute = self.reverb_curr_addr.wrapping_add(address);
        if absolute >= self.reverb_work_addr_end {
            let work_len = self.reverb_work_addr_end - self.reverb_work_addr_start;
            absolute = absolute.wrapping_sub(work_len);
            if absolute >= self.reverb_work_addr_end {
                absolute = self.reverb_work_addr_start
                    + (absolute.wrapping_sub(self.reverb_work_addr_start) % work_len);
            }
        }
        (absolute & (self.ram_size - 1)) as usize
    }

    #[inline]
    fn get_reverb_sample(&self, address: u32, ram: &[u8]) -> f32 {
        let mask = (self.ram_size - 1) as usize;
        let off = self.reverb_ram_addr(address);
        let val = i16::from_le_bytes([ram[off], ram[(off + 1) & mask]]);
        val as f32
    }

    #[inline]
    fn set_reverb_sample(&self, address: u32, value: f32, ram: &mut [u8]) {
        let mask = (self.ram_size - 1) as usize;
        let off = self.reverb_ram_addr(address);
        let clamped = value.clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        let bytes = clamped.to_le_bytes();
        ram[off] = bytes[0];
        ram[(off + 1) & mask] = bytes[1];
    }

    #[inline]
    fn get_reverb_offset(&self, param: ReverbParam) -> u32 {
        self.reverb[param as usize]
    }

    #[inline]
    fn get_reverb_coef(&self, param: ReverbParam) -> f32 {
        self.reverb_coefs[param as usize]
    }

    fn update_reverb(
        &mut self,
        reverb_sample: [i16; 2],
        out_l: &mut i16,
        out_r: &mut i16,
        ram: &mut [u8],
    ) {
        let is_odd = (self.reverb_ticks & 1) != 0;
        if is_odd {
            let input_sample_l = (reverb_sample[0] as f32) * 0.5;
            let input_sample_r = (reverb_sample[1] as f32) * 0.5;

            let iir_coef = self.get_reverb_coef(ReverbParam::IirCoef);
            let in_coef_l = self.get_reverb_coef(ReverbParam::InCoefL);
            let in_coef_r = self.get_reverb_coef(ReverbParam::InCoefR);

            let iir_input_a0 =
                self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirSrcA0), ram)
                    * iir_coef
                    + input_sample_l * in_coef_l;
            let iir_input_a1 =
                self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirSrcA1), ram)
                    * iir_coef
                    + input_sample_r * in_coef_r;
            let iir_input_b0 =
                self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirSrcB1), ram)
                    * iir_coef
                    + input_sample_l * in_coef_l;
            let iir_input_b1 =
                self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirSrcB0), ram)
                    * iir_coef
                    + input_sample_r * in_coef_r;

            let iir_alpha = self.get_reverb_coef(ReverbParam::IirAlpha);

            let iir_a0 = iir_input_a0 * iir_alpha
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirDestA0), ram)
                    * (1.0 - iir_alpha);
            let iir_a1 = iir_input_a1 * iir_alpha
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirDestA1), ram)
                    * (1.0 - iir_alpha);
            let iir_b0 = iir_input_b0 * iir_alpha
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirDestB0), ram)
                    * (1.0 - iir_alpha);
            let iir_b1 = iir_input_b1 * iir_alpha
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::IirDestB1), ram)
                    * (1.0 - iir_alpha);

            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::IirDestA0)
                    .wrapping_add(2),
                iir_a0,
                ram,
            );
            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::IirDestA1)
                    .wrapping_add(2),
                iir_a1,
                ram,
            );
            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::IirDestB0)
                    .wrapping_add(2),
                iir_b0,
                ram,
            );
            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::IirDestB1)
                    .wrapping_add(2),
                iir_b1,
                ram,
            );

            let acc_coef_a = self.get_reverb_coef(ReverbParam::AccCoefA);
            let acc_coef_b = self.get_reverb_coef(ReverbParam::AccCoefB);
            let acc_coef_c = self.get_reverb_coef(ReverbParam::AccCoefC);
            let acc_coef_d = self.get_reverb_coef(ReverbParam::AccCoefD);

            let acc0 = self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcA0), ram)
                * acc_coef_a
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcB0), ram)
                    * acc_coef_b
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcC0), ram)
                    * acc_coef_c
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcD0), ram)
                    * acc_coef_d;

            let acc1 = self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcA1), ram)
                * acc_coef_a
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcB1), ram)
                    * acc_coef_b
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcC1), ram)
                    * acc_coef_c
                + self.get_reverb_sample(self.get_reverb_offset(ReverbParam::AccSrcD1), ram)
                    * acc_coef_d;

            let fb_a0 = self.get_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestA0)
                    .wrapping_sub(self.get_reverb_offset(ReverbParam::FbSrcA)),
                ram,
            );
            let fb_a1 = self.get_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestA1)
                    .wrapping_sub(self.get_reverb_offset(ReverbParam::FbSrcA)),
                ram,
            );
            let fb_b0 = self.get_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestB0)
                    .wrapping_sub(self.get_reverb_offset(ReverbParam::FbSrcB)),
                ram,
            );
            let fb_b1 = self.get_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestB1)
                    .wrapping_sub(self.get_reverb_offset(ReverbParam::FbSrcB)),
                ram,
            );

            let fb_alpha = self.get_reverb_coef(ReverbParam::FbAlpha);
            let fb_x = self.get_reverb_coef(ReverbParam::FbX);

            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestA0),
                acc0 - fb_a0 * fb_alpha,
                ram,
            );
            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestA1),
                acc1 - fb_a1 * fb_alpha,
                ram,
            );
            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestB0),
                (fb_alpha * acc0) - fb_a0 * -fb_alpha - fb_b0 * fb_x,
                ram,
            );
            self.set_reverb_sample(
                self.get_reverb_offset(ReverbParam::MixDestB1),
                (fb_alpha * acc1) - fb_a1 * -fb_alpha - fb_b1 * fb_x,
                ram,
            );

            self.reverb_curr_addr = self.reverb_curr_addr.wrapping_add(2);
            if self.reverb_curr_addr >= self.reverb_work_addr_end {
                self.reverb_curr_addr = self.reverb_work_addr_start;
            }
            self.reverb_cached_valid = false;
        }

        if self.reverb_work_addr_start != 0 {
            let [wet_l, wet_r] = if !is_odd && self.reverb_cached_valid {
                self.reverb_cached_out
            } else {
                let sample_l = 0.333f32
                    * (self.get_reverb_sample(self.get_reverb_offset(ReverbParam::MixDestA0), ram)
                        + self.get_reverb_sample(
                            self.get_reverb_offset(ReverbParam::MixDestB0),
                            ram,
                        ));
                let sample_r = 0.333f32
                    * (self.get_reverb_sample(self.get_reverb_offset(ReverbParam::MixDestA1), ram)
                        + self.get_reverb_sample(
                            self.get_reverb_offset(ReverbParam::MixDestB1),
                            ram,
                        ));
                let out = [sample_l as i32, sample_r as i32];
                if is_odd {
                    self.reverb_cached_out = out;
                    self.reverb_cached_valid = true;
                }
                out
            };

            *out_l = (wet_l + (*out_l as i32)).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
            *out_r = (wet_r + (*out_r as i32)).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        }

        self.reverb_ticks = self.reverb_ticks.wrapping_add(1);
    }
}
