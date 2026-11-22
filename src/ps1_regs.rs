// SPDX-License-Identifier: BSD-2-Clause

//! PlayStation 1 SPU register dispatch (`0x1F801C00 + offset`, `0x000..0x400`).

use crate::device::Error;
use crate::registers::ps1;
use crate::spu_base::{
    AdsrLevel, AdsrRate, ReverbParam, SpuBase, SpuIrqWatcher, SpuSampleCache, CONTROL_DMA,
};

/// Reads a PS1 SPU register from Core 0 (`offset` even, `< 0x400`).
pub(crate) fn read_ps1_register(base: &SpuBase, offset: u32) -> Result<u16, Error> {
    if offset >= ps1::REGISTER_SPACE_SIZE || (offset & 1) != 0 {
        return Err(Error::OutOfRange("ps1 register offset"));
    }
    if offset < 0x180 {
        let voice = (offset / ps1::VOICE_STRIDE) as usize;
        let reg = offset % ps1::VOICE_STRIDE;
        let ch = base.channel(voice);
        return Ok(match reg {
            ps1::V_VOLL => ch.volume_left.raw(),
            ps1::V_VOLR => ch.volume_right.raw(),
            ps1::V_PITCH => ch.pitch,
            ps1::V_START_ADDR => (ch.address >> 3) as u16,
            ps1::V_ADSR1 => ch.adsr_level.raw(),
            ps1::V_ADSR2 => ch.adsr_rate.raw(),
            ps1::V_ENVX => (ch.adsr_volume >> 16) as u16,
            ps1::V_REPEAT_ADDR => (ch.repeat >> 3) as u16,
            _ => 0,
        });
    }
    if (ps1::RVB_BASE..ps1::RVB_END).contains(&offset) {
        let idx = ((offset - ps1::RVB_BASE) / 2) as usize;
        if let Some(param) = ReverbParam::from_index(idx) {
            let val = base.reverb_param(param);
            return Ok(if param.is_address() {
                (val >> 3) as u16
            } else {
                val as u16
            });
        }
        return Ok(0);
    }
    Ok(match offset {
        ps1::EON_LO => (base.channel_reverb() & 0xFFFF) as u16,
        ps1::EON_HI => (base.channel_reverb() >> 16) as u16,
        ps1::ENDX_LO => (base.end_flags() & 0xFFFF) as u16,
        ps1::ENDX_HI => (base.end_flags() >> 16) as u16,
        ps1::RVB_WORK_ADDR => (base.reverb_work_address_start() >> 3) as u16,
        ps1::IRQ_ADDR => (base.irq_address() >> 3) as u16,
        ps1::TRANSFER_ADDR => (base.transfer_address() >> 3) as u16,
        ps1::SPUCNT => base.control(),
        ps1::SPUSTAT => {
            let mut stat = base.control() & 0x3F;
            if base.irq_pending() {
                stat |= 0x40;
            }
            if (base.control() & CONTROL_DMA) != 0 {
                stat |= 0x80;
            }
            stat
        }
        _ => 0,
    })
}

/// Writes a PS1 SPU register to Core 0 (`offset` even, `< 0x400`).
pub(crate) fn write_ps1_register(
    base: &mut SpuBase,
    native_rate: u32,
    offset: u32,
    value: u16,
    ram: &mut [u8],
    cache: &mut SpuSampleCache,
    irq_watcher: &mut SpuIrqWatcher,
) -> Result<(), Error> {
    if offset >= ps1::REGISTER_SPACE_SIZE || (offset & 1) != 0 {
        return Err(Error::OutOfRange("ps1 register offset"));
    }
    if offset < 0x180 {
        let voice = (offset / ps1::VOICE_STRIDE) as usize;
        let reg = offset % ps1::VOICE_STRIDE;
        match reg {
            ps1::V_VOLL => base.channel_mut(voice).set_volume_left(value),
            ps1::V_VOLR => base.channel_mut(voice).set_volume_right(value),
            ps1::V_PITCH => {
                base.channel_mut(voice).pitch = value;
                base.on_channel_pitch_changed(voice);
            }
            ps1::V_START_ADDR => {
                base.channel_mut(voice).address = (value as u32) << 3;
            }
            ps1::V_ADSR1 => {
                base.channel_mut(voice).adsr_level = AdsrLevel(value);
            }
            ps1::V_ADSR2 => {
                base.channel_mut(voice).adsr_rate = AdsrRate(value);
            }
            ps1::V_ENVX => {
                base.channel_mut(voice).adsr_volume = (value as u32) << 16;
            }
            ps1::V_REPEAT_ADDR => {
                let ch = base.channel_mut(voice);
                ch.repeat = (value as u32) << 3;
                ch.repeat_set = true;
            }
            _ => {}
        }
        return Ok(());
    }
    if (ps1::RVB_BASE..ps1::RVB_END).contains(&offset) {
        let idx = ((offset - ps1::RVB_BASE) / 2) as usize;
        if let Some(param) = ReverbParam::from_index(idx) {
            let v = if param.is_address() {
                (value as u32) << 3
            } else {
                value as u32
            };
            base.set_reverb_param(param, v);
        }
        return Ok(());
    }
    match offset {
        ps1::KON_LO => base.send_key_on(value as u32),
        ps1::KON_HI => base.send_key_on((value as u32) << 16),
        ps1::KOFF_LO => base.send_key_off(value as u32),
        ps1::KOFF_HI => base.send_key_off((value as u32) << 16),
        ps1::EON_LO => base.set_channel_reverb_lo(value),
        ps1::EON_HI => base.set_channel_reverb_hi(value),
        ps1::ENDX_LO | ps1::ENDX_HI => {
            if value != 0 {
                base.clear_end_flags();
            }
        }
        ps1::RVB_WORK_ADDR => base.set_reverb_work_address_start((value as u32) << 3),
        ps1::IRQ_ADDR => base.set_irq_address((value as u32) << 3, irq_watcher),
        ps1::TRANSFER_ADDR => base.set_transfer_address((value as u32) << 3),
        ps1::TRANSFER_DATA => {
            base.write_word(value, ram, cache);
        }
        ps1::SPUCNT => {
            base.set_base_sampling_rate(native_rate);
            base.set_control(value, irq_watcher);
        }
        _ => {}
    }
    Ok(())
}
