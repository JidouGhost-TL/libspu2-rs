// SPDX-License-Identifier: BSD-2-Clause

use std::fs;
use std::path::Path;

use libspu2::Device;

/// Replays a `tests/fuzz/fuzz_api.cpp` record stream and verifies no panics,
/// integer overflows, or assertion failures occur.
fn replay_fuzz_input(data: &[u8]) {
    let mut d = Device::new();
    let mut i = 0usize;
    let mut renders = 0usize;
    while i + 5 <= data.len() && renders < 64 {
        let op = data[i] & 3;
        let a = (data[i + 1] as u32) | ((data[i + 2] as u32) << 8) | ((data[i + 3] as u32) << 16);
        let v = (data[i + 3] as u16) | ((data[i + 4] as u16) << 8);
        i += 5;
        match op {
            0 => {
                let _ = d.write_register(a & 0x7FE, v);
            }
            1 => {
                let buf = [(v & 0xFF) as u8; 64];
                let len = ((v & 63) + 1) as usize;
                let _ = d.write_ram(a, &buf[..len]);
            }
            2 => {
                let frames = ((v & 255) + 1) as usize;
                let mut pcm = vec![0i16; frames * 2];
                d.render_native(&mut pcm);
                renders += 1;
            }
            _ => {
                d.reset();
            }
        }
    }
}

fn replay_crash_file(name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fuzz_crashes")
        .join(name);
    let bytes = fs::read(&path).expect("failed to read fuzz crash reproducer");
    replay_fuzz_input(&bytes);
}

#[test]
fn fuzz_regression_computechannelvolume_sweep_phase() {
    replay_crash_file("assert-computechannelvolume-sweep-phase.bin");
}

#[test]
fn fuzz_regression_setaddresshi_shift_overflow() {
    replay_crash_file("ubsan-setaddresshi-shift-overflow.bin");
}

#[test]
fn fuzz_regression_computechannelvolume_exponential_increase() {
    replay_crash_file("assert-computechannelvolume-exponential-increase.bin");
}
