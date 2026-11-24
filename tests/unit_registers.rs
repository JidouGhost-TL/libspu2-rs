// SPDX-License-Identifier: BSD-2-Clause

use libspu2::registers::{self, core_reg, CONTROL_DMA_WRITE, VOICE_ADDR_STRIDE, VOICE_STRIDE};
use libspu2::Device;

#[test]
fn register_constants_match_hardware_offsets() {
    assert_eq!(registers::VP_PITCH, 0x004);
    assert_eq!(registers::CORE_ATTR, 0x19A);
    assert_eq!(registers::A_KON_HI, 0x1A0);
    assert_eq!(registers::VA_SSA_HI, 0x1C0);
    assert_eq!(registers::S_ENDX_HI, 0x340);
    assert_eq!(registers::STATX, 0x344);
    assert_eq!(registers::A_ESA_HI, 0x2E0);
    assert_eq!(registers::A_EEA_HI, 0x33C);
    assert_eq!(registers::P_MVOLL, 0x760);
    assert_eq!(registers::C_IRQINFO, 0x7C2);
}

#[test]
fn voice_registers_isolated_across_cores_and_voices() {
    let mut d = Device::new();
    for core in 0..2usize {
        for v in 0..24u32 {
            d.write_register(
                core_reg(core, registers::VP_PITCH + v * VOICE_STRIDE),
                (0x100 + (core as u32) * 24 + v) as u16,
            )
            .unwrap();
        }
    }
    for core in 0..2usize {
        for v in 0..24u32 {
            assert_eq!(
                d.read_register(core_reg(core, registers::VP_PITCH + v * VOICE_STRIDE))
                    .unwrap(),
                (0x100 + (core as u32) * 24 + v) as u16
            );
        }
    }
}

#[test]
fn statx_reflects_dma_bits() {
    let mut d = Device::new();
    assert_eq!(d.read_register(registers::STATX).unwrap() & 0x80, 0);
    d.write_register(registers::CORE_ATTR, 0x8000 | CONTROL_DMA_WRITE)
        .unwrap();
    assert_eq!(d.read_register(registers::STATX).unwrap() & 0x80, 0x80);
    d.write_register(registers::CORE_ATTR, 0x8000).unwrap();
    assert_eq!(d.read_register(registers::STATX).unwrap() & 0x80, 0);
}

#[test]
fn address_hi_lo_composition() {
    let mut d = Device::new();
    let target: u32 = 0x123450;
    d.write_register(registers::VA_SSA_HI, ((target >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(registers::VA_SSA_LO, ((target >> 1) & 0xFFFF) as u16)
        .unwrap();
    let hi = d.read_register(registers::VA_SSA_HI).unwrap() as u32;
    let lo = d.read_register(registers::VA_SSA_LO).unwrap() as u32;
    assert_eq!((hi << 17) | (lo << 1), target);
}

#[test]
fn all_48_voices_key_on_and_render_nonzero_on_both_cores() {
    let mut d = Device::new();
    let mut block = [0x77u8; 16];
    block[0] = 0x00;
    block[1] = 0x03;
    d.write_ram(0x5000, &block).unwrap();

    for core in 0..2usize {
        d.write_register(core_reg(core, registers::CORE_ATTR), 0xC000)
            .unwrap();
        for v in 0..24u32 {
            d.write_register(
                core_reg(core, registers::VP_VOLL + v * VOICE_STRIDE),
                0x3FFF,
            )
            .unwrap();
            d.write_register(
                core_reg(core, registers::VP_VOLR + v * VOICE_STRIDE),
                0x3FFF,
            )
            .unwrap();
            d.write_register(
                core_reg(core, registers::VP_PITCH + v * VOICE_STRIDE),
                0x1000,
            )
            .unwrap();
            d.write_register(
                core_reg(core, registers::VP_ADSR1 + v * VOICE_STRIDE),
                0x000F,
            )
            .unwrap();
            d.write_register(
                core_reg(core, registers::VP_ADSR2 + v * VOICE_STRIDE),
                0x0000,
            )
            .unwrap();
            d.write_register(
                core_reg(core, registers::VA_SSA_HI + v * VOICE_ADDR_STRIDE),
                0,
            )
            .unwrap();
            d.write_register(
                core_reg(core, registers::VA_SSA_LO + v * VOICE_ADDR_STRIDE),
                (0x5000u32 >> 1) as u16,
            )
            .unwrap();
        }
        d.write_register(core_reg(core, registers::A_KON_HI), 0xFFFF)
            .unwrap();
        d.write_register(core_reg(core, registers::A_KON_LO), 0x00FF)
            .unwrap();
    }

    let mut buf0 = vec![0i16; 256 * 2];
    let mut buf1 = vec![0i16; 256 * 2];
    d.render_core(0, &mut buf0).unwrap();
    d.render_core(1, &mut buf1).unwrap();

    assert!(buf0.iter().any(|&s| s != 0));
    assert!(buf1.iter().any(|&s| s != 0));
}
