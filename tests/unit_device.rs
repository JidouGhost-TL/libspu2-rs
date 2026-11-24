// SPDX-License-Identifier: BSD-2-Clause

use libspu2::{Device, RAM_SIZE};

fn write_loud_block(d: &mut Device, addr: u32) {
    let mut block = [0x77u8; 16];
    block[0] = 0x00; // shift 0, predictor 0
    block[1] = 0x03; // loop + repeat: keeps playing forever
    d.write_ram(addr, &block).unwrap();
}

fn setup_voice0(d: &mut Device, sample_addr: u32) {
    d.write_register(0x19A, 0xC000).unwrap(); // CORE_ATTR (enable + unmute)
    d.write_register(0x000, 0x3FFF).unwrap(); // VP_VOLL
    d.write_register(0x002, 0x3FFF).unwrap(); // VP_VOLR
    d.write_register(0x004, 0x1000).unwrap(); // VP_PITCH
    d.write_register(0x006, 0x000F).unwrap(); // VP_ADSR1
    d.write_register(0x008, 0x0000).unwrap(); // VP_ADSR2
    d.write_register(0x1C0, ((sample_addr >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1C2, ((sample_addr >> 1) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(0x1A0, 0x0001).unwrap(); // A_KON_HI
}

#[test]
fn device_reset_zeroes_ram_and_renders_silence() {
    let mut d = Device::new();
    d.write_ram(0x5000, &[1, 2, 3, 4]).unwrap();
    d.reset();
    let mut back = [9u8; 4];
    d.read_ram(0x5000, &mut back).unwrap();
    assert_eq!(back, [0, 0, 0, 0]);
    let mut pcm = vec![0x1111i16; 64 * 2];
    d.render_native(&mut pcm);
    assert!(pcm.iter().all(|&s| s == 0));
    assert_eq!(d.frame_counter(), 64);
}

#[test]
fn device_ram_bounds() {
    let mut d = Device::new();
    let mut b = [0u8; 2];
    assert!(d.write_ram(RAM_SIZE as u32, &b[..1]).is_err());
    assert!(d.write_ram((RAM_SIZE - 1) as u32, &b[..2]).is_err());
    assert!(d.read_ram(0xFFFF_FFFF, &mut b[..1]).is_err());
    assert!(d.write_ram((RAM_SIZE - 1) as u32, &b[..1]).is_ok());
}

#[test]
fn device_voice_produces_sound() {
    let mut d = Device::new();
    write_loud_block(&mut d, 0x5000);
    setup_voice0(&mut d, 0x5000);
    let mut pcm = vec![0i16; 256 * 2];
    d.render_native(&mut pcm);
    assert!(pcm.iter().any(|&s| s != 0));
}

#[test]
fn device_deterministic() {
    let mut a = vec![0i16; 300 * 2];
    let mut b = vec![0i16; 300 * 2];
    for out in [&mut a, &mut b] {
        let mut d = Device::new();
        write_loud_block(&mut d, 0x5000);
        setup_voice0(&mut d, 0x5000);
        d.render_native(out);
    }
    assert_eq!(a, b);
}

#[test]
fn device_single_frame_render() {
    let mut d = Device::new();
    let mut frame = [1i16, 1];
    d.render_native(&mut frame);
    assert_eq!(frame, [0, 0]);
}
