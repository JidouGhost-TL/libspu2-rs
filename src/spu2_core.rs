// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! Per-core SPU2 register dispatch (`Iop::Spu2::CCore` port).

use crate::registers;
use crate::spu_base::{
    AdsrLevel, AdsrRate, ReverbParam, SpuBase, SpuIrqWatcher, SpuSampleCache, CONTROL_DMA,
    MAX_CHANNEL,
};

pub const DEFAULT_BASE_SAMPLING_RATE: u32 = 48_000;

const ADDRESS_REGISTER_MAPPING: [ReverbParam; 22] = [
    ReverbParam::FbSrcA,
    ReverbParam::FbSrcB,
    ReverbParam::IirDestA0,
    ReverbParam::IirDestA1,
    ReverbParam::AccSrcA0,
    ReverbParam::AccSrcA1,
    ReverbParam::AccSrcB0,
    ReverbParam::AccSrcB1,
    ReverbParam::IirSrcA0,
    ReverbParam::IirSrcA1,
    ReverbParam::IirDestB0,
    ReverbParam::IirDestB1,
    ReverbParam::AccSrcC0,
    ReverbParam::AccSrcC1,
    ReverbParam::AccSrcD0,
    ReverbParam::AccSrcD1,
    ReverbParam::IirSrcB0,
    ReverbParam::IirSrcB1,
    ReverbParam::MixDestA0,
    ReverbParam::MixDestA1,
    ReverbParam::MixDestB0,
    ReverbParam::MixDestB1,
];

const COEFFICIENT_REGISTER_MAPPING: [ReverbParam; 10] = [
    ReverbParam::IirAlpha,
    ReverbParam::AccCoefA,
    ReverbParam::AccCoefB,
    ReverbParam::AccCoefC,
    ReverbParam::AccCoefD,
    ReverbParam::IirCoef,
    ReverbParam::FbAlpha,
    ReverbParam::FbX,
    ReverbParam::InCoefL,
    ReverbParam::InCoefR,
];

#[inline]
const fn get_address_lo(address: u32) -> u16 {
    ((address >> 1) & 0xFFFF) as u16
}

#[inline]
const fn get_address_hi(address: u32) -> u16 {
    ((address >> 17) & 0xFFFF) as u16
}

#[inline]
const fn set_address_lo(address: u32, value: u16) -> u32 {
    (address & 0xFFFE_0000) | ((value as u32) << 1)
}

#[inline]
const fn set_address_hi(address: u32, value: u16) -> u32 {
    (address & 0x0001_FFFE) | ((value as u32) << 17)
}

#[inline]
const fn decode_core_channel(offset: u32) -> Option<(usize, u32)> {
    if offset < registers::S_REG_BASE {
        let channel_id = ((offset >> 4) & 0x3F) as usize;
        let reg = offset & !(0x3F << 4);
        Some((channel_id, reg))
    } else if offset >= registers::VA_REG_BASE && offset < registers::R_REG_BASE {
        let channel_id =
            ((offset - registers::VA_REG_BASE) / registers::VOICE_ADDR_STRIDE) as usize;
        let reg = offset - (channel_id as u32) * registers::VOICE_ADDR_STRIDE;
        Some((channel_id, reg))
    } else {
        None
    }
}

/// Register-level wrapper around one [`SpuBase`] core.
pub struct Spu2Core {
    base_sampling_rate: u32,
    base: SpuBase,
}

impl Spu2Core {
    pub fn new(core_id: usize, ram_size: u32) -> Self {
        Self {
            base_sampling_rate: DEFAULT_BASE_SAMPLING_RATE,
            base: SpuBase::new(core_id, ram_size),
        }
    }

    pub fn reset(&mut self) {
        self.base_sampling_rate = DEFAULT_BASE_SAMPLING_RATE;
        self.base.reset();
    }

    pub fn set_base_sampling_rate(&mut self, rate: u32) {
        self.base_sampling_rate = rate;
        self.base.set_base_sampling_rate(rate);
    }

    pub fn base(&self) -> &SpuBase {
        &self.base
    }

    pub fn base_mut(&mut self) -> &mut SpuBase {
        &mut self.base
    }

    /// Reads a core-0-normalized register offset (`0x000..0x7FF`).
    pub fn read_register(&self, offset: u32) -> u16 {
        if let Some((channel_id, reg)) = decode_core_channel(offset) {
            self.read_register_channel(channel_id, reg)
        } else {
            self.read_register_core(offset)
        }
    }

    /// Writes a core-0-normalized register offset (`0x000..0x7FF`).
    pub fn write_register(
        &mut self,
        offset: u32,
        value: u16,
        ram: &mut [u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        if let Some((channel_id, reg)) = decode_core_channel(offset) {
            self.write_register_channel(channel_id, reg, value);
        } else {
            self.write_register_core(offset, value, ram, cache, irq_watcher);
        }
    }

    fn read_register_core(&self, offset: u32) -> u16 {
        match offset {
            registers::STATX => {
                if (self.base.control() & CONTROL_DMA) != 0 {
                    0x0080
                } else {
                    0x0000
                }
            }
            registers::S_ENDX_HI => (self.base.end_flags() & 0xFFFF) as u16,
            registers::S_ENDX_LO => (self.base.end_flags() >> 16) as u16,
            registers::S_PMON_HI => (self.base.channel_pmon() & 0xFFFF) as u16,
            registers::S_PMON_LO => (self.base.channel_pmon() >> 16) as u16,
            registers::S_NON_HI => (self.base.channel_noise() & 0xFFFF) as u16,
            registers::S_NON_LO => (self.base.channel_noise() >> 16) as u16,
            registers::S_VMIXER_HI => (self.base.channel_reverb() & 0xFFFF) as u16,
            registers::S_VMIXER_LO => (self.base.channel_reverb() >> 16) as u16,
            registers::CORE_ATTR => self.base.control(),
            registers::A_TS_MODE => self.base.transfer_mode(),
            registers::A_TSA_HI => get_address_hi(self.base.transfer_address()),
            registers::A_TSA_LO => get_address_lo(self.base.transfer_address()),
            registers::A_ESA_HI => get_address_hi(self.base.reverb_work_address_start()),
            registers::A_ESA_LO => get_address_lo(self.base.reverb_work_address_start()),
            registers::A_EEA_HI => {
                get_address_hi(self.base.reverb_work_address_end().saturating_sub(1))
            }
            registers::P_MVOLL => self.base.master_vol_l.raw(),
            registers::P_MVOLR => self.base.master_vol_r.raw(),
            registers::P_EVOLL => self.base.reverb_vol_l as u16,
            registers::P_EVOLR => self.base.reverb_vol_r as u16,
            registers::P_AVOLL => self.base.ext_input_vol_l as u16,
            registers::P_AVOLR => self.base.ext_input_vol_r as u16,
            registers::P_BVOLL => self.base.input_vol_l as u16,
            registers::P_BVOLR => self.base.input_vol_r as u16,
            _ => 0,
        }
    }

    fn write_register_core(
        &mut self,
        offset: u32,
        value: u16,
        ram: &mut [u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        if (registers::RVB_ADDR_BASE..registers::RVB_ADDR_END).contains(&offset) {
            let reg_index = ((offset - registers::RVB_ADDR_BASE) / 4) as usize;
            let param_id = ADDRESS_REGISTER_MAPPING[reg_index];
            let prev = self.base.reverb_param(param_id);
            let next = if (offset & 0x02) != 0 {
                set_address_lo(prev, value)
            } else {
                set_address_hi(prev, value)
            };
            self.base.set_reverb_param(param_id, next);
            return;
        }

        if (registers::RVB_COEF_BASE..registers::RVB_COEF_END).contains(&offset) {
            let reg_index = ((offset - registers::RVB_COEF_BASE) / 2) as usize;
            self.base
                .set_reverb_param(COEFFICIENT_REGISTER_MAPPING[reg_index], value as u32);
            return;
        }

        match offset {
            registers::CORE_ATTR => {
                self.base.set_base_sampling_rate(self.base_sampling_rate);
                self.base.set_control(value, irq_watcher);
            }
            registers::A_STD => {
                self.base.write_word(value, ram, cache);
            }
            registers::A_TS_MODE => {
                self.base.set_transfer_mode(value);
            }
            registers::S_PMON_HI => {
                self.base.set_channel_pmon_lo(value);
            }
            registers::S_PMON_LO => {
                self.base.set_channel_pmon_hi(value);
            }
            registers::S_NON_HI => {
                self.base.set_channel_noise_lo(value);
            }
            registers::S_NON_LO => {
                self.base.set_channel_noise_hi(value);
            }
            registers::S_VMIXER_HI => {
                self.base.set_channel_reverb_lo(value);
            }
            registers::S_VMIXER_LO => {
                self.base.set_channel_reverb_hi(value);
            }
            registers::A_KON_HI => {
                self.base.send_key_on(value as u32);
            }
            registers::A_KON_LO => {
                self.base.send_key_on((value as u32) << 16);
            }
            registers::A_KOFF_HI => {
                self.base.send_key_off(value as u32);
            }
            registers::A_KOFF_LO => {
                self.base.send_key_off((value as u32) << 16);
            }
            registers::S_ENDX_LO | registers::S_ENDX_HI => {
                if value != 0 {
                    self.base.clear_end_flags();
                }
            }
            registers::A_IRQA_HI => {
                let addr = set_address_hi(self.base.irq_address(), value);
                self.base.set_irq_address(addr, irq_watcher);
            }
            registers::A_IRQA_LO => {
                let addr = set_address_lo(self.base.irq_address(), value);
                self.base.set_irq_address(addr, irq_watcher);
            }
            registers::A_TSA_HI => {
                let addr = set_address_hi(self.base.transfer_address(), value);
                self.base.set_transfer_address(addr);
            }
            registers::A_TSA_LO => {
                let addr = set_address_lo(self.base.transfer_address(), value);
                self.base.set_transfer_address(addr);
            }
            registers::A_ESA_HI => {
                let addr = set_address_hi(self.base.reverb_work_address_start(), value);
                self.base.set_reverb_work_address_start(addr);
            }
            registers::A_ESA_LO => {
                let addr = set_address_lo(self.base.reverb_work_address_start(), value);
                self.base.set_reverb_work_address_start(addr);
            }
            registers::A_EEA_HI => {
                let addr = (((value as u32) & 0x0F) + 1) << 17;
                self.base.set_reverb_work_address_end(addr);
            }
            registers::P_MVOLL => {
                self.base.set_master_vol_l(value);
            }
            registers::P_MVOLR => {
                self.base.set_master_vol_r(value);
            }
            registers::P_EVOLL => {
                self.base.set_reverb_vol_l(value as i16);
            }
            registers::P_EVOLR => {
                self.base.set_reverb_vol_r(value as i16);
            }
            registers::P_AVOLL => {
                self.base.ext_input_vol_l = (value as i16) as i32;
            }
            registers::P_AVOLR => {
                self.base.ext_input_vol_r = (value as i16) as i32;
            }
            registers::P_BVOLL => {
                self.base.input_vol_l = (value as i16) as i32;
            }
            registers::P_BVOLR => {
                self.base.input_vol_r = (value as i16) as i32;
            }
            _ => {}
        }
    }

    fn read_register_channel(&self, channel_id: usize, reg: u32) -> u16 {
        if channel_id >= MAX_CHANNEL {
            return 0;
        }
        let ch = self.base.channel(channel_id);
        match reg {
            registers::VP_VOLL => ch.volume_left.raw(),
            registers::VP_VOLR => ch.volume_right.raw(),
            registers::VP_PITCH => ch.pitch,
            registers::VP_ADSR1 => ch.adsr_level.raw(),
            registers::VP_ADSR2 => ch.adsr_rate.raw(),
            registers::VP_ENVX => ch.adsr_envx as u16,
            registers::VP_VOLXL => (ch.volume_left_abs >> 16) as u16,
            registers::VP_VOLXR => (ch.volume_right_abs >> 16) as u16,
            registers::VA_SSA_HI => get_address_hi(ch.address),
            registers::VA_SSA_LO => get_address_lo(ch.address),
            registers::VA_LSAX_HI => get_address_hi(ch.repeat),
            registers::VA_LSAX_LO => get_address_lo(ch.repeat),
            registers::VA_NAX_HI => get_address_hi(ch.current),
            registers::VA_NAX_LO => get_address_lo(ch.current),
            _ => 0,
        }
    }

    fn write_register_channel(&mut self, channel_id: usize, reg: u32, value: u16) {
        if channel_id >= MAX_CHANNEL {
            return;
        }
        match reg {
            registers::VP_VOLL => self.base.channel_mut(channel_id).set_volume_left(value),
            registers::VP_VOLR => self.base.channel_mut(channel_id).set_volume_right(value),
            registers::VP_PITCH => {
                self.base.channel_mut(channel_id).pitch = value;
                self.base.on_channel_pitch_changed(channel_id);
            }
            registers::VP_ADSR1 => {
                self.base.channel_mut(channel_id).adsr_level = AdsrLevel(value);
            }
            registers::VP_ADSR2 => {
                self.base.channel_mut(channel_id).adsr_rate = AdsrRate(value);
            }
            registers::VP_ENVX => {
                self.base.channel_mut(channel_id).adsr_envx = (value as i16) as i32;
            }
            registers::VA_SSA_HI => {
                let ch = self.base.channel_mut(channel_id);
                ch.address = set_address_hi(ch.address, value);
            }
            registers::VA_SSA_LO => {
                let ch = self.base.channel_mut(channel_id);
                ch.address = set_address_lo(ch.address, value);
            }
            registers::VA_LSAX_HI => {
                let ch = self.base.channel_mut(channel_id);
                ch.repeat = set_address_hi(ch.repeat, value);
                ch.repeat_set = true;
            }
            registers::VA_LSAX_LO => {
                let ch = self.base.channel_mut(channel_id);
                ch.repeat = set_address_lo(ch.repeat, value);
                ch.repeat_set = true;
            }
            registers::VA_NAX_HI => {
                let ch = self.base.channel_mut(channel_id);
                ch.current = set_address_hi(ch.current, value);
            }
            registers::VA_NAX_LO => {
                let ch = self.base.channel_mut(channel_id);
                ch.current = set_address_lo(ch.current, value);
            }
            _ => {}
        }
    }
}
