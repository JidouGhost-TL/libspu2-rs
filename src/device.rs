// SPDX-License-Identifier: BSD-2-Clause

//! High-level SPU2 / PS1 SPU device facade.

use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use crate::adpcm::SpuSampleCache;
use crate::inspect::VoiceState;
use crate::ps1_regs;
use crate::spu2::{self, Spu2};
use crate::spu_base::{SpuIrqWatcher, CHUNK_FRAMES, MAX_CHANNEL};

/// Number of SPU2 cores (`2`).
pub const CORE_COUNT: usize = 2;
/// Number of ADPCM voices per core (`24`).
pub const VOICES_PER_CORE: usize = MAX_CHANNEL;
/// SPU2 shared RAM size in bytes (`2 MiB`).
pub const RAM_SIZE: usize = 0x0020_0000;
/// SPU2 native output sample rate in Hz (`48000`).
pub const NATIVE_SAMPLE_RATE: u32 = 48_000;
/// SPU2 register window size in bytes (`0x800`).
pub const REGISTER_SPACE_SIZE: u32 = 0x800;

/// PS1 SPU RAM size (`512 KiB`).
pub const PS1_RAM_SIZE: usize = 0x0008_0000;
/// PS1 SPU native sample rate (`44100` Hz).
pub const PS1_SAMPLE_RATE: u32 = 44_100;

/// Hardware operating mode of a [`Device`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeviceMode {
    /// PlayStation 2 SPU2: 2 cores (48 voices), 2 MiB RAM, 48 kHz native clock.
    Spu2,
    /// PlayStation 1 SPU compatibility mode: Core 0 only (24 voices), 512 KiB RAM, 44.1 kHz clock.
    Ps1Spu,
}

/// Errors returned by [`Device`] and [`crate::Synth`] operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// An argument violated a structural constraint (e.g. alignment, length, or parameter range).
    InvalidArgument(&'static str),
    /// An index, offset, or RAM range exceeded the device bounds.
    OutOfRange(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            Self::OutOfRange(msg) => write!(f, "out of range: {msg}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// Interleaved stereo 16-bit PCM frame.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct StereoI16 {
    /// Left channel sample.
    pub left: i16,
    /// Right channel sample.
    pub right: i16,
}

/// Standalone PlayStation 2 SPU2 (or PS1 SPU) synthesizer device.
pub struct Device {
    mode: DeviceMode,
    output_sample_rate: u32,
    ram: Vec<u8>,
    sample_cache: SpuSampleCache,
    irq_watcher: SpuIrqWatcher,
    spu2: Spu2,
    frame_counter: u64,
    core1_l: [i16; CHUNK_FRAMES],
    core1_r: [i16; CHUNK_FRAMES],
}

impl fmt::Debug for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Device")
            .field("mode", &self.mode)
            .field("ram_size", &self.ram.len())
            .field("sample_rate", &self.sample_rate())
            .field("frame_counter", &self.frame_counter)
            .finish_non_exhaustive()
    }
}

impl Default for Device {
    fn default() -> Self {
        Self::new()
    }
}

impl Device {
    /// Creates a new PlayStation 2 SPU2 device (2 cores, 2 MiB RAM, 48 kHz).
    #[must_use]
    pub fn new() -> Self {
        Self::with_mode(DeviceMode::Spu2)
    }

    /// Creates a new device in PS1 SPU compatibility mode (Core 0, 512 KiB RAM, 44.1 kHz).
    #[must_use]
    pub fn new_ps1() -> Self {
        Self::with_mode(DeviceMode::Ps1Spu)
    }

    fn with_mode(mode: DeviceMode) -> Self {
        let (ram_size, native_rate) = match mode {
            DeviceMode::Spu2 => (RAM_SIZE, NATIVE_SAMPLE_RATE),
            DeviceMode::Ps1Spu => (PS1_RAM_SIZE, PS1_SAMPLE_RATE),
        };
        let mut dev = Self {
            mode,
            output_sample_rate: native_rate,
            ram: vec![0u8; ram_size],
            sample_cache: SpuSampleCache::new(ram_size as u32),
            irq_watcher: SpuIrqWatcher::new(),
            spu2: Spu2::new(ram_size as u32),
            frame_counter: 0,
            core1_l: [0i16; CHUNK_FRAMES],
            core1_r: [0i16; CHUNK_FRAMES],
        };
        dev.reset();
        dev
    }

    fn native_sample_rate(&self) -> u32 {
        match self.mode {
            DeviceMode::Spu2 => NATIVE_SAMPLE_RATE,
            DeviceMode::Ps1Spu => PS1_SAMPLE_RATE,
        }
    }

    /// Returns the operating mode of this device.
    #[must_use]
    pub fn mode(&self) -> DeviceMode {
        self.mode
    }

    /// Returns the number of active cores (`2` for SPU2, `1` for PS1 SPU).
    #[must_use]
    pub fn core_count(&self) -> usize {
        match self.mode {
            DeviceMode::Spu2 => CORE_COUNT,
            DeviceMode::Ps1Spu => 1,
        }
    }

    /// Returns the RAM size in bytes (`2 MiB` for SPU2, `512 KiB` for PS1 SPU).
    #[must_use]
    pub fn ram_size(&self) -> usize {
        self.ram.len()
    }

    /// Returns a read-only slice of the full SPU RAM buffer.
    #[must_use]
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Returns the current output sample rate in Hz (defaults to `48000` for
    /// SPU2 or `44100` for PS1 SPU; adjustable via [`Device::set_output_sample_rate`]).
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    /// Sets the rendered output sample rate in Hz (e.g. `48000` for WebAudio
    /// playback in PS1 SPU mode) without changing the hardware base clock.
    ///
    /// Returns [`Error::InvalidArgument`] if `rate == 0`.
    pub fn set_output_sample_rate(&mut self, rate: u32) -> Result<(), Error> {
        if rate == 0 {
            return Err(Error::InvalidArgument("sample rate must be non-zero"));
        }
        self.output_sample_rate = rate;
        self.spu2
            .core_mut(0)
            .base_mut()
            .set_destination_sampling_rate(rate);
        self.spu2
            .core_mut(1)
            .base_mut()
            .set_destination_sampling_rate(rate);
        Ok(())
    }

    /// Returns the pending per-core IRQ bitmask (`C_IRQINFO` layout: bit 2 = Core 0,
    /// bit 3 = Core 1) without clearing it.
    #[must_use]
    pub fn irq_pending(&self) -> u16 {
        let mask = self.spu2.irq_pending();
        match self.mode {
            DeviceMode::Spu2 => mask,
            DeviceMode::Ps1Spu => mask & (1 << 2),
        }
    }

    /// Resets all registers, voices, readers, reverb, IRQ state, output sample
    /// rate (back to the mode's native rate), and zeroes SPU RAM.
    pub fn reset(&mut self) {
        self.ram.fill(0);
        self.sample_cache.clear();
        self.irq_watcher.reset();
        self.spu2.reset();

        let rate = self.native_sample_rate();
        self.output_sample_rate = rate;
        self.spu2
            .core_mut(0)
            .base_mut()
            .set_base_sampling_rate(rate);
        self.spu2
            .core_mut(1)
            .base_mut()
            .set_base_sampling_rate(rate);
        self.spu2
            .core_mut(0)
            .base_mut()
            .set_destination_sampling_rate(rate);
        self.spu2
            .core_mut(1)
            .base_mut()
            .set_destination_sampling_rate(rate);
        self.spu2
            .core_mut(0)
            .base_mut()
            .set_ps1_capture(self.mode == DeviceMode::Ps1Spu);
        self.frame_counter = 0;
    }

    /// Reads an SPU2 register at `offset` (even, `< 0x800`).
    ///
    /// Reading `C_IRQINFO` reports and clears pending per-core IRQ flags.
    /// In [`DeviceMode::Ps1Spu`], Core 1 offsets return [`Error::OutOfRange`].
    pub fn read_register(&mut self, offset: u32) -> Result<u16, Error> {
        if offset >= REGISTER_SPACE_SIZE || (offset & 1) != 0 {
            return Err(Error::OutOfRange("register offset"));
        }
        if self.mode == DeviceMode::Ps1Spu && spu2::is_core1_register(offset) {
            return Err(Error::OutOfRange("core 1 register in PS1 mode"));
        }
        Ok(self.spu2.read_register(offset))
    }

    /// Writes an SPU2 register at `offset` (even, `< 0x800`).
    ///
    /// In [`DeviceMode::Ps1Spu`], Core 1 offsets return [`Error::OutOfRange`].
    pub fn write_register(&mut self, offset: u32, value: u16) -> Result<(), Error> {
        if offset >= REGISTER_SPACE_SIZE || (offset & 1) != 0 {
            return Err(Error::OutOfRange("register offset"));
        }
        if self.mode == DeviceMode::Ps1Spu && spu2::is_core1_register(offset) {
            return Err(Error::OutOfRange("core 1 register in PS1 mode"));
        }
        self.spu2.write_register(
            offset,
            value,
            &mut self.ram,
            &mut self.sample_cache,
            &mut self.irq_watcher,
        );
        Ok(())
    }

    /// Reads a PS1 SPU register at `offset` (even, `< 0x400`, relative to `0x1F801C00`).
    ///
    /// Operates on Core 0 using PS1 8-byte address units (`value << 3`).
    pub fn read_ps1_register(&self, offset: u32) -> Result<u16, Error> {
        ps1_regs::read_ps1_register(self.spu2.core(0).base(), offset)
    }

    /// Writes a PS1 SPU register at `offset` (even, `< 0x400`, relative to `0x1F801C00`).
    ///
    /// Operates on Core 0 using PS1 8-byte address units (`value << 3`).
    pub fn write_ps1_register(&mut self, offset: u32, value: u16) -> Result<(), Error> {
        ps1_regs::write_ps1_register(
            self.spu2.core_mut(0).base_mut(),
            offset,
            value,
            &mut self.ram,
            &mut self.sample_cache,
            &mut self.irq_watcher,
        )
    }

    /// Reads bytes from SPU RAM at `address`.
    pub fn read_ram(&self, address: u32, out: &mut [u8]) -> Result<(), Error> {
        let addr = address as usize;
        if addr > self.ram.len() || out.len() > self.ram.len() - addr {
            return Err(Error::OutOfRange("ram read"));
        }
        if out.is_empty() {
            return Ok(());
        }
        out.copy_from_slice(&self.ram[addr..addr + out.len()]);
        Ok(())
    }

    /// Writes bytes to SPU RAM at `address` and invalidates overlapping decoded-sample cache entries.
    pub fn write_ram(&mut self, address: u32, data: &[u8]) -> Result<(), Error> {
        let addr = address as usize;
        if addr > self.ram.len() || data.len() > self.ram.len() - addr {
            return Err(Error::OutOfRange("ram write"));
        }
        if data.is_empty() {
            return Ok(());
        }
        self.ram[addr..addr + data.len()].copy_from_slice(data);
        self.sample_cache.clear_range(address, data.len() as u32);
        Ok(())
    }

    /// Performs a DMA transfer on `core` (DMA4 for Core 0, DMA7 for Core 1),
    /// honoring `A_TSA`, `A_TS_MODE`, and `CORE_ATTR` DMA direction bits.
    ///
    /// Supports both voice RAM transfers (read/write, capped at `0x10` blocks
    /// per read call and `0x100` blocks per write call to match Play!'s DMAC
    /// throttling) and AutoDMA block input streaming. Returns the number of
    /// blocks transferred.
    pub fn receive_dma(
        &mut self,
        core: usize,
        buffer: &mut [u8],
        block_size: u32,
        block_amount: u32,
    ) -> Result<u32, Error> {
        if core >= self.core_count() {
            return Err(Error::OutOfRange("core index"));
        }
        let required = (block_size as u64).saturating_mul(block_amount as u64);
        if required > buffer.len() as u64 {
            return Err(Error::InvalidArgument("dma buffer too small"));
        }
        Ok(self.spu2.core_mut(core).base_mut().receive_dma(
            buffer,
            block_size,
            block_amount,
            &mut self.ram,
            &mut self.sample_cache,
        ))
    }

    /// Renders `out_l.len()` stereo frames (`<= CHUNK_FRAMES`) into planar `out_l` / `out_r`.
    fn render_chunk_planar(&mut self, out_l: &mut [i16], out_r: &mut [i16]) {
        let chunk = out_l.len();
        debug_assert_eq!(chunk, out_r.len());
        debug_assert!(chunk <= CHUNK_FRAMES);

        match self.mode {
            DeviceMode::Ps1Spu => {
                self.spu2.core_mut(0).base_mut().render_planar(
                    out_l,
                    out_r,
                    &mut self.ram,
                    &mut self.sample_cache,
                    &mut self.irq_watcher,
                );
            }
            DeviceMode::Spu2 => {
                let (core0, core1) = self.spu2.cores_mut();
                core0.base_mut().render_planar(
                    out_l,
                    out_r,
                    &mut self.ram,
                    &mut self.sample_cache,
                    &mut self.irq_watcher,
                );
                let core1_enabled = core1.base().is_enabled();
                if core1_enabled || !core1.base().is_completely_idle() {
                    core1.base_mut().render_planar(
                        &mut self.core1_l[..chunk],
                        &mut self.core1_r[..chunk],
                        &mut self.ram,
                        &mut self.sample_cache,
                        &mut self.irq_watcher,
                    );
                    core0.base_mut().latch_irq(&mut self.irq_watcher);
                    if core1_enabled {
                        let vol_l = core1.base().ext_input_vol_l;
                        let vol_r = core1.base().ext_input_vol_r;
                        for (dst, &src) in out_l.iter_mut().zip(&self.core1_l[..chunk]) {
                            *dst = (((*dst as i32).wrapping_mul(vol_l) >> 15) + (src as i32))
                                .clamp(i16::MIN as i32, i16::MAX as i32)
                                as i16;
                        }
                        for (dst, &src) in out_r.iter_mut().zip(&self.core1_r[..chunk]) {
                            *dst = (((*dst as i32).wrapping_mul(vol_r) >> 15) + (src as i32))
                                .clamp(i16::MIN as i32, i16::MAX as i32)
                                as i16;
                        }
                    }
                }
            }
        }

        self.frame_counter += chunk as u64;
    }

    /// Renders `interleaved_stereo.len() / 2` stereo frames into `interleaved_stereo`
    /// (`[L0, R0, L1, R1, ...]`).
    ///
    /// For compile-time even length without panicking, prefer [`Device::render_stereo`].
    ///
    /// # Panics
    /// Panics if `interleaved_stereo.len()` is odd.
    pub fn render_native(&mut self, interleaved_stereo: &mut [i16]) {
        assert!(
            interleaved_stereo.len() % 2 == 0,
            "interleaved stereo buffer length must be even"
        );
        let mut buf_l = [0i16; CHUNK_FRAMES];
        let mut buf_r = [0i16; CHUNK_FRAMES];
        for out_slice in interleaved_stereo.chunks_mut(CHUNK_FRAMES * 2) {
            let chunk = out_slice.len() / 2;
            self.render_chunk_planar(&mut buf_l[..chunk], &mut buf_r[..chunk]);
            for (dst, (&l, &r)) in out_slice
                .chunks_exact_mut(2)
                .zip(buf_l[..chunk].iter().zip(&buf_r[..chunk]))
            {
                dst[0] = l;
                dst[1] = r;
            }
        }
    }

    /// Renders stereo frames into a [`StereoI16`] slice.
    pub fn render_stereo(&mut self, frames: &mut [StereoI16]) {
        let mut buf_l = [0i16; CHUNK_FRAMES];
        let mut buf_r = [0i16; CHUNK_FRAMES];
        for dst_chunk in frames.chunks_mut(CHUNK_FRAMES) {
            let chunk = dst_chunk.len();
            self.render_chunk_planar(&mut buf_l[..chunk], &mut buf_r[..chunk]);
            for (dst, (&left, &right)) in dst_chunk
                .iter_mut()
                .zip(buf_l[..chunk].iter().zip(&buf_r[..chunk]))
            {
                *dst = StereoI16 { left, right };
            }
        }
    }

    /// Renders stereo frames normalized to `[-1.0, 1.0]` into planar `left` and
    /// `right` `f32` buffers (useful for WebAudio / WASM / audio plugins).
    ///
    /// Returns [`Error::InvalidArgument`] if `left.len() != right.len()`.
    pub fn render_f32_planar(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), Error> {
        if left.len() != right.len() {
            return Err(Error::InvalidArgument("channel buffer lengths must match"));
        }
        const SCALE: f32 = 1.0 / 32768.0;
        let mut buf_l = [0i16; CHUNK_FRAMES];
        let mut buf_r = [0i16; CHUNK_FRAMES];
        for (l_chunk, r_chunk) in left
            .chunks_mut(CHUNK_FRAMES)
            .zip(right.chunks_mut(CHUNK_FRAMES))
        {
            let chunk = l_chunk.len();
            self.render_chunk_planar(&mut buf_l[..chunk], &mut buf_r[..chunk]);
            for (dst, &src) in l_chunk.iter_mut().zip(&buf_l[..chunk]) {
                *dst = src as f32 * SCALE;
            }
            for (dst, &src) in r_chunk.iter_mut().zip(&buf_r[..chunk]) {
                *dst = src as f32 * SCALE;
            }
        }
        Ok(())
    }

    /// Renders a single core directly into `interleaved_stereo` without inter-core mixing.
    ///
    /// Note: advances [`Device::frame_counter`] by `interleaved_stereo.len() / 2` per call.
    pub fn render_core(
        &mut self,
        core: usize,
        interleaved_stereo: &mut [i16],
    ) -> Result<(), Error> {
        if core >= self.core_count() {
            return Err(Error::OutOfRange("core index"));
        }
        if interleaved_stereo.len() % 2 != 0 {
            return Err(Error::InvalidArgument(
                "interleaved stereo length must be even",
            ));
        }
        self.spu2.core_mut(core).base_mut().render(
            interleaved_stereo,
            &mut self.ram,
            &mut self.sample_cache,
            &mut self.irq_watcher,
        );
        self.frame_counter += (interleaved_stereo.len() / 2) as u64;
        Ok(())
    }

    pub(crate) fn clear_voice_loop(&mut self, core: usize, voice: usize) {
        let ch = self.spu2.core_mut(core).base_mut().channel_mut(voice);
        ch.repeat = 0;
        ch.repeat_set = false;
    }

    /// Returns a snapshot of the requested voice's state.
    pub fn inspect_voice(&self, core: usize, voice: usize) -> Result<VoiceState, Error> {
        if core >= self.core_count() {
            return Err(Error::OutOfRange("core index"));
        }
        if voice >= VOICES_PER_CORE {
            return Err(Error::OutOfRange("voice index"));
        }
        let base = self.spu2.core(core).base();
        let ch = base.channel(voice);
        Ok(VoiceState {
            pitch: ch.pitch,
            envx: ch.adsr_envx as u16,
            envelope_phase: ch.status,
            reached_end: ((base.end_flags() >> voice) & 1) != 0,
            start_address: ch.address,
            current_address: ch.current,
            repeat_address: ch.repeat,
            volume_left: ch.volume_left_abs,
            volume_right: ch.volume_right_abs,
        })
    }

    /// Returns the cumulative number of stereo frames rendered since the last reset.
    #[must_use]
    pub fn frame_counter(&self) -> u64 {
        self.frame_counter
    }
}
