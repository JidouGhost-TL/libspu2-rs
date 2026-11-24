// SPDX-License-Identifier: BSD-2-Clause

mod common;

use libspu2::{Device, RAM_SIZE};

fn key_on_voice0(d: &mut Device, addr: u32) {
    d.write_register(0x19A, 0xC000).unwrap();
    d.write_register(0x000, 0x3FFF).unwrap();
    d.write_register(0x002, 0x3FFF).unwrap();
    d.write_register(0x004, 0x1000).unwrap();
    d.write_register(0x006, 0x000F).unwrap();
    d.write_register(0x1C0, ((addr >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1C2, ((addr >> 1) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1A0, 1).unwrap();
}

fn render_hash(d: &mut Device, frames: usize) -> u64 {
    let mut buf = vec![0i16; frames * 2];
    d.render_native(&mut buf);
    common::fnv1a64_pcm(&buf)
}

#[test]
fn cache_invalidation_overlap_from_below() {
    let pcm = common::make_sine(440.0, 28 * 8);
    let enc = common::encode_adpcm(&pcm, true, 0);

    let mut d = Device::new();
    d.write_ram(0x5000, &enc).unwrap();
    key_on_voice0(&mut d, 0x5000);
    let before = render_hash(&mut d, 2048);

    let patch = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x08];
    d.write_ram(0x5008, &patch).unwrap();
    d.write_register(0x1A0, 1).unwrap();
    let after = render_hash(&mut d, 2048);

    let mut fresh = Device::new();
    let mut patched = enc.clone();
    patched[8..16].copy_from_slice(&patch);
    fresh.write_ram(0x5000, &patched).unwrap();
    key_on_voice0(&mut fresh, 0x5000);
    let _ = render_hash(&mut fresh, 2048);
    fresh.write_register(0x1A0, 1).unwrap();
    let truth = render_hash(&mut fresh, 2048);

    assert_eq!(after, truth);
    assert_ne!(after, before);
}

#[test]
fn transfer_fifo_write_and_wrap() {
    let mut d = Device::new();
    let tsa = (RAM_SIZE - 4) as u32;
    d.write_register(0x1A8, ((tsa >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1AA, ((tsa >> 1) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1AC, 0xAAAA).unwrap();
    d.write_register(0x1AC, 0xBBBB).unwrap();
    d.write_register(0x1AC, 0xCCCC).unwrap();

    let mut back = [0u8; 4];
    d.read_ram(tsa, &mut back).unwrap();
    assert_eq!(u16::from_le_bytes([back[0], back[1]]), 0xAAAA);
    assert_eq!(u16::from_le_bytes([back[2], back[3]]), 0xBBBB);

    let mut wrapped = [0u8; 2];
    d.read_ram(0, &mut wrapped).unwrap();
    assert_eq!(u16::from_le_bytes(wrapped), 0xCCCC);
}
