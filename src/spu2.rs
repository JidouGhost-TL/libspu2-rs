// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! Dual-core SPU2 register router (`Iop::CSpu2` port).

use crate::adpcm::SpuSampleCache;
use crate::registers;
use crate::spu2_core::Spu2Core;
use crate::spu_base::SpuIrqWatcher;

const SPDIF_OUT_BYPASS: u16 = 0x0100;

/// Decoded target of an SPU2 MMIO register offset (`0x000..0x800`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegisterTarget {
    Core(usize, u32),
    Global(u32),
}

/// Decodes a register offset in `0x000..0x800` into its core or global target.
#[inline]
pub(crate) const fn decode_register_offset(offset: u32) -> RegisterTarget {
    if offset < 0x760 {
        let core_id = if (offset & 0x400) != 0 { 1 } else { 0 };
        RegisterTarget::Core(core_id, offset & !0x400)
    } else if offset < 0x7B0 {
        let core_id = ((offset - 0x760) / 40) as usize;
        RegisterTarget::Core(core_id, offset - (core_id as u32) * 40)
    } else {
        RegisterTarget::Global(offset)
    }
}

/// Returns `true` if `offset` maps to Core 1 in SPU2 register space.
#[inline]
pub(crate) const fn is_core1_register(offset: u32) -> bool {
    matches!(decode_register_offset(offset), RegisterTarget::Core(1, _))
}

/// Dual-core SPU2 register space (`0x000..0x800` relative to `0x1F900000`).
pub(crate) struct Spu2 {
    cores: [Spu2Core; 2],
    spdif_output: u16,
    spdif_mode: u16,
    spdif_media: u16,
}

impl Spu2 {
    pub(crate) fn new(ram_size: u32) -> Self {
        Self {
            cores: [Spu2Core::new(0, ram_size), Spu2Core::new(1, ram_size)],
            spdif_output: 0,
            spdif_mode: 0,
            spdif_media: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.cores[0].reset();
        self.cores[1].reset();
        self.spdif_output = 0;
        self.spdif_mode = 0;
        self.spdif_media = 0;
    }

    #[inline]
    pub(crate) fn core(&self, index: usize) -> &Spu2Core {
        &self.cores[index]
    }

    #[inline]
    pub(crate) fn core_mut(&mut self, index: usize) -> &mut Spu2Core {
        &mut self.cores[index]
    }

    #[inline]
    pub(crate) fn cores_mut(&mut self) -> (&mut Spu2Core, &mut Spu2Core) {
        let [c0, c1] = &mut self.cores;
        (c0, c1)
    }

    /// Returns the pending per-core IRQ bitmask (`C_IRQINFO` layout) without clearing it.
    pub(crate) fn irq_pending(&self) -> u16 {
        let mut result = 0u16;
        for (i, core) in self.cores.iter().enumerate() {
            if core.base().irq_pending() {
                result |= 1 << (i + 2);
            }
        }
        result
    }

    /// Reads an SPU2 register at `offset` (`0x000..0x800`).
    ///
    /// Reading `C_IRQINFO` reports and clears pending per-core IRQ flags.
    pub(crate) fn read_register(&mut self, offset: u32) -> u16 {
        match decode_register_offset(offset) {
            RegisterTarget::Core(core_id, local) => self.cores[core_id].read_register(local),
            RegisterTarget::Global(off) => self.read_global(off),
        }
    }

    /// Writes an SPU2 register at `offset` (`0x000..0x800`).
    pub(crate) fn write_register(
        &mut self,
        offset: u32,
        value: u16,
        ram: &mut [u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        match decode_register_offset(offset) {
            RegisterTarget::Core(core_id, local) => {
                self.cores[core_id].write_register(local, value, ram, cache, irq_watcher);
            }
            RegisterTarget::Global(off) => self.write_global(off, value),
        }
    }

    fn read_global(&mut self, offset: u32) -> u16 {
        match offset {
            registers::C_IRQINFO => {
                let mut result = 0u16;
                for (i, core) in self.cores.iter_mut().enumerate() {
                    if core.base().irq_pending() {
                        result |= 1 << (i + 2);
                        core.base_mut().clear_irq_pending();
                    }
                }
                result
            }
            registers::C_SPDIF_OUT => self.spdif_output,
            registers::C_SPDIF_MODE => self.spdif_mode,
            registers::C_SPDIF_MEDIA => self.spdif_media,
            _ => 0,
        }
    }

    fn write_global(&mut self, offset: u32, value: u16) {
        match offset {
            registers::C_SPDIF_OUT => {
                self.spdif_output = value;
                self.cores[0]
                    .base_mut()
                    .set_input_bypass((value & SPDIF_OUT_BYPASS) != 0);
            }
            registers::C_SPDIF_MODE => {
                self.spdif_mode = value;
            }
            registers::C_SPDIF_MEDIA => {
                self.spdif_media = value;
            }
            _ => {}
        }
    }
}
