// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! Core SPU/SPU2 synthesis engine (`Iop::CSpuBase` port).

use crate::adpcm::{SampleReader, SpuSampleCache};
use crate::envelope::{
    compute_channel_volume, update_adsr, AdsrLevel, AdsrRate, Channel, ChannelVolume,
};
use crate::inspect::EnvelopePhase;
use crate::noise::NoiseGenerator;
use crate::registers::{
    CONTROL_DMA_READ, CONTROL_DMA_STOP, CONTROL_ENABLE, CONTROL_IRQ, CONTROL_REVERB,
    CONTROL_UNMUTE, TRANSFER_MODE_BLOCK_CORE0IN, TRANSFER_MODE_BLOCK_CORE1IN, TRANSFER_MODE_VOICE,
};
use crate::reverb::{ReverbParam, ReverbState};

pub(crate) const MAX_CHANNEL: usize = 24;
pub(crate) const CONTROL_DMA: u16 = 0x0030;

const INIT_SAMPLE_RATE: u32 = 44_100;
const TIME_SCALE: u32 = 0x1000;
const RESET_IRQ_ADDR: u32 = !0u32;
const CAPTURE_BUFFER_SIZE: u32 = 0x400;

const CORE0_SIN_LEFT: u32 = 0x000;
const CORE1_SIN_LEFT: u32 = 0x800;
const CORE1_SIN_RIGHT: u32 = 0xA00;
const CORE0_OUTPUT_SIZE: u32 = 0x200;
const SOUND_INPUT_DATA_CORE0_BASE: u32 = 0x2000;
const SOUND_INPUT_DATA_CORE1_BASE: u32 = 0x2400;
const SOUND_INPUT_DATA_SIZE: usize = 0x400;
const SOUND_INPUT_DATA_SAMPLES: u32 = 0x100;

const DEFAULT_MASTER_VOL: u16 = 0x3FFF;
const DEFAULT_REVERB_VOL: i16 = 0x2AAA;

#[inline]
pub(crate) const fn clamp16(v: i32) -> i32 {
    if v < i16::MIN as i32 {
        i16::MIN as i32
    } else if v > i16::MAX as i32 {
        i16::MAX as i32
    } else {
        v
    }
}

/// Shared per-core IRQ address watcher.
pub(crate) struct SpuIrqWatcher {
    irq_addr: [u32; 2],
    irq_pending: [bool; 2],
}

impl SpuIrqWatcher {
    pub(crate) const fn new() -> Self {
        Self {
            irq_addr: [RESET_IRQ_ADDR; 2],
            irq_pending: [false; 2],
        }
    }

    pub(crate) fn reset(&mut self) {
        self.irq_addr = [RESET_IRQ_ADDR; 2];
        self.irq_pending = [false; 2];
    }

    pub(crate) fn set_irq_address(&mut self, core: usize, address: u32) {
        self.irq_addr[core] = address;
    }

    #[inline]
    pub(crate) const fn any_irq_below(&self, limit: u32) -> bool {
        self.irq_addr[0] < limit || self.irq_addr[1] < limit
    }

    #[inline]
    pub(crate) fn check_irq(&mut self, address: u32) {
        if address == self.irq_addr[0] {
            self.irq_pending[0] = true;
        }
        if address == self.irq_addr[1] {
            self.irq_pending[1] = true;
        }
    }

    pub(crate) fn clear_irq_pending(&mut self, core: usize) {
        self.irq_pending[core] = false;
    }

    #[inline]
    pub(crate) fn has_pending_irq(&self, core: usize) -> bool {
        self.irq_pending[core]
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

    #[inline]
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
        self.block_buffer
            .copy_from_slice(&block[..SOUND_INPUT_DATA_SIZE]);
        self.src_sample_idx = 0;
    }

    #[inline]
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
pub(crate) struct SpuBase {
    ram_size: u32,
    spu_number: usize,
    base_sampling_rate: u32,

    irq_addr: u32,
    irq_pending: bool,
    transfer_mode: u16,
    transfer_addr: u32,
    core0_output_offset: u32,
    channel_reverb_l: u32,
    channel_reverb_r: u32,
    channel_pmon: u32,
    channel_noise: u32,
    noise: NoiseGenerator,
    ctrl: u16,
    reverb: ReverbState,

    ps1_capture: bool,
    capture_offset: u32,

    channels: [Channel; MAX_CHANNEL],
    readers: [SampleReader; MAX_CHANNEL],

    block_reader: BlockSampleReader,
    sound_input_data_addr: u32,
    block_write_ptr: u32,

    pub(crate) master_vol_l: ChannelVolume,
    pub(crate) master_vol_r: ChannelVolume,
    pub(crate) master_vol_l_abs: i32,
    pub(crate) master_vol_r_abs: i32,
    pub(crate) reverb_vol_l: i16,
    pub(crate) reverb_vol_r: i16,
    pub(crate) input_vol_l: i32,
    pub(crate) input_vol_r: i32,
    pub(crate) ext_input_vol_l: i32,
    pub(crate) ext_input_vol_r: i32,
}

impl SpuBase {
    pub(crate) fn new(spu_number: usize, ram_size: u32) -> Self {
        assert!(ram_size.is_power_of_two() && ram_size >= 0x4000);
        let default_mvol = ChannelVolume(DEFAULT_MASTER_VOL);
        Self {
            ram_size,
            spu_number,
            base_sampling_rate: INIT_SAMPLE_RATE,
            irq_addr: RESET_IRQ_ADDR,
            irq_pending: false,
            transfer_mode: 0,
            transfer_addr: 0,
            core0_output_offset: 0,
            channel_reverb_l: 0,
            channel_reverb_r: 0,
            channel_pmon: 0,
            channel_noise: 0,
            noise: NoiseGenerator::new(),
            ctrl: 0,
            reverb: ReverbState::new(ram_size),
            ps1_capture: false,
            capture_offset: 0,
            channels: [Channel::default(); MAX_CHANNEL],
            readers: core::array::from_fn(|_| SampleReader::new(ram_size)),
            block_reader: BlockSampleReader::new(),
            sound_input_data_addr: if spu_number == 0 {
                SOUND_INPUT_DATA_CORE0_BASE
            } else {
                SOUND_INPUT_DATA_CORE1_BASE
            },
            block_write_ptr: 0,
            master_vol_l: default_mvol,
            master_vol_r: default_mvol,
            master_vol_l_abs: default_mvol.fixed_volume_i32(),
            master_vol_r_abs: default_mvol.fixed_volume_i32(),
            reverb_vol_l: DEFAULT_REVERB_VOL,
            reverb_vol_r: DEFAULT_REVERB_VOL,
            input_vol_l: 0x7FFF,
            input_vol_r: 0x7FFF,
            ext_input_vol_l: 0x7FFF,
            ext_input_vol_r: 0x7FFF,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.ctrl = 0;
        self.channel_reverb_l = 0;
        self.channel_reverb_r = 0;
        self.channel_pmon = 0;
        self.channel_noise = 0;
        self.noise.reset();
        self.irq_addr = RESET_IRQ_ADDR;
        self.irq_pending = false;
        self.transfer_mode = 0;
        self.transfer_addr = 0;
        self.core0_output_offset = 0;
        self.reverb.reset();
        self.base_sampling_rate = INIT_SAMPLE_RATE;

        let default_mvol = ChannelVolume(DEFAULT_MASTER_VOL);
        self.master_vol_l = default_mvol;
        self.master_vol_r = default_mvol;
        self.master_vol_l_abs = default_mvol.fixed_volume_i32();
        self.master_vol_r_abs = default_mvol.fixed_volume_i32();
        self.reverb_vol_l = DEFAULT_REVERB_VOL;
        self.reverb_vol_r = DEFAULT_REVERB_VOL;
        self.input_vol_l = 0x7FFF;
        self.input_vol_r = 0x7FFF;
        self.ext_input_vol_l = 0x7FFF;
        self.ext_input_vol_r = 0x7FFF;

        self.channels = [Channel::default(); MAX_CHANNEL];
        self.capture_offset = 0;

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

    pub(crate) fn set_ps1_capture(&mut self, enabled: bool) {
        self.ps1_capture = enabled;
    }

    #[inline]
    pub(crate) fn capture_second_half(&self) -> bool {
        (self.capture_offset & (CAPTURE_BUFFER_SIZE / 2)) != 0
    }

    #[inline]
    pub(crate) fn set_master_vol_l(&mut self, value: u16) {
        self.master_vol_l = ChannelVolume(value);
        if !self.master_vol_l.is_sweep() {
            self.master_vol_l_abs = self.master_vol_l.fixed_volume_i32();
        }
    }

    #[inline]
    pub(crate) fn set_master_vol_r(&mut self, value: u16) {
        self.master_vol_r = ChannelVolume(value);
        if !self.master_vol_r.is_sweep() {
            self.master_vol_r_abs = self.master_vol_r.fixed_volume_i32();
        }
    }

    #[inline]
    pub(crate) fn is_enabled(&self) -> bool {
        (self.ctrl & CONTROL_ENABLE) != 0
    }

    /// Returns `true` if this core has no active/stepping voices, no active
    /// reverb, no queued AutoDMA block input, and no IRQ enabled.
    pub(crate) fn is_completely_idle(&self) -> bool {
        if (self.ctrl & (CONTROL_REVERB | CONTROL_IRQ)) != 0 {
            return false;
        }
        if self.master_vol_l.is_sweep() || self.master_vol_r.is_sweep() {
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
                || ch.adsr_envx != 0
                || !self.readers[i].is_inert()
                || ch.volume_left.is_sweep()
                || ch.volume_right.is_sweep()
            {
                return false;
            }
        }
        true
    }

    #[inline]
    pub(crate) fn control(&self) -> u16 {
        self.ctrl
    }

    pub(crate) fn set_control(&mut self, value: u16, irq_watcher: &mut SpuIrqWatcher) {
        self.ctrl = value;
        if (self.ctrl & CONTROL_IRQ) == 0 {
            self.clear_irq_pending();
            irq_watcher.clear_irq_pending(self.spu_number);
        }
    }

    pub(crate) fn set_base_sampling_rate(&mut self, sampling_rate: u32) {
        self.base_sampling_rate = sampling_rate;
        self.block_reader.set_base_sampling_rate(sampling_rate);
    }

    pub(crate) fn set_input_bypass(&mut self, input_bypass: bool) {
        self.block_reader.set_spdif_bypass(input_bypass);
    }

    pub(crate) fn set_destination_sampling_rate(&mut self, sampling_rate: u32) {
        self.block_reader
            .set_destination_sampling_rate(sampling_rate);
        for reader in &mut self.readers {
            reader.set_destination_sampling_rate(sampling_rate);
        }
    }

    #[inline]
    pub(crate) fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    #[inline]
    pub(crate) fn clear_irq_pending(&mut self) {
        self.irq_pending = false;
    }

    #[inline]
    pub(crate) fn irq_address(&self) -> u32 {
        self.irq_addr
    }

    pub(crate) fn set_irq_address(&mut self, value: u32, irq_watcher: &mut SpuIrqWatcher) {
        self.irq_addr = value & (self.ram_size - 1);
        irq_watcher.set_irq_address(self.spu_number, self.irq_addr);
    }

    #[inline]
    pub(crate) fn transfer_mode(&self) -> u16 {
        self.transfer_mode
    }

    #[inline]
    pub(crate) fn set_transfer_mode(&mut self, transfer_mode: u16) {
        self.transfer_mode = transfer_mode;
    }

    #[inline]
    pub(crate) fn transfer_address(&self) -> u32 {
        self.transfer_addr
    }

    #[inline]
    pub(crate) fn set_transfer_address(&mut self, value: u32) {
        self.transfer_addr = value & (self.ram_size - 1);
    }

    #[inline]
    pub(crate) fn channel_reverb(&self) -> u32 {
        self.channel_reverb_l | self.channel_reverb_r
    }

    #[inline]
    pub(crate) fn channel_reverb_l(&self) -> u32 {
        self.channel_reverb_l
    }

    #[inline]
    pub(crate) fn channel_reverb_r(&self) -> u32 {
        self.channel_reverb_r
    }

    #[inline]
    pub(crate) fn set_channel_reverb_l_lo(&mut self, value: u16) {
        self.channel_reverb_l = (self.channel_reverb_l & 0xFFFF_0000) | (value as u32);
    }

    #[inline]
    pub(crate) fn set_channel_reverb_l_hi(&mut self, value: u16) {
        self.channel_reverb_l = (self.channel_reverb_l & 0x0000_FFFF) | ((value as u32) << 16);
    }

    #[inline]
    pub(crate) fn set_channel_reverb_r_lo(&mut self, value: u16) {
        self.channel_reverb_r = (self.channel_reverb_r & 0xFFFF_0000) | (value as u32);
    }

    #[inline]
    pub(crate) fn set_channel_reverb_r_hi(&mut self, value: u16) {
        self.channel_reverb_r = (self.channel_reverb_r & 0x0000_FFFF) | ((value as u32) << 16);
    }

    #[inline]
    pub(crate) fn set_channel_reverb_both_lo(&mut self, value: u16) {
        self.set_channel_reverb_l_lo(value);
        self.set_channel_reverb_r_lo(value);
    }

    #[inline]
    pub(crate) fn set_channel_reverb_both_hi(&mut self, value: u16) {
        self.set_channel_reverb_l_hi(value);
        self.set_channel_reverb_r_hi(value);
    }

    #[inline]
    pub(crate) fn channel_pmon(&self) -> u32 {
        self.channel_pmon
    }

    #[inline]
    pub(crate) fn set_channel_pmon_lo(&mut self, value: u16) {
        // Voice 0 cannot be pitch-modulated on PS1/PS2 hardware.
        self.channel_pmon = (self.channel_pmon & 0x00FF_0000) | ((value & 0xFFFE) as u32);
    }

    #[inline]
    pub(crate) fn set_channel_pmon_hi(&mut self, value: u16) {
        self.channel_pmon = (self.channel_pmon & 0x0000_FFFF) | (((value & 0x00FF) as u32) << 16);
    }

    #[inline]
    pub(crate) fn channel_noise(&self) -> u32 {
        self.channel_noise
    }

    #[inline]
    pub(crate) fn set_channel_noise_lo(&mut self, value: u16) {
        self.channel_noise = (self.channel_noise & 0x00FF_0000) | (value as u32);
    }

    #[inline]
    pub(crate) fn set_channel_noise_hi(&mut self, value: u16) {
        self.channel_noise = (self.channel_noise & 0x0000_FFFF) | (((value & 0x00FF) as u32) << 16);
    }

    #[inline]
    pub(crate) fn set_reverb_vol_l(&mut self, value: i16) {
        self.reverb_vol_l = value;
    }

    #[inline]
    pub(crate) fn set_reverb_vol_r(&mut self, value: i16) {
        self.reverb_vol_r = value;
    }

    #[inline]
    pub(crate) fn reverb_param(&self, param: ReverbParam) -> u32 {
        self.reverb.param(param)
    }

    #[inline]
    pub(crate) fn set_reverb_param(&mut self, param: ReverbParam, value: u32) {
        self.reverb.set_param(param, value);
    }

    pub(crate) fn end_flags(&self) -> u32 {
        let mut result = 0u32;
        for (i, reader) in self.readers.iter().enumerate() {
            if reader.end_flag() {
                result |= 1 << i;
            }
        }
        result
    }

    pub(crate) fn clear_end_flags(&mut self) {
        for reader in &mut self.readers {
            reader.clear_end_flag();
        }
    }

    #[inline]
    pub(crate) fn channel(&self, channel_number: usize) -> &Channel {
        &self.channels[channel_number]
    }

    #[inline]
    pub(crate) fn channel_mut(&mut self, channel_number: usize) -> &mut Channel {
        &mut self.channels[channel_number]
    }

    #[inline]
    pub(crate) fn set_voice_volume_left(&mut self, voice: usize, value: u16) {
        self.channels[voice].set_volume_left(value);
    }

    #[inline]
    pub(crate) fn set_voice_volume_right(&mut self, voice: usize, value: u16) {
        self.channels[voice].set_volume_right(value);
    }

    #[inline]
    pub(crate) fn set_voice_pitch(&mut self, voice: usize, value: u16) {
        self.channels[voice].pitch = value;
        self.readers[voice].set_pitch(self.base_sampling_rate, value);
    }

    #[inline]
    pub(crate) fn set_voice_adsr1(&mut self, voice: usize, value: u16) {
        self.channels[voice].adsr_level = AdsrLevel(value);
    }

    #[inline]
    pub(crate) fn set_voice_adsr2(&mut self, voice: usize, value: u16) {
        self.channels[voice].adsr_rate = AdsrRate(value);
    }

    #[inline]
    pub(crate) fn set_voice_envx(&mut self, voice: usize, value: u16) {
        self.channels[voice].adsr_envx = (value as i16) as i32;
    }

    pub(crate) fn send_key_on(&mut self, channels: u32) {
        let mut mask = channels & ((1u32 << MAX_CHANNEL) - 1);
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let ch = &mut self.channels[i];
            ch.status = EnvelopePhase::KeyOn;
            ch.repeat_set = false;
            ch.current = ch.address;
            ch.adsr_envx = 0;
            ch.adsr_cycles = 0;
            self.readers[i].clear_end_flag();
        }
    }

    pub(crate) fn send_key_off(&mut self, channels: u32) {
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
                    ch.adsr_cycles = 0;
                }
            }
        }
    }

    #[inline]
    pub(crate) fn reverb_work_address_start(&self) -> u32 {
        self.reverb.work_address_start()
    }

    #[inline]
    pub(crate) fn set_reverb_work_address_start(&mut self, address: u32) {
        self.reverb.set_work_address_start(address);
    }

    #[inline]
    pub(crate) fn reverb_work_address_end(&self) -> u32 {
        self.reverb.work_address_end()
    }

    #[inline]
    pub(crate) fn set_reverb_work_address_end(&mut self, address: u32) {
        self.reverb.set_work_address_end(address);
    }

    pub(crate) fn write_word(&mut self, value: u16, ram: &mut [u8], cache: &mut SpuSampleCache) {
        let mask = self.ram_size - 1;
        self.transfer_addr &= mask;
        let bytes = value.to_le_bytes();
        let addr = self.transfer_addr as usize;
        ram[addr] = bytes[0];
        ram[(addr + 1) & (mask as usize)] = bytes[1];
        cache.clear_range(self.transfer_addr, 2);
        self.transfer_addr = self.transfer_addr.wrapping_add(2) & mask;
    }

    pub(crate) fn receive_dma(
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

    pub(crate) fn latch_irq(&mut self, irq_watcher: &mut SpuIrqWatcher) {
        if (self.ctrl & CONTROL_IRQ) != 0 && irq_watcher.has_pending_irq(self.spu_number) {
            self.irq_pending = true;
            irq_watcher.clear_irq_pending(self.spu_number);
        }
    }

    #[inline]
    pub(crate) fn mix_samples(input_sample: i32, volume_level: i32, output: &mut i16) {
        let scaled = input_sample.wrapping_mul(volume_level) >> 15;
        let result = clamp16(scaled.wrapping_add(*output as i32));
        *output = result as i16;
    }

    #[inline]
    fn scale_master(sample: i16, master_vol: i32) -> i16 {
        clamp16((sample as i32).wrapping_mul(master_vol) >> 15) as i16
    }

    /// Renders `samples.len() / 2` stereo frames into `samples` (interleaved L, R).
    pub(crate) fn render(
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

        let update_reverb =
            ((self.ctrl & CONTROL_REVERB) != 0) && self.reverb.has_valid_work_area();
        if update_reverb {
            self.reverb.prepare_render();
        }
        let irq_enabled = (self.ctrl & CONTROL_IRQ) != 0;
        let unmuted = (self.ctrl & CONTROL_UNMUTE) != 0;
        let channel_reverb = self.channel_reverb();
        let need_voice_out = self.channel_pmon != 0 || self.ps1_capture;
        let check_ps1_capture_irq = self.ps1_capture && irq_watcher.any_irq_below(0x1000);
        let (noise_div_inc, noise_cnt_inc) = NoiseGenerator::clock_increments(self.ctrl);

        let master_l_sweep = self.master_vol_l.is_sweep();
        let master_r_sweep = self.master_vol_r.is_sweep();
        if !master_l_sweep {
            self.master_vol_l_abs = self.master_vol_l.fixed_volume_i32();
        }
        if !master_r_sweep {
            self.master_vol_r_abs = self.master_vol_r.fixed_volume_i32();
        }
        let mut master_l = self.master_vol_l_abs >> 16;
        let mut master_r = self.master_vol_r_abs >> 16;

        // Build active-voice bitmask; sync inert voices once outside the hot loop,
        // and hoist reader repeat synchronization for active voices.
        let mut active_mask = 0u32;
        for i in 0..MAX_CHANNEL {
            let ch = &mut self.channels[i];
            let reader = &mut self.readers[i];
            reader.set_repeat(ch.repeat);
            if ch.status == EnvelopePhase::Stopped
                && ch.adsr_envx == 0
                && reader.is_inert()
                && !ch.volume_left.is_sweep()
                && !ch.volume_right.is_sweep()
            {
                ch.current = reader.current();
                ch.volume_left_abs = ch.volume_left.fixed_volume_i32();
                ch.volume_right_abs = ch.volume_right.fixed_volume_i32();
            } else {
                if !ch.volume_left.is_sweep() {
                    ch.volume_left_abs = ch.volume_left.fixed_volume_i32();
                }
                if !ch.volume_right.is_sweep() {
                    ch.volume_right_abs = ch.volume_right.fixed_volume_i32();
                }
                active_mask |= 1u32 << i;
            }
        }

        let check_core0_irq = self.spu_number == 0
            && !self.ps1_capture
            && irq_enabled
            && (self.irq_addr < CORE0_OUTPUT_SIZE
                || (CORE1_SIN_LEFT..CORE1_SIN_LEFT + CORE0_OUTPUT_SIZE).contains(&self.irq_addr)
                || (CORE1_SIN_RIGHT..CORE1_SIN_RIGHT + CORE0_OUTPUT_SIZE).contains(&self.irq_addr));

        let mut voice_out = [0i16; MAX_CHANNEL];

        for frame in samples.chunks_exact_mut(2) {
            let mut out_l: i16 = 0;
            let mut out_r: i16 = 0;
            let mut reverb_sample = [0i16; 2];
            if need_voice_out {
                voice_out = [0i16; MAX_CHANNEL];
            }

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
                    ch.adsr_envx = 0;
                    ch.adsr_cycles = 0;
                    if reader.did_change_repeat() && !ch.repeat_set {
                        ch.repeat = reader.repeat();
                        reader.clear_did_change_repeat();
                    }
                } else {
                    if reader.is_done() {
                        ch.status = EnvelopePhase::Stopped;
                        ch.adsr_envx = 0;
                        reader.clear_is_done();
                    }
                    if reader.did_change_repeat() {
                        if !ch.repeat_set {
                            ch.repeat = reader.repeat();
                        } else {
                            reader.set_repeat(ch.repeat);
                        }
                        reader.clear_did_change_repeat();
                    }
                }

                let step_override = if i > 0 && ((self.channel_pmon & (1 << i)) != 0) {
                    let step = (ch.pitch as i16) as i32;
                    let factor = (voice_out[i - 1] as i32) + 0x8000;
                    let modulated_pitch = (((step * factor) >> 15) & 0xFFFF).min(0x3FFF) as u16;
                    Some(reader.step_for_pitch(self.base_sampling_rate, modulated_pitch))
                } else {
                    None
                };

                let adpcm_sample = reader.get_sample(step_override, ram, cache, irq_watcher);
                let read_sample = if (self.channel_noise & (1 << i)) != 0 {
                    self.noise.level() as i32
                } else {
                    adpcm_sample
                };

                // Apply current ENVX to this frame's sample before ticking ADSR for the next frame.
                let envx = ch.adsr_envx;
                update_adsr(ch);
                let vol_l = if ch.volume_left.is_sweep() {
                    let v = compute_channel_volume(ch.volume_left, ch.volume_left_abs);
                    ch.volume_left_abs = v;
                    v
                } else {
                    ch.volume_left_abs
                };
                let vol_r = if ch.volume_right.is_sweep() {
                    let v = compute_channel_volume(ch.volume_right, ch.volume_right_abs);
                    ch.volume_right_abs = v;
                    v
                } else {
                    ch.volume_right_abs
                };

                if read_sample == 0 {
                    continue;
                }

                let input_sample = read_sample.wrapping_mul(envx) >> 15;
                if need_voice_out {
                    voice_out[i] = clamp16(input_sample) as i16;
                }

                if input_sample == 0 {
                    continue;
                }

                let volume_left = vol_l >> 16;
                let volume_right = vol_r >> 16;

                if unmuted {
                    Self::mix_samples(input_sample, volume_left, &mut out_l);
                    Self::mix_samples(input_sample, volume_right, &mut out_r);
                }

                if update_reverb && ((channel_reverb & (1 << i)) != 0) {
                    Self::mix_samples(input_sample, volume_left, &mut reverb_sample[0]);
                    Self::mix_samples(input_sample, volume_right, &mut reverb_sample[1]);
                }
            }

            self.noise.step(noise_div_inc, noise_cnt_inc);

            if self.ps1_capture {
                self.write_ps1_capture(
                    voice_out[1],
                    voice_out[3],
                    check_ps1_capture_irq,
                    ram,
                    cache,
                    irq_watcher,
                );
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
                self.reverb.update(
                    reverb_sample,
                    unmuted,
                    self.reverb_vol_l,
                    self.reverb_vol_r,
                    &mut out_l,
                    &mut out_r,
                    ram,
                );
            }

            if master_l_sweep {
                self.master_vol_l_abs =
                    compute_channel_volume(self.master_vol_l, self.master_vol_l_abs);
                master_l = self.master_vol_l_abs >> 16;
            }
            if master_r_sweep {
                self.master_vol_r_abs =
                    compute_channel_volume(self.master_vol_r, self.master_vol_r_abs);
                master_r = self.master_vol_r_abs >> 16;
            }

            frame[0] = Self::scale_master(out_l, master_l);
            frame[1] = Self::scale_master(out_r, master_r);
        }

        // Sync current ADPCM read addresses once after the frame loop.
        let mut mask = active_mask;
        while mask != 0 {
            let i = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            self.channels[i].current = self.readers[i].current();
        }

        if self.spu_number == 0 && !self.ps1_capture && !check_core0_irq {
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
    fn write_ps1_capture(
        &mut self,
        voice1: i16,
        voice3: i16,
        check_irq: bool,
        ram: &mut [u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        let off = self.capture_offset as usize;
        let writes: [(usize, i16); 4] = [
            (off, 0),
            (0x0400 + off, 0),
            (0x0800 + off, voice1),
            (0x0C00 + off, voice3),
        ];
        for (addr, sample) in writes {
            let bytes = sample.to_le_bytes();
            ram[addr] = bytes[0];
            ram[addr + 1] = bytes[1];
            cache.invalidate_aligned_word(addr as u32);
            if check_irq {
                irq_watcher.check_irq(addr as u32);
            }
        }
        self.capture_offset = (self.capture_offset + 2) & (CAPTURE_BUFFER_SIZE - 2);
    }
}
