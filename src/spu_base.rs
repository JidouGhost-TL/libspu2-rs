// SPDX-License-Identifier: BSD-2-Clause
//
// Portions derived from the Play! PlayStation 2 emulator.
// Original copyright retained in NOTICE.

//! Core SPU/SPU2 synthesis engine (`Iop::CSpuBase` port).

use crate::inspect::EnvelopePhase;
use crate::registers::{
    CONTROL_DMA_READ, CONTROL_DMA_STOP, CONTROL_ENABLE, CONTROL_IRQ, CONTROL_REVERB,
    CONTROL_UNMUTE, TRANSFER_MODE_BLOCK_CORE0IN, TRANSFER_MODE_BLOCK_CORE1IN, TRANSFER_MODE_VOICE,
};

pub const MAX_CHANNEL: usize = 24;
pub const REVERB_PARAM_COUNT: usize = 32;
pub const BUFFER_SAMPLES: usize = 28;
pub const CONTROL_DMA: u16 = 0x0030;

const INIT_SAMPLE_RATE: u32 = 44_100;
const PITCH_BASE: u32 = 0x1000;
const TIME_SCALE: u32 = 0x1000;
const RESET_IRQ_ADDR: u32 = !0u32;
const MAX_ADSR_LEVEL: i32 = 0x7FFF;
const CAPTURE_BUFFER_SIZE: u32 = 0x400;

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

/// Symmetric half-band 39-tap FIR filter odd-offset coefficients (offsets ±19, ±17, ..., ±1).
/// Center tap (offset 0) is `0x4000`; all other even-offset taps are `0`.
const REVERB_FIR_ODD_TAPS: [i32; 10] = [
    -0x0001, 0x0002, -0x000A, 0x0023, -0x0067, 0x010A, -0x0268, 0x0534, -0x0B90, 0x2806,
];
const REVERB_FIR_CENTER_TAP: i32 = 0x4000;

#[inline]
const fn clamp16(v: i32) -> i32 {
    if v < i16::MIN as i32 {
        i16::MIN as i32
    } else if v > i16::MAX as i32 {
        i16::MAX as i32
    } else {
        v
    }
}

const LINEAR_INCREASE_SWEEP_DELTAS: [u32; 0x80] = [
    0x3A00_0000,
    0x3000_0000,
    0x2800_0000,
    0x2000_0000,
    0x1D00_0000,
    0x1800_0000,
    0x1400_0000,
    0x1000_0000,
    0x0E80_0000,
    0x0C00_0000,
    0x0A00_0000,
    0x0800_0000,
    0x0740_0000,
    0x0600_0000,
    0x0500_0000,
    0x0400_0000,
    0x03A0_0000,
    0x0300_0000,
    0x0280_0000,
    0x0200_0000,
    0x01D0_0000,
    0x0180_0000,
    0x0140_0000,
    0x0100_0000,
    0x00E8_0000,
    0x00C0_0000,
    0x00A0_0000,
    0x0080_0000,
    0x0074_0000,
    0x0060_0000,
    0x0050_0000,
    0x0040_0000,
    0x003A_0000,
    0x0030_0000,
    0x0028_0000,
    0x0020_0000,
    0x001D_0000,
    0x0018_0000,
    0x0014_0000,
    0x0010_0000,
    0x000E_8000,
    0x000C_0000,
    0x000A_0000,
    0x0008_0000,
    0x0007_4000,
    0x0006_0000,
    0x0005_0000,
    0x0004_0000,
    0x0003_A000,
    0x0003_0000,
    0x0002_8000,
    0x0002_0000,
    0x0001_D000,
    0x0001_8000,
    0x0001_4000,
    0x0001_0000,
    0x0000_E800,
    0x0000_C000,
    0x0000_A000,
    0x0000_8000,
    0x0000_7400,
    0x0000_6000,
    0x0000_5000,
    0x0000_4000,
    0x0000_3A00,
    0x0000_3000,
    0x0000_2800,
    0x0000_2000,
    0x0000_1D00,
    0x0000_1800,
    0x0000_1400,
    0x0000_1000,
    0x0000_0E80,
    0x0000_0C00,
    0x0000_0A00,
    0x0000_0800,
    0x0000_0740,
    0x0000_0600,
    0x0000_0500,
    0x0000_0400,
    0x0000_03A0,
    0x0000_0300,
    0x0000_0280,
    0x0000_0200,
    0x0000_01D0,
    0x0000_0180,
    0x0000_0140,
    0x0000_0100,
    0x0000_00E8,
    0x0000_00C0,
    0x0000_00A0,
    0x0000_0080,
    0x0000_0074,
    0x0000_0060,
    0x0000_0050,
    0x0000_0040,
    0x0000_003A,
    0x0000_0030,
    0x0000_0028,
    0x0000_0020,
    0x0000_001D,
    0x0000_0018,
    0x0000_0014,
    0x0000_0010,
    0x0000_000E,
    0x0000_000C,
    0x0000_000A,
    0x0000_0008,
    0x0000_0007,
    0x0000_0006,
    0x0000_0005,
    0x0000_0004,
    0x0000_0003,
    0x0000_0003,
    0x0000_0002,
    0x0000_0002,
    0x0000_0001,
    0x0000_0001,
    0x0000_0001,
    0x0000_0001,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
];

const LINEAR_DECREASE_SWEEP_DELTAS: [u32; 0x80] = [
    0x4800_0000,
    0x4000_0000,
    0x3800_0000,
    0x3000_0000,
    0x2400_0000,
    0x2000_0000,
    0x1C00_0000,
    0x1800_0000,
    0x1200_0000,
    0x1000_0000,
    0x0E00_0000,
    0x0C00_0000,
    0x0900_0000,
    0x0800_0000,
    0x0700_0000,
    0x0600_0000,
    0x0480_0000,
    0x0400_0000,
    0x0380_0000,
    0x0300_0000,
    0x0240_0000,
    0x0200_0000,
    0x01C0_0000,
    0x0180_0000,
    0x0120_0000,
    0x0100_0000,
    0x00E0_0000,
    0x00C0_0000,
    0x0090_0000,
    0x0080_0000,
    0x0070_0000,
    0x0060_0000,
    0x0048_0000,
    0x0040_0000,
    0x0038_0000,
    0x0030_0000,
    0x0024_0000,
    0x0020_0000,
    0x001C_0000,
    0x0018_0000,
    0x0012_0000,
    0x0010_0000,
    0x000E_0000,
    0x000C_0000,
    0x0009_0000,
    0x0008_0000,
    0x0007_0000,
    0x0006_0000,
    0x0004_8000,
    0x0004_0000,
    0x0003_8000,
    0x0003_0000,
    0x0002_4000,
    0x0002_0000,
    0x0001_C000,
    0x0001_8000,
    0x0001_2000,
    0x0001_0000,
    0x0000_E000,
    0x0000_C000,
    0x0000_9000,
    0x0000_8000,
    0x0000_7000,
    0x0000_6000,
    0x0000_4800,
    0x0000_4000,
    0x0000_3800,
    0x0000_3000,
    0x0000_2400,
    0x0000_2000,
    0x0000_1C00,
    0x0000_1800,
    0x0000_1200,
    0x0000_1000,
    0x0000_0E00,
    0x0000_0C00,
    0x0000_0900,
    0x0000_0800,
    0x0000_0700,
    0x0000_0600,
    0x0000_0480,
    0x0000_0400,
    0x0000_0380,
    0x0000_0300,
    0x0000_0240,
    0x0000_0200,
    0x0000_01C0,
    0x0000_0180,
    0x0000_0120,
    0x0000_0100,
    0x0000_00E0,
    0x0000_00C0,
    0x0000_0090,
    0x0000_0080,
    0x0000_0070,
    0x0000_0060,
    0x0000_0048,
    0x0000_0040,
    0x0000_0038,
    0x0000_0030,
    0x0000_0024,
    0x0000_0020,
    0x0000_001C,
    0x0000_0018,
    0x0000_0012,
    0x0000_0010,
    0x0000_000E,
    0x0000_000C,
    0x0000_0009,
    0x0000_0008,
    0x0000_0007,
    0x0000_0006,
    0x0000_0004,
    0x0000_0004,
    0x0000_0003,
    0x0000_0003,
    0x0000_0002,
    0x0000_0002,
    0x0000_0001,
    0x0000_0001,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
    0x0000_0000,
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

/// PlayStation 1 / PlayStation 2 SPU 512-entry 4-tap Gaussian interpolation ROM table.
#[rustfmt::skip]
const GAUSS_TABLE: [i32; 512] = [
    -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001,
    -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001, -0x001,
    0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0001,
    0x0001, 0x0001, 0x0001, 0x0002, 0x0002, 0x0002, 0x0003, 0x0003,
    0x0003, 0x0004, 0x0004, 0x0005, 0x0005, 0x0006, 0x0007, 0x0007,
    0x0008, 0x0009, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E,
    0x000F, 0x0010, 0x0011, 0x0012, 0x0013, 0x0015, 0x0016, 0x0018,
    0x0019, 0x001B, 0x001C, 0x001E, 0x0020, 0x0021, 0x0023, 0x0025,
    0x0027, 0x0029, 0x002C, 0x002E, 0x0030, 0x0033, 0x0035, 0x0038,
    0x003A, 0x003D, 0x0040, 0x0043, 0x0046, 0x0049, 0x004D, 0x0050,
    0x0054, 0x0057, 0x005B, 0x005F, 0x0063, 0x0067, 0x006B, 0x006F,
    0x0074, 0x0078, 0x007D, 0x0082, 0x0087, 0x008C, 0x0091, 0x0096,
    0x009C, 0x00A1, 0x00A7, 0x00AD, 0x00B3, 0x00BA, 0x00C0, 0x00C7,
    0x00CD, 0x00D4, 0x00DB, 0x00E3, 0x00EA, 0x00F2, 0x00FA, 0x0101,
    0x010A, 0x0112, 0x011B, 0x0123, 0x012C, 0x0135, 0x013F, 0x0148,
    0x0152, 0x015C, 0x0166, 0x0171, 0x017B, 0x0186, 0x0191, 0x019C,
    0x01A8, 0x01B4, 0x01C0, 0x01CC, 0x01D9, 0x01E5, 0x01F2, 0x0200,
    0x020D, 0x021B, 0x0229, 0x0237, 0x0246, 0x0255, 0x0264, 0x0273,
    0x0283, 0x0293, 0x02A3, 0x02B4, 0x02C4, 0x02D6, 0x02E7, 0x02F9,
    0x030B, 0x031D, 0x0330, 0x0343, 0x0356, 0x036A, 0x037E, 0x0392,
    0x03A7, 0x03BC, 0x03D1, 0x03E7, 0x03FC, 0x0413, 0x042A, 0x0441,
    0x0458, 0x0470, 0x0488, 0x04A0, 0x04B9, 0x04D2, 0x04EC, 0x0506,
    0x0520, 0x053B, 0x0556, 0x0572, 0x058E, 0x05AA, 0x05C7, 0x05E4,
    0x0601, 0x061F, 0x063E, 0x065C, 0x067C, 0x069B, 0x06BB, 0x06DC,
    0x06FD, 0x071E, 0x0740, 0x0762, 0x0784, 0x07A7, 0x07CB, 0x07EF,
    0x0813, 0x0838, 0x085D, 0x0883, 0x08A9, 0x08D0, 0x08F7, 0x091E,
    0x0946, 0x096F, 0x0998, 0x09C1, 0x09EB, 0x0A16, 0x0A40, 0x0A6C,
    0x0A98, 0x0AC4, 0x0AF1, 0x0B1E, 0x0B4C, 0x0B7A, 0x0BA9, 0x0BD8,
    0x0C07, 0x0C38, 0x0C68, 0x0C99, 0x0CCB, 0x0CFD, 0x0D30, 0x0D63,
    0x0D97, 0x0DCB, 0x0E00, 0x0E35, 0x0E6B, 0x0EA1, 0x0ED7, 0x0F0F,
    0x0F46, 0x0F7F, 0x0FB7, 0x0FF1, 0x102A, 0x1065, 0x109F, 0x10DB,
    0x1116, 0x1153, 0x118F, 0x11CD, 0x120B, 0x1249, 0x1288, 0x12C7,
    0x1307, 0x1347, 0x1388, 0x13C9, 0x140B, 0x144D, 0x1490, 0x14D4,
    0x1517, 0x155C, 0x15A0, 0x15E6, 0x162C, 0x1672, 0x16B9, 0x1700,
    0x1747, 0x1790, 0x17D8, 0x1821, 0x186B, 0x18B5, 0x1900, 0x194B,
    0x1996, 0x19E2, 0x1A2E, 0x1A7B, 0x1AC8, 0x1B16, 0x1B64, 0x1BB3,
    0x1C02, 0x1C51, 0x1CA1, 0x1CF1, 0x1D42, 0x1D93, 0x1DE5, 0x1E37,
    0x1E89, 0x1EDC, 0x1F2F, 0x1F82, 0x1FD6, 0x202A, 0x207F, 0x20D4,
    0x2129, 0x217F, 0x21D5, 0x222C, 0x2282, 0x22DA, 0x2331, 0x2389,
    0x23E1, 0x2439, 0x2492, 0x24EB, 0x2545, 0x259E, 0x25F8, 0x2653,
    0x26AD, 0x2708, 0x2763, 0x27BE, 0x281A, 0x2876, 0x28D2, 0x292E,
    0x298B, 0x29E7, 0x2A44, 0x2AA1, 0x2AFF, 0x2B5C, 0x2BBA, 0x2C18,
    0x2C76, 0x2CD4, 0x2D33, 0x2D91, 0x2DF0, 0x2E4F, 0x2EAE, 0x2F0D,
    0x2F6C, 0x2FCC, 0x302B, 0x308B, 0x30EA, 0x314A, 0x31AA, 0x3209,
    0x3269, 0x32C9, 0x3329, 0x3389, 0x33E9, 0x3449, 0x34A9, 0x3509,
    0x3569, 0x35C9, 0x3629, 0x3689, 0x36E8, 0x3748, 0x37A8, 0x3807,
    0x3867, 0x38C6, 0x3926, 0x3985, 0x39E4, 0x3A43, 0x3AA2, 0x3B00,
    0x3B5F, 0x3BBD, 0x3C1B, 0x3C79, 0x3CD7, 0x3D35, 0x3D92, 0x3DEF,
    0x3E4C, 0x3EA9, 0x3F05, 0x3F62, 0x3FBD, 0x4019, 0x4074, 0x40D0,
    0x412A, 0x4185, 0x41DF, 0x4239, 0x4292, 0x42EB, 0x4344, 0x439C,
    0x43F4, 0x444C, 0x44A3, 0x44FA, 0x4550, 0x45A6, 0x45FC, 0x4651,
    0x46A6, 0x46FA, 0x474E, 0x47A1, 0x47F4, 0x4846, 0x4898, 0x48E9,
    0x493A, 0x498A, 0x49D9, 0x4A29, 0x4A77, 0x4AC5, 0x4B13, 0x4B5F,
    0x4BAC, 0x4BF7, 0x4C42, 0x4C8D, 0x4CD7, 0x4D20, 0x4D68, 0x4DB0,
    0x4DF7, 0x4E3E, 0x4E84, 0x4EC9, 0x4F0E, 0x4F52, 0x4F95, 0x4FD7,
    0x5019, 0x505A, 0x509A, 0x50DA, 0x5118, 0x5156, 0x5194, 0x51D0,
    0x520C, 0x5247, 0x5281, 0x52BA, 0x52F3, 0x532A, 0x5361, 0x5397,
    0x53CC, 0x5401, 0x5434, 0x5467, 0x5499, 0x54CA, 0x54FA, 0x5529,
    0x5558, 0x5585, 0x55B2, 0x55DE, 0x5609, 0x5632, 0x565B, 0x5684,
    0x56AB, 0x56D1, 0x56F6, 0x571B, 0x573E, 0x5761, 0x5782, 0x57A3,
    0x57C3, 0x57E2, 0x57FF, 0x581C, 0x5838, 0x5853, 0x586D, 0x5886,
    0x589E, 0x58B5, 0x58CB, 0x58E0, 0x58F4, 0x5907, 0x5919, 0x592A,
    0x593A, 0x5949, 0x5958, 0x5965, 0x5971, 0x597C, 0x5986, 0x598F,
    0x5997, 0x599E, 0x59A4, 0x59A9, 0x59AD, 0x59B0, 0x59B2, 0x59B3,
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
        if let Some(wrap_end) = addr
            .checked_add(size)
            .and_then(|e| e.checked_sub(self.ram_size))
        {
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
    pub const fn fixed_volume_i32(self) -> i32 {
        (((self.0 << 1) as i16) as i32) << 16
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
    pub adsr_envx: i32,
    pub adsr_cycles: i32,
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
            self.volume_left_abs = self.volume_left.fixed_volume_i32();
        }
    }

    #[inline]
    pub fn set_volume_right(&mut self, value: u16) {
        self.volume_right = ChannelVolume(value);
        if !self.volume_right.is_sweep() {
            self.volume_right_abs = self.volume_right.fixed_volume_i32();
        }
    }
}

const GAUSS_HISTORY: usize = 3;

#[derive(Clone, Copy, Default)]
struct HalfBlockMeta {
    addr: u32,
    end_flag: bool,
    done: bool,
    repeat_set: bool,
    repeat_addr: u32,
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
    latched_repeat_addr: u32,
    buffer: [i16; GAUSS_HISTORY + BUFFER_SAMPLES * 2],
    cur_meta: HalfBlockMeta,
    next_meta: HalfBlockMeta,
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
            latched_repeat_addr: 0,
            buffer: [0; GAUSS_HISTORY + BUFFER_SAMPLES * 2],
            cur_meta: HalfBlockMeta::default(),
            next_meta: HalfBlockMeta::default(),
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
        self.latched_repeat_addr = 0;
        self.buffer = [0; GAUSS_HISTORY + BUFFER_SAMPLES * 2];
        self.cur_meta = HalfBlockMeta::default();
        self.next_meta = HalfBlockMeta::default();
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
        self.latched_repeat_addr = self.repeat_addr;
        self.cur_meta = HalfBlockMeta {
            addr: self.next_sample_addr,
            ..HalfBlockMeta::default()
        };
        self.next_meta = HalfBlockMeta::default();
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
        self.buffer = [0; GAUSS_HISTORY + BUFFER_SAMPLES * 2];
    }

    fn set_pitch(&mut self, base_sampling_rate: u32, pitch: u16) {
        self.src_sampling_rate = base_sampling_rate.wrapping_mul(pitch.min(0x3FFF) as u32);
        self.update_sample_step();
    }

    fn update_sample_step(&mut self) {
        if let Some(step) = self.src_sampling_rate.checked_div(self.dst_sampling_rate) {
            self.sample_step = step;
        }
    }

    #[inline]
    fn step_for_pitch(&self, base_sampling_rate: u32, pitch: u16) -> u32 {
        base_sampling_rate
            .wrapping_mul(pitch.min(0x3FFF) as u32)
            .checked_div(self.dst_sampling_rate)
            .unwrap_or(0)
    }

    #[inline]
    fn get_sample(
        &mut self,
        step_override: Option<u32>,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) -> i32 {
        let src_sample_idx = (self.src_sample_idx / PITCH_BASE) as usize;
        let gauss_idx = ((self.src_sample_idx >> 4) & 0xFF) as usize;
        let window = &self.buffer[src_sample_idx..src_sample_idx + 4];
        let s0 = window[0] as i32; // oldest
        let s1 = window[1] as i32; // older
        let s2 = window[2] as i32; // old
        let s3 = window[3] as i32; // new
        let g0 = GAUSS_TABLE[0x0FF - gauss_idx];
        let g1 = GAUSS_TABLE[0x1FF - gauss_idx];
        let g2 = GAUSS_TABLE[0x100 + gauss_idx];
        let g3 = GAUSS_TABLE[gauss_idx];
        // Hardware Gaussian interpolation: full-precision 4-tap MAC then >> 15.
        let out = clamp16((g0 * s0 + g1 * s1 + g2 * s2 + g3 * s3) >> 15);

        let step = step_override.unwrap_or(self.sample_step);
        let mut next_idx = (self.src_sample_idx as u64) + (step as u64);
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
        out
    }

    fn advance_buffer(
        &mut self,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) {
        if self.next_valid {
            // Commit end/done flags from the block that just finished playing.
            if self.cur_meta.end_flag {
                self.end_flag = true;
            }
            if self.cur_meta.done {
                self.done = true;
            }
            self.buffer
                .copy_within(BUFFER_SAMPLES..(GAUSS_HISTORY + BUFFER_SAMPLES * 2), 0);
            self.cur_meta = self.next_meta;
            if self.cur_meta.repeat_set {
                self.latched_repeat_addr = self.cur_meta.repeat_addr;
                self.did_change_repeat = true;
            }
            self.next_meta =
                self.unpack_into_half(GAUSS_HISTORY + BUFFER_SAMPLES, ram, cache, irq_watcher);
        } else {
            self.buffer[..GAUSS_HISTORY].fill(0);
            self.cur_meta = self.unpack_into_half(GAUSS_HISTORY, ram, cache, irq_watcher);
            if self.cur_meta.repeat_set {
                self.latched_repeat_addr = self.cur_meta.repeat_addr;
                self.did_change_repeat = true;
            }
            self.next_meta =
                self.unpack_into_half(GAUSS_HISTORY + BUFFER_SAMPLES, ram, cache, irq_watcher);
            self.next_valid = true;
        }
    }

    fn unpack_into_half(
        &mut self,
        dst_offset: usize,
        ram: &[u8],
        cache: &mut SpuSampleCache,
        irq_watcher: &mut SpuIrqWatcher,
    ) -> HalfBlockMeta {
        let mask = (self.ram_size - 1) as usize;
        let block_addr = self.next_sample_addr;
        let base_addr = (block_addr as usize) & mask;

        irq_watcher.check_irq(block_addr);

        let header = ram[base_addr];
        let flags = ram[(base_addr + 1) & mask];
        let mut shift_factor = (header & 0x0F) as u32;
        if shift_factor > 12 {
            shift_factor = 9;
        }
        let predict_number = (header >> 4) as usize;

        let mut cache_key = CacheKey {
            address: block_addr,
            s1: self.s1,
            s2: self.s2,
        };
        if predict_number == 0 || predict_number >= 5 {
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

                let sample0 = clamp16(
                    ((first_sample as i32) >> shift_factor) + ((s1 * k0) >> 6) + ((s2 * k1) >> 6),
                );
                s2 = s1;
                s1 = sample0;
                out[0] = sample0 as i16;

                let sample1 = clamp16(
                    ((second_sample as i32) >> shift_factor) + ((s1 * k0) >> 6) + ((s2 * k1) >> 6),
                );
                s2 = s1;
                s1 = sample1;
                out[1] = sample1 as i16;
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

        let mut meta = HalfBlockMeta {
            addr: block_addr,
            end_flag: false,
            done: false,
            repeat_set: false,
            repeat_addr: self.repeat_addr,
        };

        if (flags & 0x04) != 0 {
            self.repeat_addr = block_addr;
            meta.repeat_set = true;
            meta.repeat_addr = block_addr;
        }

        self.next_sample_addr = block_addr.wrapping_add(0x10) & (self.ram_size - 1);

        if (flags & 0x01) != 0 {
            meta.end_flag = true;
            self.next_sample_addr = self.repeat_addr;
            if (flags & 0x03) != 0x03 {
                meta.done = true;
            }
        }

        meta
    }

    fn repeat(&self) -> u32 {
        self.latched_repeat_addr
    }

    fn set_repeat(&mut self, repeat_addr: u32) {
        self.repeat_addr = repeat_addr & (self.ram_size - 1);
    }

    fn current(&self) -> u32 {
        let intra_sample_idx = ((self.src_sample_idx / PITCH_BASE) / 2).min(0x0E);
        self.cur_meta.addr.wrapping_add(intra_sample_idx)
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
        self.block_buffer
            .copy_from_slice(&block[..SOUND_INPUT_DATA_SIZE]);
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
    channel_pmon: u32,
    channel_noise: u32,
    noise_divider: u32,
    noise_counter: u32,
    noise_level: i16,
    reverb_work_addr_start: u32,
    reverb_work_addr_end: u32,
    reverb_curr_addr: u32,
    ctrl: u16,
    reverb_ticks: i32,
    reverb: [u32; REVERB_PARAM_COUNT],
    reverb_in_buf: [[i16; 64]; 2],
    reverb_in_pos: usize,
    reverb_out_buf: [[i16; 32]; 2],
    reverb_out_pos: [usize; 2],

    ps1_capture: bool,
    capture_offset: u32,

    channels: [Channel; MAX_CHANNEL],
    readers: [SampleReader; MAX_CHANNEL],

    block_reader: BlockSampleReader,
    sound_input_data_addr: u32,
    block_write_ptr: u32,

    pub master_vol_l: ChannelVolume,
    pub master_vol_r: ChannelVolume,
    pub master_vol_l_abs: i32,
    pub master_vol_r_abs: i32,
    pub reverb_vol_l: i16,
    pub reverb_vol_r: i16,
    pub input_vol_l: i32,
    pub input_vol_r: i32,
    pub ext_input_vol_l: i32,
    pub ext_input_vol_r: i32,
}

const DEFAULT_MASTER_VOL: u16 = 0x3FFF;
const DEFAULT_REVERB_VOL: i16 = 0x2AAA;

impl SpuBase {
    pub fn new(spu_number: usize, ram_size: u32) -> Self {
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
            channel_reverb: 0,
            channel_pmon: 0,
            channel_noise: 0,
            noise_divider: 0,
            noise_counter: 0,
            noise_level: 0,
            reverb_work_addr_start: 0,
            reverb_work_addr_end: 0x80000,
            reverb_curr_addr: 0,
            ctrl: 0,
            reverb_ticks: 0,
            reverb: [0; REVERB_PARAM_COUNT],
            reverb_in_buf: [[0; 64]; 2],
            reverb_in_pos: 0,
            reverb_out_buf: [[0; 32]; 2],
            reverb_out_pos: [0; 2],
            ps1_capture: false,
            capture_offset: 0,
            channels: [Channel::default(); MAX_CHANNEL],
            readers: std::array::from_fn(|_| SampleReader::new(ram_size)),
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

    pub fn reset(&mut self) {
        self.ctrl = 0;
        self.channel_reverb = 0;
        self.channel_pmon = 0;
        self.channel_noise = 0;
        self.noise_divider = 0;
        self.noise_counter = 0;
        self.noise_level = 0;
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
        self.reverb = [0; REVERB_PARAM_COUNT];
        self.reverb_in_buf = [[0; 64]; 2];
        self.reverb_in_pos = 0;
        self.reverb_out_buf = [[0; 32]; 2];
        self.reverb_out_pos = [0; 2];
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

    pub fn set_ps1_capture(&mut self, enabled: bool) {
        self.ps1_capture = enabled;
    }

    #[inline]
    pub fn capture_second_half(&self) -> bool {
        (self.capture_offset & (CAPTURE_BUFFER_SIZE / 2)) != 0
    }

    #[inline]
    pub fn set_master_vol_l(&mut self, value: u16) {
        self.master_vol_l = ChannelVolume(value);
        if !self.master_vol_l.is_sweep() {
            self.master_vol_l_abs = self.master_vol_l.fixed_volume_i32();
        }
    }

    #[inline]
    pub fn set_master_vol_r(&mut self, value: u16) {
        self.master_vol_r = ChannelVolume(value);
        if !self.master_vol_r.is_sweep() {
            self.master_vol_r_abs = self.master_vol_r.fixed_volume_i32();
        }
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

    pub fn channel_pmon(&self) -> u32 {
        self.channel_pmon
    }

    pub fn set_channel_pmon_lo(&mut self, value: u16) {
        // Voice 0 cannot be pitch-modulated on PS1/PS2 hardware.
        self.channel_pmon = (self.channel_pmon & 0x00FF_0000) | ((value & 0xFFFE) as u32);
    }

    pub fn set_channel_pmon_hi(&mut self, value: u16) {
        self.channel_pmon = (self.channel_pmon & 0x0000_FFFF) | (((value & 0x00FF) as u32) << 16);
    }

    pub fn channel_noise(&self) -> u32 {
        self.channel_noise
    }

    pub fn set_channel_noise_lo(&mut self, value: u16) {
        self.channel_noise = (self.channel_noise & 0x00FF_0000) | (value as u32);
    }

    pub fn set_channel_noise_hi(&mut self, value: u16) {
        self.channel_noise = (self.channel_noise & 0x0000_FFFF) | (((value & 0x00FF) as u32) << 16);
    }

    pub fn set_reverb_vol_l(&mut self, value: i16) {
        self.reverb_vol_l = value;
    }

    pub fn set_reverb_vol_r(&mut self, value: i16) {
        self.reverb_vol_r = value;
    }

    #[inline]
    pub(crate) fn reverb_param(&self, param: ReverbParam) -> u32 {
        self.reverb[param as usize]
    }

    pub(crate) fn set_reverb_param(&mut self, param: ReverbParam, value: u32) {
        self.reverb[param as usize] = value;
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
            ch.status = EnvelopePhase::KeyOn;
            ch.repeat_set = false;
            ch.current = ch.address;
            ch.adsr_envx = 0;
            ch.adsr_cycles = 0;
            self.readers[i].clear_end_flag();
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
                    ch.adsr_cycles = 0;
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
    }

    pub fn reverb_work_address_end(&self) -> u32 {
        self.reverb_work_addr_end
    }

    pub fn set_reverb_work_address_end(&mut self, address: u32) {
        self.reverb_work_addr_end = address;
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
        let scaled = input_sample.wrapping_mul(volume_level) >> 15;
        let result = clamp16(scaled.wrapping_add(*output as i32));
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
        let unmuted = (self.ctrl & CONTROL_UNMUTE) != 0;

        // Build active-voice bitmask; sync inert voices once outside the hot loop.
        let mut active_mask = 0u32;
        for i in 0..MAX_CHANNEL {
            let ch = &mut self.channels[i];
            let reader = &mut self.readers[i];
            if ch.status == EnvelopePhase::Stopped
                && ch.adsr_envx == 0
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
            && !self.ps1_capture
            && irq_enabled
            && (self.irq_addr < CORE0_OUTPUT_SIZE
                || (CORE1_SIN_LEFT..CORE1_SIN_LEFT + CORE0_OUTPUT_SIZE).contains(&self.irq_addr)
                || (CORE1_SIN_RIGHT..CORE1_SIN_RIGHT + CORE0_OUTPUT_SIZE).contains(&self.irq_addr));

        for frame in samples.chunks_exact_mut(2) {
            let mut out_l: i16 = 0;
            let mut out_r: i16 = 0;
            let mut reverb_sample = [0i16; 2];
            let mut voice_out = [0i16; MAX_CHANNEL];

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
                    if reader.did_change_repeat() && !ch.repeat_set {
                        ch.repeat = reader.repeat();
                        reader.clear_did_change_repeat();
                    }
                    reader.set_repeat(ch.repeat);
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
                ch.current = reader.current();
                let read_sample = if (self.channel_noise & (1 << i)) != 0 {
                    self.noise_level as i32
                } else {
                    adpcm_sample
                };

                // Apply current ENVX to this frame's sample before ticking ADSR for the next frame.
                let envx = ch.adsr_envx;
                Self::update_adsr(ch);
                let vol_l = Self::compute_channel_volume(ch.volume_left, ch.volume_left_abs);
                let vol_r = Self::compute_channel_volume(ch.volume_right, ch.volume_right_abs);
                ch.volume_left_abs = vol_l;
                ch.volume_right_abs = vol_r;

                if read_sample == 0 {
                    continue;
                }

                let input_sample = read_sample.wrapping_mul(envx) >> 15;
                voice_out[i] = clamp16(input_sample) as i16;

                if input_sample == 0 {
                    continue;
                }

                let volume_left = vol_l >> 16;
                let volume_right = vol_r >> 16;

                if unmuted {
                    Self::mix_samples(input_sample, volume_left, &mut out_l);
                    Self::mix_samples(input_sample, volume_right, &mut out_r);
                }

                if update_reverb && ((self.channel_reverb & (1 << i)) != 0) {
                    Self::mix_samples(input_sample, volume_left, &mut reverb_sample[0]);
                    Self::mix_samples(input_sample, volume_right, &mut reverb_sample[1]);
                }
            }

            self.step_noise();

            if self.ps1_capture {
                self.write_ps1_capture(voice_out[1], voice_out[3], ram, cache, irq_watcher);
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
                self.update_reverb(reverb_sample, unmuted, &mut out_l, &mut out_r, ram);
            }

            self.master_vol_l_abs =
                Self::compute_channel_volume(self.master_vol_l, self.master_vol_l_abs);
            self.master_vol_r_abs =
                Self::compute_channel_volume(self.master_vol_r, self.master_vol_r_abs);
            let master_l = self.master_vol_l_abs >> 16;
            let master_r = self.master_vol_r_abs >> 16;

            frame[0] = Self::scale_master(out_l, master_l);
            frame[1] = Self::scale_master(out_r, master_r);
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
            cache.clear_range(addr as u32, 2);
            irq_watcher.check_irq(addr as u32);
        }
        self.capture_offset = (self.capture_offset + 2) & (CAPTURE_BUFFER_SIZE - 2);
    }

    #[inline]
    fn scale_master(sample: i16, master_vol: i32) -> i16 {
        clamp16((sample as i32).wrapping_mul(master_vol) >> 15) as i16
    }

    #[inline]
    fn step_noise(&mut self) {
        let rf = ((self.ctrl >> 8) & 0x3F) as u32;
        let (div_inc, cnt_inc) = if rf >= 0x3C {
            (0x8000u32, 8u32)
        } else {
            let shift = rf >> 2;
            let step = rf & 0x03;
            (2u32 << shift, 4 + step)
        };

        self.noise_divider += div_inc;
        if self.noise_divider >= 0x8000 {
            self.noise_divider -= 0x8000;
            self.noise_counter += cnt_inc;
            if self.noise_counter >= 8 {
                self.noise_counter -= 8;
                let l = self.noise_level as u16;
                let parity = ((l >> 15) ^ (l >> 12) ^ (l >> 11) ^ (l >> 10) ^ 1) & 1;
                self.noise_level = ((l << 1) | parity) as i16;
            }
        }
    }

    #[inline]
    fn compute_channel_volume(volume: ChannelVolume, current_volume: i32) -> i32 {
        if !volume.is_sweep() {
            volume.fixed_volume_i32()
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

    /// Exact hardware 15-bit discrete cycle-counter ADSR envelope generator
    /// (per Nocash PSX-SPX specification).
    #[inline]
    fn update_adsr(ch: &mut Channel) {
        let (shift, base_step, decreasing, exponential) = match ch.status {
            EnvelopePhase::Stopped | EnvelopePhase::KeyOn => return,
            EnvelopePhase::Attack => {
                let rate = ch.adsr_level.attack_rate();
                let shift = rate >> 2;
                let step_idx = (rate & 3) as i32;
                (
                    shift,
                    7 - step_idx,
                    false,
                    ch.adsr_level.attack_exponential(),
                )
            }
            EnvelopePhase::Decay => (ch.adsr_level.decay_rate(), -8, true, true),
            EnvelopePhase::Sustain => {
                let rate = ch.adsr_rate.sustain_rate();
                let shift = rate >> 2;
                let step_idx = (rate & 3) as i32;
                let dec = ch.adsr_rate.sustain_decrease();
                let base = if dec { -(8 - step_idx) } else { 7 - step_idx };
                (shift, base, dec, ch.adsr_rate.sustain_exponential())
            }
            EnvelopePhase::Release => (
                ch.adsr_rate.release_rate(),
                -8,
                true,
                ch.adsr_rate.release_exponential(),
            ),
        };

        let mut adsr_cycles: i32 = 1i32 << shift.saturating_sub(11);
        let mut adsr_step: i32 = base_step << 11u32.saturating_sub(shift);

        if exponential {
            if !decreasing {
                if ch.adsr_envx >= 0x6000 {
                    if shift < 10 {
                        adsr_step >>= 2;
                    } else if shift == 10 {
                        adsr_step >>= 1;
                        adsr_cycles = adsr_cycles.saturating_mul(2);
                    } else {
                        adsr_cycles = adsr_cycles.saturating_mul(4);
                    }
                }
            } else {
                adsr_step = (adsr_step * ch.adsr_envx) >> 15;
            }
        }

        if ch.adsr_cycles <= 0 {
            ch.adsr_cycles = adsr_cycles;
        }
        ch.adsr_cycles = ch.adsr_cycles.wrapping_sub(1);
        if ch.adsr_cycles > 0 {
            return;
        }

        let next = (ch.adsr_envx + adsr_step).clamp(0, MAX_ADSR_LEVEL);
        ch.adsr_envx = next;

        match ch.status {
            EnvelopePhase::Attack => {
                if next >= MAX_ADSR_LEVEL {
                    ch.status = EnvelopePhase::Decay;
                    ch.adsr_cycles = 0;
                }
            }
            EnvelopePhase::Decay => {
                let sustain_target = ((ch.adsr_level.sustain_level() + 1) * 0x800) as i32;
                if next < sustain_target {
                    ch.status = EnvelopePhase::Sustain;
                    ch.adsr_cycles = 0;
                }
            }
            EnvelopePhase::Release => {
                if next <= 0 {
                    ch.status = EnvelopePhase::Stopped;
                    ch.adsr_cycles = 0;
                }
            }
            EnvelopePhase::Sustain | EnvelopePhase::Stopped | EnvelopePhase::KeyOn => {}
        }
    }

    #[inline]
    fn reverb_ram_addr(&self, offset: i64) -> usize {
        let work_len = (self.reverb_work_addr_end - self.reverb_work_addr_start) as i64;
        let rel = (self.reverb_curr_addr - self.reverb_work_addr_start) as i64;
        let wrapped = (rel + offset).rem_euclid(work_len) as u32;
        let absolute = self.reverb_work_addr_start + wrapped;
        (absolute & (self.ram_size - 1)) as usize
    }

    #[inline]
    fn get_reverb_sample(&self, offset: i64, ram: &[u8]) -> i32 {
        let mask = (self.ram_size - 1) as usize;
        let off = self.reverb_ram_addr(offset);
        i16::from_le_bytes([ram[off], ram[(off + 1) & mask]]) as i32
    }

    #[inline]
    fn set_reverb_sample(&self, offset: i64, value: i32, ram: &mut [u8]) {
        let mask = (self.ram_size - 1) as usize;
        let off = self.reverb_ram_addr(offset);
        let bytes = (clamp16(value) as i16).to_le_bytes();
        ram[off] = bytes[0];
        ram[(off + 1) & mask] = bytes[1];
    }

    #[inline]
    fn get_reverb_offset(&self, param: ReverbParam) -> i64 {
        self.reverb[param as usize] as i64
    }

    #[inline]
    fn get_reverb_coef(&self, param: ReverbParam) -> i32 {
        (self.reverb[param as usize] as i16) as i32
    }

    /// 39-tap symmetric half-band FIR downsample (44.1/48 kHz -> 22.05/24 kHz).
    #[inline]
    fn reverb_fir_downsample(&self, ch: usize) -> i32 {
        let buf = &self.reverb_in_buf[ch];
        let pos = self.reverb_in_pos;
        let mut acc = REVERB_FIR_CENTER_TAP * (buf[pos.wrapping_sub(19) & 63] as i32);
        for (j, &tap) in REVERB_FIR_ODD_TAPS.iter().enumerate() {
            let a = buf[pos.wrapping_sub(38 - j * 2) & 63] as i32;
            let b = buf[pos.wrapping_sub(j * 2) & 63] as i32;
            acc += tap * (a + b);
        }
        clamp16(acc >> 15)
    }

    /// 39-tap symmetric half-band zero-stuffed FIR upsample (22.05/24 kHz -> 44.1/48 kHz).
    #[inline]
    fn reverb_fir_upsample(&self, ch: usize, is_active: bool) -> i32 {
        let buf = &self.reverb_out_buf[ch];
        let pos = self.reverb_out_pos[ch];
        if is_active {
            let mut acc = 0i32;
            for (j, &tap) in REVERB_FIR_ODD_TAPS.iter().enumerate() {
                let a = buf[pos.wrapping_sub(19 - j) & 31] as i32;
                let b = buf[pos.wrapping_sub(j) & 31] as i32;
                acc += tap * (a + b);
            }
            clamp16(acc >> 14)
        } else {
            buf[pos.wrapping_sub(9) & 31] as i32
        }
    }

    #[inline]
    const fn neg16(v: i32) -> i32 {
        clamp16(-v)
    }

    /// Hardware IIR `(1 - IIR_ALPHA) * sample` term in Q14 (pre-`>> 1`).
    /// Handles `IIR_ALPHA == -32768` via saturating negation so `-(-32768) -> 0x7FFF`.
    #[inline]
    const fn reverb_iiasm(iir_alpha: i32, sample: i32) -> i32 {
        (sample << 1) + ((sample * Self::neg16(iir_alpha)) >> 14)
    }

    /// Runs one channel (0 = Left on odd cycles, 1 = Right on even cycles) of the
    /// hardware 16-bit fixed-point reverb engine.
    fn step_reverb_channel(&mut self, ch: usize, ram: &mut [u8]) -> i16 {
        let in_s = self.reverb_fir_downsample(ch);

        let v_in = self.get_reverb_coef(if ch == 0 {
            ReverbParam::InCoefL
        } else {
            ReverbParam::InCoefR
        });
        let v_wall = self.get_reverb_coef(ReverbParam::IirCoef);
        let v_iir = self.get_reverb_coef(ReverbParam::IirAlpha);

        let (m_same, d_same, m_diff, d_diff) = if ch == 0 {
            (
                self.get_reverb_offset(ReverbParam::IirDestA0),
                self.get_reverb_offset(ReverbParam::IirSrcA0),
                self.get_reverb_offset(ReverbParam::IirDestB0),
                self.get_reverb_offset(ReverbParam::IirSrcB1),
            )
        } else {
            (
                self.get_reverb_offset(ReverbParam::IirDestA1),
                self.get_reverb_offset(ReverbParam::IirSrcA1),
                self.get_reverb_offset(ReverbParam::IirDestB1),
                self.get_reverb_offset(ReverbParam::IirSrcB0),
            )
        };

        let in_term = (in_s * v_in) >> 14;

        // Same-side reflection (IIR).
        let iir_in_same =
            clamp16((((self.get_reverb_sample(d_same, ram) * v_wall) >> 14) + in_term) >> 1);
        let prev_same = self.get_reverb_sample(m_same - 2, ram);
        let iir_same =
            clamp16((((iir_in_same * v_iir) >> 14) + Self::reverb_iiasm(v_iir, prev_same)) >> 1);
        self.set_reverb_sample(m_same, iir_same, ram);

        // Cross-coupled different-side reflection (IIR).
        let iir_in_diff =
            clamp16((((self.get_reverb_sample(d_diff, ram) * v_wall) >> 14) + in_term) >> 1);
        let prev_diff = self.get_reverb_sample(m_diff - 2, ram);
        let iir_diff =
            clamp16((((iir_in_diff * v_iir) >> 14) + Self::reverb_iiasm(v_iir, prev_diff)) >> 1);
        self.set_reverb_sample(m_diff, iir_diff, ram);

        // Early echo (4-tap comb filter): unclamped Q14 accumulator folded directly into APF1.
        let v_comb1 = self.get_reverb_coef(ReverbParam::AccCoefA);
        let v_comb2 = self.get_reverb_coef(ReverbParam::AccCoefB);
        let v_comb3 = self.get_reverb_coef(ReverbParam::AccCoefC);
        let v_comb4 = self.get_reverb_coef(ReverbParam::AccCoefD);

        let (src_a, src_b, src_c, src_d) = if ch == 0 {
            (
                ReverbParam::AccSrcA0,
                ReverbParam::AccSrcB0,
                ReverbParam::AccSrcC0,
                ReverbParam::AccSrcD0,
            )
        } else {
            (
                ReverbParam::AccSrcA1,
                ReverbParam::AccSrcB1,
                ReverbParam::AccSrcC1,
                ReverbParam::AccSrcD1,
            )
        };

        let comb_acc = ((v_comb1 * self.get_reverb_sample(self.get_reverb_offset(src_a), ram))
            >> 14)
            + ((v_comb2 * self.get_reverb_sample(self.get_reverb_offset(src_b), ram)) >> 14)
            + ((v_comb3 * self.get_reverb_sample(self.get_reverb_offset(src_c), ram)) >> 14)
            + ((v_comb4 * self.get_reverb_sample(self.get_reverb_offset(src_d), ram)) >> 14);

        // Late reverb APF1 (cascaded into APF2).
        let d_apf1 = self.get_reverb_offset(ReverbParam::FbSrcA);
        let d_apf2 = self.get_reverb_offset(ReverbParam::FbSrcB);
        let (m_apf1, m_apf2) = if ch == 0 {
            (
                self.get_reverb_offset(ReverbParam::MixDestA0),
                self.get_reverb_offset(ReverbParam::MixDestB0),
            )
        } else {
            (
                self.get_reverb_offset(ReverbParam::MixDestA1),
                self.get_reverb_offset(ReverbParam::MixDestB1),
            )
        };
        let v_apf1 = self.get_reverb_coef(ReverbParam::FbAlpha);
        let v_apf2 = self.get_reverb_coef(ReverbParam::FbX);

        let fb_a = self.get_reverb_sample(m_apf1 - d_apf1, ram);
        let mda = clamp16((comb_acc + ((fb_a * Self::neg16(v_apf1)) >> 14)) >> 1);
        self.set_reverb_sample(m_apf1, mda, ram);

        let apf1_out_q14 = (fb_a << 1) + ((mda * v_apf1) >> 14);

        // Late reverb APF2.
        let fb_b = self.get_reverb_sample(m_apf2 - d_apf2, ram);
        let mdb = clamp16((apf1_out_q14 + ((fb_b * Self::neg16(v_apf2)) >> 14)) >> 1);
        self.set_reverb_sample(m_apf2, mdb, ram);

        clamp16(((fb_b << 1) + ((mdb * v_apf2) >> 14)) >> 1) as i16
    }

    /// Hardware 16-bit saturating fixed-point reverb engine with alternating L/R
    /// 44.1 kHz cycles and 39-tap half-band FIR downsample/upsample.
    fn update_reverb(
        &mut self,
        reverb_sample: [i16; 2],
        unmuted: bool,
        out_l: &mut i16,
        out_r: &mut i16,
        ram: &mut [u8],
    ) {
        self.reverb_in_pos = (self.reverb_in_pos + 1) & 63;
        self.reverb_in_buf[0][self.reverb_in_pos] = reverb_sample[0];
        self.reverb_in_buf[1][self.reverb_in_pos] = reverb_sample[1];

        let is_odd = (self.reverb_ticks & 1) != 0;
        if is_odd {
            let wet_22k_l = self.step_reverb_channel(0, ram);
            self.reverb_out_pos[0] = (self.reverb_out_pos[0] + 1) & 31;
            self.reverb_out_buf[0][self.reverb_out_pos[0]] = wet_22k_l;
        } else {
            let wet_22k_r = self.step_reverb_channel(1, ram);
            self.reverb_out_pos[1] = (self.reverb_out_pos[1] + 1) & 31;
            self.reverb_out_buf[1][self.reverb_out_pos[1]] = wet_22k_r;

            self.reverb_curr_addr = self.reverb_curr_addr.wrapping_add(2);
            if self.reverb_curr_addr >= self.reverb_work_addr_end {
                self.reverb_curr_addr = self.reverb_work_addr_start;
            }
        }

        if unmuted {
            let up_l = self.reverb_fir_upsample(0, is_odd);
            let up_r = self.reverb_fir_upsample(1, !is_odd);
            let wet_l = (up_l * (self.reverb_vol_l as i32)) >> 15;
            let wet_r = (up_r * (self.reverb_vol_r as i32)) >> 15;
            *out_l = clamp16((*out_l as i32) + wet_l) as i16;
            *out_r = clamp16((*out_r as i32) + wet_r) as i16;
        }

        self.reverb_ticks = self.reverb_ticks.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gauss_table_unity_dc_gain_and_no_overflow() {
        for i in 0..256usize {
            let g0 = GAUSS_TABLE[0x0FF - i];
            let g1 = GAUSS_TABLE[0x1FF - i];
            let g2 = GAUSS_TABLE[0x100 + i];
            let g3 = GAUSS_TABLE[i];
            let sum = g0 + g1 + g2 + g3;
            assert!(
                (0x7F7F..=0x7F81).contains(&sum),
                "gauss sum at i={i:#x} was {sum:#x}"
            );

            // Verify zero overflow for all extreme full-scale i16::MIN/MAX combinations.
            let extremes = [i16::MIN as i32, i16::MAX as i32];
            for &s0 in &extremes {
                for &s1 in &extremes {
                    for &s2 in &extremes {
                        for &s3 in &extremes {
                            let acc = g0
                                .checked_mul(s0)
                                .and_then(|v| v.checked_add(g1.checked_mul(s1)?))
                                .and_then(|v| v.checked_add(g2.checked_mul(s2)?))
                                .and_then(|v| v.checked_add(g3.checked_mul(s3)?))
                                .expect("gaussian accumulation must not overflow i32");
                            let out = acc >> 15;
                            assert!(
                                (i16::MIN as i32..=i16::MAX as i32).contains(&out),
                                "gaussian output {out} out of i16 range at i={i:#x}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn gauss_impulse_response_is_smooth_and_symmetric() {
        // Place a single impulse at sample index 10 in a silent buffer and sweep frac across 4 samples.
        let mut buf = [0i16; 16];
        buf[10] = 16384;
        let eval = |pos_q12: usize| -> i32 {
            let idx = pos_q12 >> 12;
            let frac = (pos_q12 >> 4) & 0xFF;
            let s0 = buf[idx] as i32;
            let s1 = buf[idx + 1] as i32;
            let s2 = buf[idx + 2] as i32;
            let s3 = buf[idx + 3] as i32;
            (GAUSS_TABLE[0x0FF - frac] * s0
                + GAUSS_TABLE[0x1FF - frac] * s1
                + GAUSS_TABLE[0x100 + frac] * s2
                + GAUSS_TABLE[frac] * s3)
                >> 15
        };

        // Peak occurs when the impulse sits on tap s1 (idx = 9, frac = 0) with weight gauss[0x1FF].
        let peak = eval(9 << 12);
        assert_eq!(peak, (16384 * GAUSS_TABLE[0x1FF]) >> 15);
        assert!(peak > 11_000);

        // Exact half-sample symmetry around the center between (8, frac=255) and (9, frac=0),
        // covering both the inner bell (s2 vs s1) and outer tails (s3 vs s0).
        for step in 0..256usize {
            let inner_l = eval((8 << 12) + ((255 - step) << 4));
            let inner_r = eval((9 << 12) + (step << 4));
            assert_eq!(inner_l, inner_r, "inner symmetry mismatch at step={step}");
            assert!(inner_l <= peak);

            let outer_l = eval((7 << 12) + ((255 - step) << 4));
            let outer_r = eval((10 << 12) + (step << 4));
            assert_eq!(outer_l, outer_r, "outer symmetry mismatch at step={step}");
            assert!(outer_l <= inner_l);
        }
    }

    #[test]
    fn cxd2922q_gauss_per_term_shift_wrap_and_saturate() {
        // Verify hardware-capture Gaussian MAC: full-precision 4-tap sum, >> 15, clamp16.
        let mut reader = SampleReader::new(0x80000);
        reader.set_destination_sampling_rate(44_100);
        reader.set_pitch(44_100, 0x1000);
        reader.buffer[0] = -32768;
        reader.buffer[1] = -32768;
        reader.buffer[2] = -32768;
        reader.buffer[3] = -32768;
        reader.src_sample_idx = 0x0080; // gauss_idx = 8
        let ram = vec![0u8; 0x80000];
        let mut cache = SpuSampleCache::new(0x80000);
        let mut irq = SpuIrqWatcher::new();
        let got = reader.get_sample(None, &ram, &mut cache, &mut irq);

        let gi = 8usize;
        let s = -32768i32;
        let expected = clamp16(
            (GAUSS_TABLE[0x0FF - gi] * s
                + GAUSS_TABLE[0x1FF - gi] * s
                + GAUSS_TABLE[0x100 + gi] * s
                + GAUSS_TABLE[gi] * s)
                >> 15,
        );
        assert_eq!(got, expected);
    }

    #[test]
    fn reverb_39tap_halfband_fir_unity_dc_gain() {
        let mut base = SpuBase::new(0, 0x80000);
        // Feed constant DC 16000 into the downsample ring buffer.
        for _ in 0..64 {
            base.reverb_in_pos = (base.reverb_in_pos + 1) & 63;
            base.reverb_in_buf[0][base.reverb_in_pos] = 16_000;
        }
        let down = base.reverb_fir_downsample(0);
        // Sum of all 39 taps = 0x7FFE / 0x8000 => 16000 * 32766 >> 15 = 15999.
        assert_eq!(down, (16_000 * 0x7FFE) >> 15);

        // Feed constant DC 16000 into the upsample ring buffer.
        for _ in 0..32 {
            base.reverb_out_pos[0] = (base.reverb_out_pos[0] + 1) & 31;
            base.reverb_out_buf[0][base.reverb_out_pos[0]] = 16_000;
        }
        let up_even = base.reverb_fir_upsample(0, false);
        let up_odd = base.reverb_fir_upsample(0, true);
        assert_eq!(up_even, 16_000);
        assert_eq!(up_odd, (16_000 * 2 * 0x1FFF) >> 14);
    }
}
