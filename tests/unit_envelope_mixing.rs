// SPDX-License-Identifier: BSD-2-Clause

mod common;

use libspu2::Device;

fn upload_loop(d: &mut Device, addr: u32) {
    let pcm = common::make_sine(440.0, 28 * 64);
    let enc = common::encode_adpcm(&pcm, true, 0);
    d.write_ram(addr, &enc).unwrap();
}

fn render(d: &mut Device, frames: usize) {
    let mut buf = vec![0i16; frames * 2];
    d.render_native(&mut buf);
}

fn setup_voice(d: &mut Device, adsr1: u16, adsr2: u16, vol_l: u16, vol_r: u16) {
    d.write_register(0x19A, 0x8000).unwrap();
    d.write_register(0x000, vol_l).unwrap();
    d.write_register(0x002, vol_r).unwrap();
    d.write_register(0x004, 0x1000).unwrap();
    d.write_register(0x006, adsr1).unwrap();
    d.write_register(0x008, adsr2).unwrap();
    d.write_register(0x1C0, 0).unwrap();
    d.write_register(0x1C2, 0x5000 >> 1).unwrap();
    d.write_register(0x1A0, 1).unwrap();
}

#[test]
fn adsr_slow_attack_lags_fast_attack() {
    let mut fast = Device::new();
    let mut slow = Device::new();
    upload_loop(&mut fast, 0x5000);
    upload_loop(&mut slow, 0x5000);
    setup_voice(&mut fast, 0x000F, 0, 0x3FFF, 0x3FFF);
    setup_voice(&mut slow, 0x000F | (0x40 << 8), 0, 0x3FFF, 0x3FFF);
    render(&mut fast, 64);
    render(&mut slow, 64);
    assert!(fast.read_register(0x00A).unwrap() > slow.read_register(0x00A).unwrap());
    assert!(slow.read_register(0x00A).unwrap() > 0);
}

#[test]
fn envx_nonzero_immediately_after_keyon() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000);
    d.write_register(0x19A, 0x8000).unwrap();
    d.write_register(0x006, 0x000F).unwrap();
    d.write_register(0x1C2, 0x5000 >> 1).unwrap();
    d.write_register(0x1A0, 1).unwrap();
    assert_ne!(d.read_register(0x00A).unwrap(), 0);
}

#[test]
fn stereo_routing_left_only() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000);
    setup_voice(&mut d, 0x000F, 0, 0x3FFF, 0x0000);
    render(&mut d, 64);
    let mut buf = vec![0i16; 256 * 2];
    d.render_native(&mut buf);
    let mut peak_l = 0i32;
    let mut peak_r = 0i32;
    for chunk in buf.chunks_exact(2) {
        peak_l = peak_l.max((chunk[0] as i32).abs());
        peak_r = peak_r.max((chunk[1] as i32).abs());
    }
    assert!(peak_l > 1000);
    assert_eq!(peak_r, 0);
}

#[test]
fn volume_phase_bit() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000);
    setup_voice(&mut d, 0x000F, 0, 0x4000 | 0x3FFF, 0x3FFF);
    render(&mut d, 64);
    assert_eq!(d.read_register(0x00C).unwrap(), 0);
}

#[test]
fn linear_sweep_increase_wraps_period_3() {
    let mut d = Device::new();
    upload_loop(&mut d, 0x5000);
    setup_voice(&mut d, 0x000F, 0, 0x0000, 0x3FFF);
    render(&mut d, 4);
    d.write_register(0x000, 0x8000).unwrap();
    render(&mut d, 48_000);
    assert_eq!(d.read_register(0x00C).unwrap(), 0x0000);
}

#[test]
fn reverb_toggle_changes_output() {
    let run = |reverb: bool| -> u64 {
        let mut d = Device::new();
        upload_loop(&mut d, 0x5000);
        d.write_register(0x19A, if reverb { 0x80C0 } else { 0x8000 })
            .unwrap();
        if reverb {
            d.write_register(0x2E0, 0x000C).unwrap();
            d.write_register(0x2E2, 0x0000).unwrap();
            d.write_register(0x33C, 0x000F).unwrap();
            d.write_register(0x194, 0x0001).unwrap();
            for i in 0..10u32 {
                d.write_register(0x774 + 2 * i, 0x2000).unwrap();
            }
        }
        d.write_register(0x000, 0x3FFF).unwrap();
        d.write_register(0x002, 0x3FFF).unwrap();
        d.write_register(0x004, 0x1000).unwrap();
        d.write_register(0x006, 0x000F).unwrap();
        d.write_register(0x1C2, 0x5000 >> 1).unwrap();
        d.write_register(0x1A0, 1).unwrap();
        let mut buf = vec![0i16; 24_000 * 2];
        d.render_native(&mut buf);
        common::fnv1a64_pcm(&buf)
    };
    assert_ne!(run(false), run(true));
}
