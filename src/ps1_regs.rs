// SPDX-License-Identifier: BSD-2-Clause

//! PlayStation 1 SPU register dispatch (`0x1F801C00 + offset`, `0x000..0x400`).

use crate::adpcm::SpuSampleCache;
use crate::device::Error;
use crate::registers::ps1;
use crate::reverb::ReverbParam;
use crate::spu_base::{SpuBase, SpuIrqWatcher, CONTROL_DMA};

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
            ps1::V_ENVX => ch.adsr_envx as u16,
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
        ps1::MVOLL => base.master_vol_l.raw(),
        ps1::MVOLR => base.master_vol_r.raw(),
        ps1::EVOLL => base.reverb_vol_l as u16,
        ps1::EVOLR => base.reverb_vol_r as u16,
        ps1::PMON_LO => (base.channel_pmon() & 0xFFFF) as u16,
        ps1::PMON_HI => (base.channel_pmon() >> 16) as u16,
        ps1::NON_LO => (base.channel_noise() & 0xFFFF) as u16,
        ps1::NON_HI => (base.channel_noise() >> 16) as u16,
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
            if base.capture_second_half() {
                stat |= 0x0800;
            }
            stat
        }
        _ => 0,
    })
}

/// Writes a PS1 SPU register to Core 0 (`offset` even, `< 0x400`).
pub(crate) fn write_ps1_register(
    base: &mut SpuBase,
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
            ps1::V_VOLL => base.set_voice_volume_left(voice, value),
            ps1::V_VOLR => base.set_voice_volume_right(voice, value),
            ps1::V_PITCH => base.set_voice_pitch(voice, value),
            ps1::V_START_ADDR => {
                base.channel_mut(voice).address = (value as u32) << 3;
            }
            ps1::V_ADSR1 => base.set_voice_adsr1(voice, value),
            ps1::V_ADSR2 => base.set_voice_adsr2(voice, value),
            ps1::V_ENVX => base.set_voice_envx(voice, value),
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
        ps1::MVOLL => base.set_master_vol_l(value),
        ps1::MVOLR => base.set_master_vol_r(value),
        ps1::EVOLL => base.set_reverb_vol_l(value as i16),
        ps1::EVOLR => base.set_reverb_vol_r(value as i16),
        ps1::KON_LO => base.send_key_on(value as u32),
        ps1::KON_HI => base.send_key_on((value as u32) << 16),
        ps1::KOFF_LO => base.send_key_off(value as u32),
        ps1::KOFF_HI => base.send_key_off((value as u32) << 16),
        ps1::PMON_LO => base.set_channel_pmon_lo(value),
        ps1::PMON_HI => base.set_channel_pmon_hi(value),
        ps1::NON_LO => base.set_channel_noise_lo(value),
        ps1::NON_HI => base.set_channel_noise_hi(value),
        ps1::EON_LO => base.set_channel_reverb_both_lo(value),
        ps1::EON_HI => base.set_channel_reverb_both_hi(value),
        // PS1 ENDX is read-only; bits are cleared per-voice on KON.
        ps1::ENDX_LO | ps1::ENDX_HI => {}
        ps1::RVB_WORK_ADDR => base.set_reverb_work_address_start((value as u32) << 3),
        ps1::IRQ_ADDR => base.set_irq_address((value as u32) << 3, irq_watcher),
        ps1::TRANSFER_ADDR => base.set_transfer_address((value as u32) << 3),
        ps1::TRANSFER_DATA => {
            base.write_word(value, ram, cache);
        }
        ps1::SPUCNT => {
            base.set_control(value, irq_watcher);
        }
        _ => {}
    }
    Ok(())
}
