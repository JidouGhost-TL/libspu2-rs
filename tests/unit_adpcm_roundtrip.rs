// SPDX-License-Identifier: BSD-2-Clause

mod common;

use libspu2::Device;

fn reference_decode(data: &[u8]) -> Vec<i16> {
    const K0: [i32; 5] = [0, 60, 115, 98, 122];
    const K1: [i32; 5] = [0, 0, -52, -55, -60];
    let mut out = Vec::new();
    let mut s1 = 0i32;
    let mut s2 = 0i32;
    for chunk in data.chunks_exact(16) {
        let shift = (chunk[0] & 0xF) as u32;
        let pred = (chunk[0] >> 4) as usize;
        for &byte in &chunk[2..16] {
            for n in 0..2 {
                let nib = if n == 0 {
                    (((byte & 0x0F) as u16) << 12) as i16
                } else {
                    (((byte & 0xF0) as u16) << 8) as i16
                };
                let work = (nib as i32) >> shift;
                let cur = work * 64 + (s1 * K0[pred]) / 64 + (s2 * K1[pred]) / 64;
                s2 = s1;
                s1 = cur;
                let r = ((cur + 32) / 64).clamp(-32768, 32767);
                out.push(r as i16);
            }
        }
    }
    out
}

#[test]
fn adpcm_roundtrip_sine_quality() {
    let pcm = common::make_sine(440.0, 28 * 64);
    let enc = common::encode_adpcm(&pcm, false, 0);
    assert_eq!(enc.len(), (pcm.len() / 28) * 16);
    let dec = reference_decode(&enc);
    assert_eq!(dec.len(), pcm.len());
    let mut err = 0.0f64;
    let mut ref_e = 0.0f64;
    for (&d_s, &p_s) in dec.iter().zip(pcm.iter()) {
        let diff = (d_s as f64) - (p_s as f64);
        err += diff * diff;
        ref_e += (p_s as f64) * (p_s as f64);
    }
    assert!(ref_e > 0.0);
    assert!(err / ref_e < 0.01);
}

#[test]
fn adpcm_flags() {
    let pcm = common::make_sine(440.0, 28 * 4);
    let one_shot = common::encode_adpcm(&pcm, false, 0);
    assert_eq!(one_shot[3 * 16 + 1], 0x01);
    let looped = common::encode_adpcm(&pcm, true, 28 * 2);
    assert_eq!(looped[2 * 16 + 1] & 0x04, 0x04);
    assert_eq!(looped[3 * 16 + 1], 0x03);
}

#[test]
fn adpcm_plays_on_device() {
    let mut d = Device::new();
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);
    d.write_ram(0x5000, &enc).unwrap();
    d.write_register(0x19A, 0xC000).unwrap();
    d.write_register(0x000, 0x3FFF).unwrap();
    d.write_register(0x002, 0x3FFF).unwrap();
    d.write_register(0x004, 0x1000).unwrap();
    d.write_register(0x006, 0x000F).unwrap();
    d.write_register(0x008, 0x0000).unwrap();
    d.write_register(0x1C0, 0).unwrap();
    d.write_register(0x1C2, (0x5000u32 >> 1) as u16).unwrap();
    d.write_register(0x1A0, 0x0001).unwrap();
    let mut out = vec![0i16; 1024 * 2];
    d.render_native(&mut out);
    let peak = out.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    assert!(peak > 1000);
}
