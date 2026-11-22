// SPDX-License-Identifier: BSD-2-Clause

mod common;

use common::SpuTestFixture;
use libspu2::registers::{self, CONTROL_IRQ};

#[test]
fn sputest_key_on_off() {
    let mut f = SpuTestFixture::new();
    let addr1 = 0x5000u32;
    let addr2 = 0x6000u32;
    f.poke_ram(addr1, 0, 0x07);
    f.poke_ram(addr2, 0, 0x07);
    for i in 0..SpuTestFixture::VOICE_COUNT {
        f.set_voice_register(0, i, registers::VP_PITCH, 0x1000);
        f.set_voice_address(0, i, registers::VA_SSA_HI, addr1);
    }
    f.set_core_register(0, registers::A_KON_HI, 0xFFFF);
    f.set_core_register(0, registers::A_KON_LO, 0xFF);
    f.run_spu(64);
    assert_eq!(
        f.get_voice_address(0, 0, registers::VA_NAX_HI) & !0xFu32,
        addr1 & !0xFu32
    );

    f.set_voice_address(0, 0, registers::VA_SSA_HI, addr2);
    f.set_core_register(0, registers::A_KON_HI, 1);
    f.set_core_register(0, registers::A_KOFF_HI, 1);
    f.run_spu(64);
    assert_eq!(
        f.get_voice_address(0, 0, registers::VA_NAX_HI) & !0xFu32,
        addr2 & !0xFu32
    );

    f.set_core_register(0, registers::CORE_ATTR, CONTROL_IRQ);
    f.set_core_address(0, registers::A_IRQA_HI, !0u32);
    f.run_spu(1024);
    assert_eq!(f.read_irq_info(), 0);
    assert_eq!(
        f.get_voice_address(0, 0, registers::VA_NAX_HI) & !0xFu32,
        addr2 & !0xFu32
    );
}

#[test]
fn sputest_sweep() {
    let mut f = SpuTestFixture::new();
    let addr = 0x5000u32;
    f.poke_ram(addr, 0, 0x04);
    for i in 0..SpuTestFixture::VOICE_COUNT {
        f.set_voice_register(0, i, registers::VP_PITCH, 0x1000);
        f.set_voice_address(0, i, registers::VA_SSA_HI, addr);
    }
    f.set_voice_register(0, 0, registers::VP_VOLL, 0x3FFF);
    f.set_core_register(0, registers::A_KON_HI, 1);
    f.set_core_register(0, registers::A_KON_LO, 0);
    f.run_spu(128);
    assert_eq!(f.get_voice_register(0, 0, registers::VP_VOLXL), 0x7FFE);
    f.set_voice_register(0, 0, registers::VP_VOLL, 0xE010);
    f.run_spu(128 * 128);
    assert_eq!(f.get_voice_register(0, 0, registers::VP_VOLXL), 0);
}

#[test]
fn sputest_simple_irq() {
    let mut f = SpuTestFixture::new();
    let null_addr = 0x5000u32;
    let irq_addr = 0x6000u32;
    f.poke_ram(null_addr, 0, 0x07);
    f.poke_ram(irq_addr, 0, 0x07);
    for i in 0..SpuTestFixture::VOICE_COUNT {
        f.set_voice_register(0, i, registers::VP_PITCH, 0x1000);
        f.set_voice_address(0, i, registers::VA_SSA_HI, null_addr);
    }
    f.set_core_register(0, registers::A_KON_HI, 0xFFFF);
    f.set_core_register(0, registers::A_KON_LO, 0xFF);
    f.set_core_register(0, registers::CORE_ATTR, CONTROL_IRQ);
    f.set_core_address(0, registers::A_IRQA_HI, !0u32);
    f.run_spu(1024);
    assert_eq!(f.read_irq_info(), 0);
    f.set_core_address(0, registers::A_IRQA_HI, irq_addr);
    f.set_voice_address(0, 0, registers::VA_SSA_HI, irq_addr);
    f.set_core_register(0, registers::A_KON_HI, 0x1);
    f.run_spu(1024);
    assert_eq!(f.read_irq_info(), 0x04);
}

#[test]
fn sputest_multi_core_irq() {
    let mut f = SpuTestFixture::new();
    let null_addr = 0x5000u32;
    let core0_irq = 0x6000u32;
    let core1_irq = 0x7000u32;
    f.poke_ram(null_addr, 0, 0x07);
    f.poke_ram(core0_irq, 0, 0x07);
    f.poke_ram(core1_irq, 0, 0x07);
    for i in 0..SpuTestFixture::VOICE_COUNT {
        f.set_voice_register(0, i, registers::VP_PITCH, 0x1000);
        f.set_voice_address(0, i, registers::VA_SSA_HI, null_addr);
    }
    f.set_core_register(0, registers::A_KON_HI, 0xFFFF);
    f.set_core_register(0, registers::A_KON_LO, 0xFF);
    for i in 0..SpuTestFixture::CORE_COUNT {
        f.set_core_register(i, registers::CORE_ATTR, CONTROL_IRQ);
        f.set_core_address(i, registers::A_IRQA_HI, !0u32);
    }
    f.run_spu(1024);
    assert_eq!(f.read_irq_info(), 0);
    f.set_core_address(0, registers::A_IRQA_HI, core0_irq);
    f.set_core_address(1, registers::A_IRQA_HI, core1_irq);
    f.set_voice_address(0, 0, registers::VA_SSA_HI, core1_irq);
    f.set_core_register(0, registers::A_KON_HI, 0x1);
    f.run_spu(1024);
    assert_eq!(f.read_irq_info(), 0x08);
    let attr = f.get_core_register(1, registers::CORE_ATTR);
    f.set_core_register(1, registers::CORE_ATTR, attr & !CONTROL_IRQ);
    f.set_core_address(1, registers::A_IRQA_HI, !0u32);
    f.run_spu(1024);
    assert_eq!(f.read_irq_info(), 0);
    f.set_voice_address(1, 0, registers::VA_SSA_HI, core0_irq);
    f.set_core_register(1, registers::A_KON_HI, 0x1);
    f.run_spu(512);
    f.run_spu(512);
    assert_eq!(f.read_irq_info(), 0x04);
}

#[test]
fn sputest_set_repeat() {
    let mut f = SpuTestFixture::new();
    let null_addr = 0x5000u32;
    let other_addr = 0x6000u32;
    f.poke_ram(null_addr, 0, 0x07);
    f.poke_ram(other_addr, 0, 0x01);
    for i in 0..SpuTestFixture::VOICE_COUNT {
        f.set_voice_register(0, i, registers::VP_PITCH, 0x1000);
        f.set_voice_address(0, i, registers::VA_SSA_HI, null_addr);
    }
    f.set_core_register(0, registers::A_KON_HI, 0xFFFF);
    f.set_core_register(0, registers::A_KON_LO, 0xFF);
    f.run_spu(128);
    assert_eq!(f.get_voice_address(0, 0, registers::VA_LSAX_HI), null_addr);
    f.set_voice_address(0, 0, registers::VA_LSAX_HI, other_addr);
    f.run_spu(128);
    assert_eq!(f.get_voice_address(0, 0, registers::VA_LSAX_HI), other_addr);
    f.set_voice_address(0, 0, registers::VA_SSA_HI, other_addr);
    f.set_core_register(0, registers::A_KON_HI, 1);
    f.run_spu(128);
    assert_eq!(f.get_voice_address(0, 0, registers::VA_LSAX_HI), other_addr);
}

#[test]
fn sputest_set_repeat2() {
    let mut f = SpuTestFixture::new();
    let null_addr = 0x5000u32;
    let loop_addr = 0x6000u32;
    f.poke_ram(null_addr, 0, 0x07);
    f.poke_ram(loop_addr, 0, 0x00);
    f.poke_ram(loop_addr + 0x10, 0, 0x04);
    f.poke_ram(loop_addr + 0x20, 0, 0x03);
    f.set_voice_register(0, 0, registers::VP_PITCH, 0x1000);
    f.set_voice_address(0, 0, registers::VA_SSA_HI, loop_addr);
    f.set_core_register(0, registers::A_KON_HI, 0x1);
    f.run_spu(1024);
    assert_eq!(
        f.get_voice_address(0, 0, registers::VA_LSAX_HI),
        loop_addr + 0x10
    );
    assert_eq!(f.get_core_register(0, registers::S_ENDX_HI), 1);
    f.set_core_register(0, registers::A_KOFF_HI, 0x1);
    f.run_spu(1024);
    f.set_voice_register(0, 0, registers::VP_PITCH, 0x1000);
    f.set_voice_address(0, 0, registers::VA_SSA_HI, loop_addr);
    f.set_core_register(0, registers::A_KON_HI, 0x1);
    f.set_voice_address(0, 0, registers::VA_LSAX_HI, null_addr);
    f.run_spu(1024);
    assert_eq!(f.get_voice_address(0, 0, registers::VA_LSAX_HI), null_addr);
}
