// SPDX-License-Identifier: BSD-2-Clause

mod common;

use libspu2::Device;

fn upload_loop(d: &mut Device, addr: u32, samples: usize) {
    let pcm = common::make_sine(440.0, samples);
    let enc = common::encode_adpcm(&pcm, true, 0);
    d.write_ram(addr, &enc).unwrap();
}

fn voice_on(d: &mut Device, addr: u32, pitch: u16) {
    d.write_register(0x19A, 0x8000).unwrap();
    d.write_register(0x000, 0x3FFF).unwrap();
    d.write_register(0x002, 0x3FFF).unwrap();
    d.write_register(0x004, pitch).unwrap();
    d.write_register(0x006, 0x000F).unwrap();
    d.write_register(0x008, 0x0000).unwrap();
    d.write_register(0x1C0, ((addr >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1C2, ((addr >> 1) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1A0, 0x0001).unwrap();
}

fn nax(d: &mut Device) -> u32 {
    ((d.read_register(0x1C8).unwrap() as u32) << 17)
        | ((d.read_register(0x1CA).unwrap() as u32) << 1)
}

fn render(d: &mut Device, frames: usize) {
    let mut buf = vec![0i16; frames * 2];
    d.render_native(&mut buf);
}

#[test]
fn pitch_half_advances_half_as_fast() {
    let mut fast = Device::new();
    let mut slow = Device::new();
    upload_loop(&mut fast, 0x5000, 28 * 64);
    upload_loop(&mut slow, 0x5000, 28 * 64);
    voice_on(&mut fast, 0x5000, 0x1000);
    voice_on(&mut slow, 0x5000, 0x0800);
    render(&mut fast, 512);
    render(&mut slow, 512);
    let fast_delta = nax(&mut fast) - 0x5000;
    let slow_delta = nax(&mut slow) - 0x5000;
    assert!(fast_delta > 0);
    assert!((fast_delta as i32 - 2 * slow_delta as i32).abs() <= 32);
}

#[test]
fn one_shot_sets_endx_and_stops() {
    let mut d = Device::new();
    let pcm = common::make_sine(440.0, 28 * 4);
    let enc = common::encode_adpcm(&pcm, false, 0);
    d.write_ram(0x5000, &enc).unwrap();
    voice_on(&mut d, 0x5000, 0x1000);
    render(&mut d, 28 * 4 + 256);
    assert_eq!(d.read_register(0x340).unwrap() & 1, 1);
    assert_eq!(d.read_register(0x00A).unwrap(), 0);
}

#[test]
fn loop_flag_03_keeps_playing() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000, 28 * 4);
    voice_on(&mut d, 0x5000, 0x1000);
    render(&mut d, 28 * 16);
    assert_eq!(d.read_register(0x340).unwrap() & 1, 1);
    let mut buf = vec![0i16; 256 * 2];
    d.render_native(&mut buf);
    let peak = buf.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    assert!(peak > 0);
}

#[test]
fn endx_cleared_by_write() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000, 28 * 4);
    voice_on(&mut d, 0x5000, 0x1000);
    render(&mut d, 28 * 16);
    assert_eq!(d.read_register(0x340).unwrap() & 1, 1);
    d.write_register(0x340, 1).unwrap();
    assert_eq!(d.read_register(0x340).unwrap(), 0);
    assert_eq!(d.read_register(0x342).unwrap(), 0);
}

#[test]
fn repeated_keyon_restarts_voice() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000, 28 * 64);
    voice_on(&mut d, 0x5000, 0x1000);
    render(&mut d, 1024);
    let advanced = nax(&mut d);
    assert!(advanced > 0x5000);
    d.write_register(0x1A0, 0x0001).unwrap();
    render(&mut d, 4);
    let restarted = nax(&mut d);
    assert!(restarted < advanced);
    assert!((restarted & !0xFu32) >= 0x5000);
}

#[test]
fn keyoff_releases_to_silence() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000, 28 * 64);
    voice_on(&mut d, 0x5000, 0x1000);
    render(&mut d, 512);
    assert_ne!(d.read_register(0x00A).unwrap(), 0);
    d.write_register(0x1A4, 0x0001).unwrap();
    render(&mut d, 48_000);
    assert_eq!(d.read_register(0x00A).unwrap(), 0);
}
