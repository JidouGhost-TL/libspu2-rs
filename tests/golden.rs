// SPDX-License-Identifier: BSD-2-Clause

mod common;

use std::fs;
use std::path::Path;

use libspu2::Device;

fn check_golden(name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let fixture_path = dir.join(format!("{name}.fixture"));
    let expected_path = dir.join(format!("{name}.expected"));

    let mut device = Device::new();
    let result = common::run_fixture(&fixture_path, &mut device);
    let expected = fs::read_to_string(&expected_path)
        .expect("failed to read expected hash")
        .trim()
        .to_string();
    let actual = format!("{:016x}", result.pcm_hash);
    assert_eq!(
        actual, expected,
        "golden {name} mismatch: expected {expected}, got {actual} ({} frames)",
        result.frames_rendered
    );
}

#[test]
fn golden_single_voice() {
    check_golden("single_voice");
}

#[test]
fn golden_two_core_mix() {
    check_golden("two_core_mix");
}

#[test]
fn golden_reverb_smoke() {
    check_golden("reverb_smoke");
}

#[test]
fn golden_reverb_actually_differs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut a = Device::new();
    let mut b = Device::new();
    let ra = common::run_fixture(&dir.join("single_voice.fixture"), &mut a);
    let rb = common::run_fixture(&dir.join("reverb_smoke.fixture"), &mut b);
    assert_ne!(ra.pcm_hash, rb.pcm_hash);
}
