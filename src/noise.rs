// SPDX-License-Identifier: BSD-2-Clause

//! Hardware 16-bit LFSR noise generator.

/// PlayStation 1 / PlayStation 2 SPU 16-bit Galois/Fibonacci LFSR noise generator.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct NoiseGenerator {
    divider: u32,
    counter: u32,
    level: i16,
}

impl NoiseGenerator {
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            divider: 0,
            counter: 0,
            level: 0,
        }
    }

    #[inline]
    pub(crate) fn reset(&mut self) {
        self.divider = 0;
        self.counter = 0;
        self.level = 0;
    }

    #[inline]
    pub(crate) const fn level(&self) -> i16 {
        self.level
    }

    /// Precomputes the `(div_inc, cnt_inc)` clock increments from `CORE_ATTR` / `SPUCNT`.
    #[inline]
    pub(crate) const fn clock_increments(ctrl: u16) -> (u32, u32) {
        let rf = ((ctrl >> 8) & 0x3F) as u32;
        if rf >= 0x3C {
            (0x8000, 8)
        } else {
            let shift = rf >> 2;
            let step = rf & 0x03;
            (2u32 << shift, 4 + step)
        }
    }

    /// Advances the noise LFSR by one output sample tick using precomputed increments.
    #[inline]
    pub(crate) fn step(&mut self, div_inc: u32, cnt_inc: u32) {
        self.divider += div_inc;
        if self.divider >= 0x8000 {
            self.divider -= 0x8000;
            self.counter += cnt_inc;
            if self.counter >= 8 {
                self.counter -= 8;
                let l = self.level as u16;
                let parity = ((l >> 15) ^ (l >> 12) ^ (l >> 11) ^ (l >> 10) ^ 1) & 1;
                self.level = ((l << 1) | parity) as i16;
            }
        }
    }
}
