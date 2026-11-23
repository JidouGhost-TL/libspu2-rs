// SPDX-License-Identifier: BSD-2-Clause

//! High-level voice-oriented [`Synth`] facade over [`Device`].

use crate::device::{Device, Error, StereoI16, VOICES_PER_CORE};
use crate::registers;

/// ADSR envelope parameters for [`VoiceParameters`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdsrConfig {
    /// Attack rate (`0..=0x7F`; `0` = fastest).
    pub attack_rate: u8,
    /// Decay rate (`0..=0xF`; `0` = fastest).
    pub decay_rate: u8,
    /// Sustain level (`0..=0xF`; `0xF` = maximum).
    pub sustain_level: u8,
    /// Sustain rate (`0..=0x7F`; `0x7F` = flat hold).
    pub sustain_rate: u8,
    /// Release rate (`0..=0x1F`; `0` = fastest).
    pub release_rate: u8,
    /// Use exponential attack curve (`ADSR1` bit 15).
    pub attack_exponential: bool,
    /// Use exponential sustain curve (`ADSR2` bit 15).
    pub sustain_exponential: bool,
    /// Decrease level during sustain instead of increasing (`ADSR2` bit 14).
    pub sustain_decrease: bool,
    /// Use exponential release curve (`ADSR2` bit 5).
    pub release_exponential: bool,
}

impl Default for AdsrConfig {
    fn default() -> Self {
        Self {
            attack_rate: 0,
            decay_rate: 0,
            sustain_level: 0x0F,
            sustain_rate: 0x7F,
            release_rate: 0,
            attack_exponential: false,
            sustain_exponential: false,
            sustain_decrease: false,
            release_exponential: false,
        }
    }
}

impl AdsrConfig {
    /// Decodes raw hardware `ADSR1` and `ADSR2` register words into an [`AdsrConfig`].
    #[must_use]
    pub const fn from_raw(adsr1: u16, adsr2: u16) -> Self {
        Self {
            sustain_level: (adsr1 & 0x0F) as u8,
            decay_rate: ((adsr1 >> 4) & 0x0F) as u8,
            attack_rate: ((adsr1 >> 8) & 0x7F) as u8,
            attack_exponential: (adsr1 & 0x8000) != 0,
            release_rate: (adsr2 & 0x1F) as u8,
            release_exponential: (adsr2 & 0x0020) != 0,
            sustain_rate: ((adsr2 >> 6) & 0x7F) as u8,
            sustain_decrease: (adsr2 & 0x4000) != 0,
            sustain_exponential: (adsr2 & 0x8000) != 0,
        }
    }

    /// Encodes this configuration into raw hardware `(ADSR1, ADSR2)` register words.
    #[must_use]
    pub const fn to_raw(&self) -> (u16, u16) {
        let adsr1 = ((self.sustain_level & 0x0F) as u16)
            | (((self.decay_rate & 0x0F) as u16) << 4)
            | (((self.attack_rate & 0x7F) as u16) << 8)
            | if self.attack_exponential { 0x8000 } else { 0 };
        let adsr2 = ((self.release_rate & 0x1F) as u16)
            | if self.release_exponential { 0x0020 } else { 0 }
            | (((self.sustain_rate & 0x7F) as u16) << 6)
            | if self.sustain_decrease { 0x4000 } else { 0 }
            | if self.sustain_exponential { 0x8000 } else { 0 };
        (adsr1, adsr2)
    }
}

/// High-level voice configuration translated to standard SPU2 register writes by [`Synth`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceParameters {
    /// Target core (`0` or `1`; only `0` in [`crate::DeviceMode::Ps1Spu`]).
    pub core: usize,
    /// Voice index within the core (`0..24`).
    pub index: usize,
    /// 16-byte-aligned ADPCM start address in SPU RAM.
    pub start_address: u32,
    /// Optional 16-byte-aligned ADPCM loop address; `None` clears any prior loop override.
    pub loop_address: Option<u32>,
    /// Playback pitch (`0..=0x3FFF`; `0x1000` = unity).
    pub pitch: u16,
    /// Fixed left volume (`0..=0x3FFF`).
    pub left_volume: u16,
    /// Fixed right volume (`0..=0x3FFF`).
    pub right_volume: u16,
    /// ADSR envelope configuration.
    pub adsr: AdsrConfig,
}

impl Default for VoiceParameters {
    fn default() -> Self {
        Self {
            core: 0,
            index: 0,
            start_address: 0,
            loop_address: None,
            pitch: 0x1000,
            left_volume: 0x3FFF,
            right_volume: 0x3FFF,
            adsr: AdsrConfig::default(),
        }
    }
}

/// Convenience voice-level wrapper over a [`Device`].
///
/// Operations translate to standard SPU2 register writes (plus clearing any
/// prior loop-address override when `VoiceParameters::loop_address` is `None`)
/// so low-level and high-level callers share a single authoritative synthesis
/// path.
#[derive(Debug)]
pub struct Synth<'a> {
    device: &'a mut Device,
}

impl<'a> Synth<'a> {
    /// Wraps a mutable reference to `device`.
    pub fn new(device: &'a mut Device) -> Self {
        Self { device }
    }

    /// Returns a shared reference to the underlying [`Device`].
    #[must_use]
    pub fn device(&self) -> &Device {
        self.device
    }

    /// Returns a mutable reference to the underlying [`Device`].
    pub fn device_mut(&mut self) -> &mut Device {
        self.device
    }

    /// Enables or disables `core` via `CORE_ATTR` bit 15 (`CONTROL_ENABLE`),
    /// preserving all other `CORE_ATTR` bits.
    pub fn enable_core(&mut self, core: usize, enabled: bool) -> Result<(), Error> {
        self.validate_core(core)?;
        let reg = registers::core_reg(core, registers::CORE_ATTR);
        let prev = self.device.read_register(reg)?;
        let next = if enabled {
            prev | registers::CONTROL_ENABLE
        } else {
            prev & !registers::CONTROL_ENABLE
        };
        self.device.write_register(reg, next)
    }

    /// Validates and writes all voice registers for `vp`.
    ///
    /// When `vp.loop_address` is `None`, any prior explicit loop override on
    /// that voice is cleared so embedded ADPCM loop flags take effect normally.
    pub fn configure_voice(&mut self, vp: &VoiceParameters) -> Result<(), Error> {
        self.validate_core_and_voice(vp.core, vp.index)?;
        self.validate_address(vp.start_address)?;
        if let Some(loop_addr) = vp.loop_address {
            self.validate_address(loop_addr)?;
        }
        if vp.left_volume > 0x3FFF || vp.right_volume > 0x3FFF {
            return Err(Error::InvalidArgument("volume"));
        }
        if vp.pitch > 0x3FFF {
            return Err(Error::InvalidArgument("pitch"));
        }
        let a = &vp.adsr;
        if a.attack_rate > 0x7F
            || a.decay_rate > 0x0F
            || a.sustain_level > 0x0F
            || a.sustain_rate > 0x7F
            || a.release_rate > 0x1F
        {
            return Err(Error::InvalidArgument("adsr"));
        }

        let (adsr1, adsr2) = a.to_raw();
        let idx = vp.index as u32;
        let vreg = |reg: u32| registers::core_reg(vp.core, reg + idx * registers::VOICE_STRIDE);
        let areg =
            |reg: u32| registers::core_reg(vp.core, reg + idx * registers::VOICE_ADDR_STRIDE);

        self.device
            .write_register(vreg(registers::VP_VOLL), vp.left_volume)?;
        self.device
            .write_register(vreg(registers::VP_VOLR), vp.right_volume)?;
        self.device
            .write_register(vreg(registers::VP_PITCH), vp.pitch)?;
        self.device
            .write_register(vreg(registers::VP_ADSR1), adsr1)?;
        self.device
            .write_register(vreg(registers::VP_ADSR2), adsr2)?;
        self.device.write_register(
            areg(registers::VA_SSA_HI),
            ((vp.start_address >> 17) & 0xFFFF) as u16,
        )?;
        self.device.write_register(
            areg(registers::VA_SSA_LO),
            ((vp.start_address >> 1) & 0xFFFF) as u16,
        )?;
        if let Some(loop_addr) = vp.loop_address {
            self.device.write_register(
                areg(registers::VA_LSAX_HI),
                ((loop_addr >> 17) & 0xFFFF) as u16,
            )?;
            self.device.write_register(
                areg(registers::VA_LSAX_LO),
                ((loop_addr >> 1) & 0xFFFF) as u16,
            )?;
        } else {
            self.device.clear_voice_loop(vp.core, vp.index);
        }
        Ok(())
    }

    /// Updates the pitch register (`0..=0x3FFF`) of a single voice live.
    pub fn set_pitch(&mut self, core: usize, voice: usize, pitch: u16) -> Result<(), Error> {
        self.validate_core_and_voice(core, voice)?;
        if pitch > 0x3FFF {
            return Err(Error::InvalidArgument("pitch"));
        }
        let reg = registers::core_reg(
            core,
            registers::VP_PITCH + (voice as u32) * registers::VOICE_STRIDE,
        );
        self.device.write_register(reg, pitch)
    }

    /// Updates the fixed left and right volume (`0..=0x3FFF`) of a single voice live.
    pub fn set_volume(
        &mut self,
        core: usize,
        voice: usize,
        left: u16,
        right: u16,
    ) -> Result<(), Error> {
        self.validate_core_and_voice(core, voice)?;
        if left > 0x3FFF || right > 0x3FFF {
            return Err(Error::InvalidArgument("volume"));
        }
        let stride = (voice as u32) * registers::VOICE_STRIDE;
        self.device
            .write_register(registers::core_reg(core, registers::VP_VOLL + stride), left)?;
        self.device.write_register(
            registers::core_reg(core, registers::VP_VOLR + stride),
            right,
        )
    }

    /// Updates the signed fixed left and right volume (`-0x4000..=0x3FFF`,
    /// where negative values invert phase) of a single voice live.
    pub fn set_volume_signed(
        &mut self,
        core: usize,
        voice: usize,
        left: i16,
        right: i16,
    ) -> Result<(), Error> {
        self.validate_core_and_voice(core, voice)?;
        if !(-0x4000..=0x3FFF).contains(&left) || !(-0x4000..=0x3FFF).contains(&right) {
            return Err(Error::InvalidArgument("volume"));
        }
        let stride = (voice as u32) * registers::VOICE_STRIDE;
        self.device.write_register(
            registers::core_reg(core, registers::VP_VOLL + stride),
            (left as u16) & 0x7FFF,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::VP_VOLR + stride),
            (right as u16) & 0x7FFF,
        )
    }

    /// Updates the ADSR envelope registers (`VP_ADSR1` / `VP_ADSR2`) of a single voice live.
    pub fn set_adsr(&mut self, core: usize, voice: usize, adsr: AdsrConfig) -> Result<(), Error> {
        self.validate_core_and_voice(core, voice)?;
        if adsr.attack_rate > 0x7F
            || adsr.decay_rate > 0x0F
            || adsr.sustain_level > 0x0F
            || adsr.sustain_rate > 0x7F
            || adsr.release_rate > 0x1F
        {
            return Err(Error::InvalidArgument("adsr"));
        }
        let (adsr1, adsr2) = adsr.to_raw();
        let stride = (voice as u32) * registers::VOICE_STRIDE;
        self.device.write_register(
            registers::core_reg(core, registers::VP_ADSR1 + stride),
            adsr1,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::VP_ADSR2 + stride),
            adsr2,
        )
    }

    /// Sets the 24-bit per-voice pitch modulation (`PMON`) enable mask on `core`.
    ///
    /// Bit 0 (voice 0) is ignored by hardware since there is no preceding voice.
    pub fn set_pmon(&mut self, core: usize, voice_mask: u32) -> Result<(), Error> {
        self.validate_core(core)?;
        self.device.write_register(
            registers::core_reg(core, registers::S_PMON_HI),
            (voice_mask & 0xFFFE) as u16,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::S_PMON_LO),
            ((voice_mask >> 16) & 0x00FF) as u16,
        )
    }

    /// Sets the 24-bit per-voice hardware noise generator (`NON`) enable mask on `core`.
    pub fn set_noise(&mut self, core: usize, voice_mask: u32) -> Result<(), Error> {
        self.validate_core(core)?;
        self.device.write_register(
            registers::core_reg(core, registers::S_NON_HI),
            (voice_mask & 0xFFFF) as u16,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::S_NON_LO),
            ((voice_mask >> 16) & 0x00FF) as u16,
        )
    }

    /// Sets the hardware noise clock frequency `shift` (`0..=15`) and `step` (`0..=3`)
    /// in `CORE_ATTR` bits 8–13, preserving all other `CORE_ATTR` bits.
    pub fn set_noise_clock(&mut self, core: usize, shift: u8, step: u8) -> Result<(), Error> {
        self.validate_core(core)?;
        if shift > 0x0F || step > 0x03 {
            return Err(Error::InvalidArgument("noise clock"));
        }
        let reg = registers::core_reg(core, registers::CORE_ATTR);
        let prev = self.device.read_register(reg)?;
        let next = (prev & !0x3F00) | ((shift as u16) << 10) | ((step as u16) << 8);
        self.device.write_register(reg, next)
    }

    /// Sets the signed fixed master output volume (`P_MVOLL` / `P_MVOLR`, `-0x4000..=0x3FFF`)
    /// for `core`.
    pub fn set_master_volume(&mut self, core: usize, left: i16, right: i16) -> Result<(), Error> {
        self.validate_core(core)?;
        if !(-0x4000..=0x3FFF).contains(&left) || !(-0x4000..=0x3FFF).contains(&right) {
            return Err(Error::InvalidArgument("master volume"));
        }
        self.device.write_register(
            registers::core_reg(core, registers::P_MVOLL),
            (left as u16) & 0x7FFF,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::P_MVOLR),
            (right as u16) & 0x7FFF,
        )
    }

    /// Sets the signed 16-bit reverb wet return volume (`P_EVOLL` / `P_EVOLR`, `-0x8000..=0x7FFF`)
    /// for `core`.
    pub fn set_reverb_volume(&mut self, core: usize, left: i16, right: i16) -> Result<(), Error> {
        self.validate_core(core)?;
        self.device
            .write_register(registers::core_reg(core, registers::P_EVOLL), left as u16)?;
        self.device
            .write_register(registers::core_reg(core, registers::P_EVOLR), right as u16)
    }

    /// Sets the reverb work area start (`A_ESA`) and end (`A_EEA_HI`) addresses in SPU RAM.
    ///
    /// `start_address` must be even and `< end_address`; `end_address` must be `<= ram_size()`
    /// and is mapped to the hardware 128 KiB `A_EEA_HI` boundary.
    pub fn set_reverb_work_area(
        &mut self,
        core: usize,
        start_address: u32,
        end_address: u32,
    ) -> Result<(), Error> {
        self.validate_core(core)?;
        if (start_address & 1) != 0 {
            return Err(Error::InvalidArgument("address alignment"));
        }
        if start_address >= end_address {
            return Err(Error::InvalidArgument("reverb work area"));
        }
        if (end_address as usize) > self.device.ram_size() {
            return Err(Error::OutOfRange("address"));
        }
        self.device.write_register(
            registers::core_reg(core, registers::A_ESA_HI),
            ((start_address >> 17) & 0xFFFF) as u16,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::A_ESA_LO),
            ((start_address >> 1) & 0xFFFF) as u16,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::A_EEA_HI),
            (((end_address - 1) >> 17) & 0x0F) as u16,
        )
    }

    /// Sets the 24-bit per-voice reverb send enable mask on `core`.
    pub fn set_reverb_send(&mut self, core: usize, voice_mask: u32) -> Result<(), Error> {
        self.validate_core(core)?;
        self.device.write_register(
            registers::core_reg(core, registers::S_VMIXER_HI),
            (voice_mask & 0xFFFF) as u16,
        )?;
        self.device.write_register(
            registers::core_reg(core, registers::S_VMIXER_LO),
            ((voice_mask >> 16) & 0x00FF) as u16,
        )
    }

    /// Triggers key-on for the voices in `voice_mask` on `core`.
    pub fn key_on(&mut self, core: usize, voice_mask: u32) -> Result<(), Error> {
        self.validate_core(core)?;
        self.device.write_register(
            registers::core_reg(core, registers::A_KON_HI),
            (voice_mask & 0xFFFF) as u16,
        )?;
        if (voice_mask >> 16) != 0 {
            self.device.write_register(
                registers::core_reg(core, registers::A_KON_LO),
                (voice_mask >> 16) as u16,
            )?;
        }
        Ok(())
    }

    /// Triggers key-on for a single `voice` (`0..24`) on `core`.
    pub fn key_on_voice(&mut self, core: usize, voice: usize) -> Result<(), Error> {
        self.validate_core_and_voice(core, voice)?;
        self.key_on(core, 1u32 << voice)
    }

    /// Triggers key-off for the voices in `voice_mask` on `core`.
    pub fn key_off(&mut self, core: usize, voice_mask: u32) -> Result<(), Error> {
        self.validate_core(core)?;
        self.device.write_register(
            registers::core_reg(core, registers::A_KOFF_HI),
            (voice_mask & 0xFFFF) as u16,
        )?;
        if (voice_mask >> 16) != 0 {
            self.device.write_register(
                registers::core_reg(core, registers::A_KOFF_LO),
                (voice_mask >> 16) as u16,
            )?;
        }
        Ok(())
    }

    /// Triggers key-off for a single `voice` (`0..24`) on `core`.
    pub fn key_off_voice(&mut self, core: usize, voice: usize) -> Result<(), Error> {
        self.validate_core_and_voice(core, voice)?;
        self.key_off(core, 1u32 << voice)
    }

    /// Convenience forwarding to [`Device::render_native`].
    ///
    /// # Panics
    /// Panics if `interleaved_stereo.len()` is odd.
    pub fn render_native(&mut self, interleaved_stereo: &mut [i16]) {
        self.device.render_native(interleaved_stereo);
    }

    /// Convenience forwarding to [`Device::render_stereo`].
    pub fn render_stereo(&mut self, frames: &mut [StereoI16]) {
        self.device.render_stereo(frames);
    }

    /// Convenience forwarding to [`Device::render_f32_planar`].
    pub fn render_f32_planar(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), Error> {
        self.device.render_f32_planar(left, right)
    }

    fn validate_core(&self, core: usize) -> Result<(), Error> {
        if core >= self.device.core_count() {
            return Err(Error::OutOfRange("core index"));
        }
        Ok(())
    }

    fn validate_core_and_voice(&self, core: usize, voice: usize) -> Result<(), Error> {
        self.validate_core(core)?;
        if voice >= VOICES_PER_CORE {
            return Err(Error::OutOfRange("voice index"));
        }
        Ok(())
    }

    fn validate_address(&self, addr: u32) -> Result<(), Error> {
        if (addr as usize) >= self.device.ram_size() {
            return Err(Error::OutOfRange("address"));
        }
        if (addr & 0x0F) != 0 {
            return Err(Error::InvalidArgument("address alignment"));
        }
        Ok(())
    }
}
