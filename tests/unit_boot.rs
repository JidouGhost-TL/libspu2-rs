// SPDX-License-Identifier: BSD-2-Clause

use libspu2::{registers, Device};

#[test]
fn boot_renders_silence() {
    let mut device = Device::new();
    let mut samples = vec![0x7777i16; 512 * 2];
    device.render_core(0, &mut samples).unwrap();
    assert!(samples.iter().all(|&s| s == 0));
}

#[test]
fn boot_register_roundtrip_both_cores() {
    let mut device = Device::new();
    let off0 = registers::VP_PITCH + 3 * registers::VOICE_STRIDE;
    let off1 = registers::core_reg(1, off0);
    device.write_register(off0, 0x1234).unwrap();
    device.write_register(off1, 0x0ABC).unwrap();
    assert_eq!(device.read_register(off0).unwrap(), 0x1234);
    assert_eq!(device.read_register(off1).unwrap(), 0x0ABC);
}
