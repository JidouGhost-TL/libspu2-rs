// SPDX-License-Identifier: BSD-2-Clause

//! SPU2 and PS1 SPU register offset constants and mirroring helpers.
//!
//! SPU2 register offsets are relative to the hardware register block base
//! (`0x1F900000`). Constants represent core-0 offsets; use [`core_reg`] to
//! translate them for core 1.
//!
//! ## Address units
//! `VA_*`, `A_IRQA`, `A_TSA`, and `A_ESA` pairs store a byte address as
//! `(addr >> 17)` in `_HI` and `((addr >> 1) & 0xFFFF)` in `_LO`.
//!
//! ## Naming note
//! `A_KON_HI`, `A_KOFF_HI`, and `S_ENDX_HI` carry voices 0–15 (the low half
//! of the 24-voice mask); the corresponding `_LO` registers carry voices 16–23.

// Per-voice registers (stride 0x10, voices 0..24)

/// Voice left volume / sweep control (`VP_VOLL`).
pub const VP_VOLL: u32 = 0x000;
/// Voice right volume / sweep control (`VP_VOLR`).
pub const VP_VOLR: u32 = 0x002;
/// Voice pitch (`VP_PITCH`; `0x1000` = unity).
pub const VP_PITCH: u32 = 0x004;
/// Voice ADSR1 register (attack/decay/sustain level).
pub const VP_ADSR1: u32 = 0x006;
/// Voice ADSR2 register (sustain/release rates and modes).
pub const VP_ADSR2: u32 = 0x008;
/// Voice current envelope level (`VP_ENVX`).
pub const VP_ENVX: u32 = 0x00A;
/// Voice current left volume level (`VP_VOLXL`).
pub const VP_VOLXL: u32 = 0x00C;
/// Voice current right volume level (`VP_VOLXR`).
pub const VP_VOLXR: u32 = 0x00E;

// Core registers

/// Base offset of per-core status/control registers.
pub const S_REG_BASE: u32 = 0x180;
/// Pitch modulation enable mask, voices 0–15 (`S_PMON_HI`).
pub const S_PMON_HI: u32 = 0x180;
/// Pitch modulation enable mask, voices 16–23 (`S_PMON_LO`).
pub const S_PMON_LO: u32 = 0x182;
/// Noise generator enable mask, voices 0–15 (`S_NON_HI`).
pub const S_NON_HI: u32 = 0x184;
/// Noise generator enable mask, voices 16–23 (`S_NON_LO`).
pub const S_NON_LO: u32 = 0x186;
/// Voice dry left mix enable mask, voices 0–15 (`S_VMIXL_HI`).
pub const S_VMIXL_HI: u32 = 0x188;
/// Voice dry left mix enable mask, voices 16–23 (`S_VMIXL_LO`).
pub const S_VMIXL_LO: u32 = 0x18A;
/// Voice wet (reverb) left mix enable mask, voices 0–15 (`S_VMIXEL_HI`).
pub const S_VMIXEL_HI: u32 = 0x18C;
/// Voice wet (reverb) left mix enable mask, voices 16–23 (`S_VMIXEL_LO`).
pub const S_VMIXEL_LO: u32 = 0x18E;
/// Voice dry right mix enable mask, voices 0–15 (`S_VMIXR_HI`).
pub const S_VMIXR_HI: u32 = 0x190;
/// Voice dry right mix enable mask, voices 16–23 (`S_VMIXR_LO`).
pub const S_VMIXR_LO: u32 = 0x192;
/// Voice wet (reverb) right mix enable mask, voices 0–15 (`S_VMIXER_HI`).
pub const S_VMIXER_HI: u32 = 0x194;
/// Voice wet (reverb) right mix enable mask, voices 16–23 (`S_VMIXER_LO`).
pub const S_VMIXER_LO: u32 = 0x196;
/// Core dry/wet bus mix switches (`P_MMIX`).
pub const P_MMIX: u32 = 0x198;
/// Core control/attribute register (`CORE_ATTR`).
pub const CORE_ATTR: u32 = 0x19A;
/// Core IRQ target address, high half (`A_IRQA_HI`).
pub const A_IRQA_HI: u32 = 0x19C;
/// Core IRQ target address, low half (`A_IRQA_LO`).
pub const A_IRQA_LO: u32 = 0x19E;
/// Voice key-on trigger mask, voices 0–15 (`A_KON_HI`).
pub const A_KON_HI: u32 = 0x1A0;
/// Voice key-on trigger mask, voices 16–23 (`A_KON_LO`).
pub const A_KON_LO: u32 = 0x1A2;
/// Voice key-off trigger mask, voices 0–15 (`A_KOFF_HI`).
pub const A_KOFF_HI: u32 = 0x1A4;
/// Voice key-off trigger mask, voices 16–23 (`A_KOFF_LO`).
pub const A_KOFF_LO: u32 = 0x1A6;
/// RAM transfer start address, high half (`A_TSA_HI`).
pub const A_TSA_HI: u32 = 0x1A8;
/// RAM transfer start address, low half (`A_TSA_LO`).
pub const A_TSA_LO: u32 = 0x1AA;
/// Manual RAM transfer data port (`A_STD`).
pub const A_STD: u32 = 0x1AC;
/// AutoDMA / voice transfer mode register (`A_TS_MODE`).
pub const A_TS_MODE: u32 = 0x1B0;

// Per-voice addresses (stride 12, voices 0..24)

/// Base offset of per-voice address registers (`VA_*`).
pub const VA_REG_BASE: u32 = 0x1C0;
/// Voice start address, high half (`VA_SSA_HI`).
pub const VA_SSA_HI: u32 = 0x1C0;
/// Voice start address, low half (`VA_SSA_LO`).
pub const VA_SSA_LO: u32 = 0x1C2;
/// Voice loop/repeat address, high half (`VA_LSAX_HI`).
pub const VA_LSAX_HI: u32 = 0x1C4;
/// Voice loop/repeat address, low half (`VA_LSAX_LO`).
pub const VA_LSAX_LO: u32 = 0x1C6;
/// Voice next/current ADPCM address, high half (`VA_NAX_HI`).
pub const VA_NAX_HI: u32 = 0x1C8;
/// Voice next/current ADPCM address, low half (`VA_NAX_LO`).
pub const VA_NAX_LO: u32 = 0x1CA;

// Reverb and status

/// Base offset of reverb address and status registers.
pub const R_REG_BASE: u32 = 0x2E0;
/// Reverb work area start address, high half (`A_ESA_HI`).
pub const A_ESA_HI: u32 = 0x2E0;
/// Reverb work area start address, low half (`A_ESA_LO`).
pub const A_ESA_LO: u32 = 0x2E2;
/// Start of 22 x 32-bit reverb address parameters (`0x2E4..0x33C`).
pub const RVB_ADDR_BASE: u32 = 0x2E4;
/// End (exclusive) of reverb address parameters (`0x33C`).
pub const RVB_ADDR_END: u32 = 0x33C;
/// Reverb work area end address, high half (`A_EEA_HI`).
pub const A_EEA_HI: u32 = 0x33C;
/// Reverb work area end address, low half (`A_EEA_LO`).
pub const A_EEA_LO: u32 = 0x33E;
/// Voice end-of-sample flags, voices 0–15 (`S_ENDX_HI`).
pub const S_ENDX_HI: u32 = 0x340;
/// Voice end-of-sample flags, voices 16–23 (`S_ENDX_LO`).
pub const S_ENDX_LO: u32 = 0x342;
/// Core DMA / transfer status register (`STATX`).
pub const STATX: u32 = 0x344;

// Volume block (core 1 = +40 decimal, NOT | 0x400)

/// Core master left volume (`P_MVOLL`).
pub const P_MVOLL: u32 = 0x760;
/// Core master right volume (`P_MVOLR`).
pub const P_MVOLR: u32 = 0x762;
/// Core effect (reverb) return left volume (`P_EVOLL`).
pub const P_EVOLL: u32 = 0x764;
/// Core effect (reverb) return right volume (`P_EVOLR`).
pub const P_EVOLR: u32 = 0x766;
/// External input (Core 0 → Core 1) left volume (`P_AVOLL`).
pub const P_AVOLL: u32 = 0x768;
/// External input (Core 0 → Core 1) right volume (`P_AVOLR`).
pub const P_AVOLR: u32 = 0x76A;
/// AutoDMA block input left volume (`P_BVOLL`).
pub const P_BVOLL: u32 = 0x76C;
/// AutoDMA block input right volume (`P_BVOLR`).
pub const P_BVOLR: u32 = 0x76E;
/// Start of 10 x 16-bit reverb coefficient parameters (`0x774..0x788`).
pub const RVB_COEF_BASE: u32 = 0x774;
/// End (exclusive) of reverb coefficient parameters (`0x788`).
pub const RVB_COEF_END: u32 = 0x788;

// Core control bits (`CORE_ATTR` / `SPUCNT`)

/// `CORE_ATTR` / `SPUCNT` bits: DMA stopped (`0x0000`).
pub const CONTROL_DMA_STOP: u16 = 0x0000;
/// `CORE_ATTR` / `SPUCNT` bits: manual FIFO I/O transfer mode (`0x0010`).
pub const CONTROL_DMA_IO: u16 = 0x0010;
/// `CORE_ATTR` / `SPUCNT` bits: DMA write transfer mode (`0x0020`).
pub const CONTROL_DMA_WRITE: u16 = 0x0020;
/// `CORE_ATTR` / `SPUCNT` bits: DMA read transfer mode (`0x0030`).
pub const CONTROL_DMA_READ: u16 = 0x0030;
/// `CORE_ATTR` / `SPUCNT` bit: enable IRQ on address match (`0x0040`).
pub const CONTROL_IRQ: u16 = 0x0040;
/// `CORE_ATTR` / `SPUCNT` bit: enable reverb master write (`0x0080`).
pub const CONTROL_REVERB: u16 = 0x0080;
/// `CORE_ATTR` / `SPUCNT` bit: unmute core output (`0x4000`).
pub const CONTROL_UNMUTE: u16 = 0x4000;
/// `CORE_ATTR` / `SPUCNT` bit: enable core (`0x8000`).
pub const CONTROL_ENABLE: u16 = 0x8000;

// Transfer mode values (`A_TS_MODE`)

/// `A_TS_MODE` value: voice RAM DMA transfer (`0`).
pub const TRANSFER_MODE_VOICE: u16 = 0;
/// `A_TS_MODE` value: AutoDMA block input streaming on Core 0 (`1`).
pub const TRANSFER_MODE_BLOCK_CORE0IN: u16 = 1;
/// `A_TS_MODE` value: AutoDMA block input streaming on Core 1 (`2`).
pub const TRANSFER_MODE_BLOCK_CORE1IN: u16 = 2;
/// `A_TS_MODE` value: AutoDMA block read (`4`).
pub const TRANSFER_MODE_BLOCK_READ: u16 = 4;

// Global registers

/// Global S/PDIF output / bypass control (`C_SPDIF_OUT`).
pub const C_SPDIF_OUT: u32 = 0x7C0;
/// Global IRQ info / status register (`C_IRQINFO`).
pub const C_IRQINFO: u32 = 0x7C2;
/// Global S/PDIF mode register (`C_SPDIF_MODE`).
pub const C_SPDIF_MODE: u32 = 0x7C6;
/// Global S/PDIF media register (`C_SPDIF_MEDIA`).
pub const C_SPDIF_MEDIA: u32 = 0x7C8;

/// Hardware base address of the SPU2 register block in IOP address space.
pub const SPU2_REGS_BEGIN: u32 = 0x1F90_0000;

/// Byte stride between adjacent voice parameter blocks (`VP_*`).
pub const VOICE_STRIDE: u32 = 0x10;
/// Byte stride between adjacent voice address blocks (`VA_*`).
pub const VOICE_ADDR_STRIDE: u32 = 12;

/// Translates a core-0 register offset into the corresponding offset for `core` (`0` or `1`).
///
/// Offsets below `0x760` mirror to core 1 via `offset | 0x400`; offsets at or
/// above `0x760` (the volume/coefficient block) mirror via `offset + 40`.
#[inline]
#[must_use]
pub const fn core_reg(core: usize, offset: u32) -> u32 {
    if core == 0 {
        offset
    } else if offset < 0x760 {
        offset | 0x400
    } else {
        offset + 40
    }
}

/// PS1 SPU register offsets, relative to `0x1F801C00` (`0x000..0x400`).
///
/// Address registers in PS1 SPU use 8-byte units (`byte_address = value << 3`).
/// Constants outside Play!'s Tier-1 subset (`MVOLL`/`MVOLR`, `EVOLL`/`EVOLR`,
/// `PMON_LO`/`PMON_HI`, `NON_LO`/`NON_HI`, `TRANSFER_CTRL`) are provided for
/// register-map completeness; writes are accepted as no-ops and reads return `0`.
pub mod ps1 {
    /// Hardware base address of the PS1 SPU register block.
    pub const REGS_BEGIN: u32 = 0x1F80_1C00;
    /// Size of the PS1 SPU register space in bytes.
    pub const REGISTER_SPACE_SIZE: u32 = 0x400;

    /// Byte stride between adjacent voice register blocks (`0x000..0x180`).
    pub const VOICE_STRIDE: u32 = 0x10;

    // Per-voice offsets (stride 0x10, voices 0..24)

    /// PS1 voice left volume (`V_VOLL`).
    pub const V_VOLL: u32 = 0x000;
    /// PS1 voice right volume (`V_VOLR`).
    pub const V_VOLR: u32 = 0x002;
    /// PS1 voice pitch (`V_PITCH`).
    pub const V_PITCH: u32 = 0x004;
    /// PS1 voice ADPCM start address in 8-byte units (`V_START_ADDR`).
    pub const V_START_ADDR: u32 = 0x006;
    /// PS1 voice ADSR1 register (`V_ADSR1`).
    pub const V_ADSR1: u32 = 0x008;
    /// PS1 voice ADSR2 register (`V_ADSR2`).
    pub const V_ADSR2: u32 = 0x00A;
    /// PS1 voice current envelope level (`V_ENVX`).
    pub const V_ENVX: u32 = 0x00C;
    /// PS1 voice ADPCM repeat address in 8-byte units (`V_REPEAT_ADDR`).
    pub const V_REPEAT_ADDR: u32 = 0x00E;

    // Control and global registers

    /// PS1 master left volume (`MVOLL`).
    pub const MVOLL: u32 = 0x180;
    /// PS1 master right volume (`MVOLR`).
    pub const MVOLR: u32 = 0x182;
    /// PS1 reverb return left volume (`EVOLL`).
    pub const EVOLL: u32 = 0x184;
    /// PS1 reverb return right volume (`EVOLR`).
    pub const EVOLR: u32 = 0x186;
    /// PS1 key-on trigger mask, voices 0–15 (`KON_LO`).
    pub const KON_LO: u32 = 0x188;
    /// PS1 key-on trigger mask, voices 16–23 (`KON_HI`).
    pub const KON_HI: u32 = 0x18A;
    /// PS1 key-off trigger mask, voices 0–15 (`KOFF_LO`).
    pub const KOFF_LO: u32 = 0x18C;
    /// PS1 key-off trigger mask, voices 16–23 (`KOFF_HI`).
    pub const KOFF_HI: u32 = 0x18E;
    /// PS1 pitch modulation enable mask, voices 0–15 (`PMON_LO`).
    pub const PMON_LO: u32 = 0x190;
    /// PS1 pitch modulation enable mask, voices 16–23 (`PMON_HI`).
    pub const PMON_HI: u32 = 0x192;
    /// PS1 noise enable mask, voices 0–15 (`NON_LO`).
    pub const NON_LO: u32 = 0x194;
    /// PS1 noise enable mask, voices 16–23 (`NON_HI`).
    pub const NON_HI: u32 = 0x196;
    /// PS1 reverb send enable mask, voices 0–15 (`EON_LO`).
    pub const EON_LO: u32 = 0x198;
    /// PS1 reverb send enable mask, voices 16–23 (`EON_HI`).
    pub const EON_HI: u32 = 0x19A;
    /// PS1 voice end flags, voices 0–15 (`ENDX_LO`).
    pub const ENDX_LO: u32 = 0x19C;
    /// PS1 voice end flags, voices 16–23 (`ENDX_HI`).
    pub const ENDX_HI: u32 = 0x19E;
    /// PS1 reverb work area base address in 8-byte units (`RVB_WORK_ADDR`).
    pub const RVB_WORK_ADDR: u32 = 0x1A2;
    /// PS1 IRQ address in 8-byte units (`IRQ_ADDR`).
    pub const IRQ_ADDR: u32 = 0x1A4;
    /// PS1 RAM transfer start address in 8-byte units (`TRANSFER_ADDR`).
    pub const TRANSFER_ADDR: u32 = 0x1A6;
    /// PS1 RAM transfer data port (`TRANSFER_DATA`).
    pub const TRANSFER_DATA: u32 = 0x1A8;
    /// PS1 SPU control register (`SPUCNT`).
    pub const SPUCNT: u32 = 0x1AA;
    /// PS1 RAM transfer control register (`TRANSFER_CTRL`).
    pub const TRANSFER_CTRL: u32 = 0x1AC;
    /// PS1 SPU status register (`SPUSTAT`).
    pub const SPUSTAT: u32 = 0x1AE;

    /// Base offset of the 32 x 16-bit PS1 reverb parameter registers (`0x1C0..0x200`).
    pub const RVB_BASE: u32 = 0x1C0;
    /// End (exclusive) of the PS1 reverb parameter registers (`0x200`).
    pub const RVB_END: u32 = 0x200;
}
