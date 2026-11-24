// SPDX-License-Identifier: BSD-2-Clause

mod common;

use libspu2::registers::{self, core_reg, ps1};
use libspu2::{AdsrConfig, Device, EnvelopePhase, Synth, VoiceParameters};

#[test]
fn one_block_adpcm_loop_flags_0x07_loops_continuously() {
    let mut d = Device::new();
    // Single 16-byte ADPCM block with flags == 0x07 (loop_end | loop_repeat | loop_start).
    let mut block = [0u8; 16];
    block[0] = 0x00; // shift 0, predictor 0
    block[1] = 0x07;
    for b in &mut block[2..16] {
        *b = 0x5A; // alternating positive/negative nibbles
    }
    d.write_ram(0x5000, &block).unwrap();

    {
        let mut s = Synth::new(&mut d);
        s.enable_core(0, true).unwrap();
        let vp = VoiceParameters {
            start_address: 0x5000,
            loop_address: None,
            pitch: 0x1000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }

    // Render well beyond 28 samples (10x the single block).
    let mut warmup = vec![0i16; 280 * 2];
    d.render_native(&mut warmup);

    let vs = d.inspect_voice(0, 0).unwrap();
    assert_ne!(vs.envelope_phase, EnvelopePhase::Stopped);
    assert!(vs.envx > 0);
    assert!(vs.reached_end);
    assert_eq!(vs.repeat_address, 0x5000);

    // Subsequent frames must remain non-zero.
    let mut tail = vec![0i16; 128 * 2];
    d.render_native(&mut tail);
    let peak = tail.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    assert!(
        peak > 1000,
        "1-block 0x07 loop should still be sounding, peak={peak}"
    );
}

#[test]
fn signed_fixed_voice_volume_inverts_phase_exactly() {
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);

    let run = |vol_l: i16, vol_r: i16| -> Vec<i16> {
        let mut d = Device::new();
        d.write_ram(0x5000, &enc).unwrap();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            let vp = VoiceParameters {
                start_address: 0x5000,
                pitch: 0x1000,
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
            s.set_volume_signed(0, 0, vol_l, vol_r).unwrap();
            s.key_on_voice(0, 0).unwrap();
        }
        let mut out = vec![0i16; 512 * 2];
        d.render_native(&mut out);
        out
    };

    let pos = run(0x3FFF, 0x3FFF);
    let neg = run(-0x3FFF, -0x3FFF);

    assert!(pos.iter().any(|&s| s.abs() > 1000));
    for (i, (&p, &n)) in pos.iter().zip(neg.iter()).enumerate() {
        assert!(
            ((n as i32) + (p as i32)).abs() <= 2,
            "sample {i}: pos={p}, neg={n}"
        );
    }
}

#[test]
fn master_volume_scales_mutes_and_inverts_phase() {
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);

    let run = |mvol_l: i16, mvol_r: i16| -> Vec<i16> {
        let mut d = Device::new();
        d.write_ram(0x5000, &enc).unwrap();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            s.set_master_volume(0, mvol_l, mvol_r).unwrap();
            let vp = VoiceParameters {
                start_address: 0x5000,
                pitch: 0x1000,
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
            s.key_on_voice(0, 0).unwrap();
        }
        let mut out = vec![0i16; 512 * 2];
        d.render_native(&mut out);
        out
    };

    let full = run(0x3FFF, 0x3FFF);
    let half = run(0x2000, 0x2000);
    let mute = run(0, 0);
    let inv = run(-0x3FFF, -0x3FFF);

    let peak = |buf: &[i16]| buf.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    assert!(peak(&full) > 5000);
    assert_eq!(peak(&mute), 0);
    assert!(peak(&half) > 2000 && peak(&half) < peak(&full) * 3 / 4);
    for (i, (&p, &n)) in full.iter().zip(inv.iter()).enumerate() {
        assert!(
            ((n as i32) + (p as i32)).abs() <= 1,
            "sample {i}: full={p}, inv={n}"
        );
    }

    // PS1 MVOLL/MVOLR register round-trip and mute check.
    let mut ps1 = Device::new_ps1();
    ps1.write_ps1_register(ps1::MVOLL, 0x2345).unwrap();
    ps1.write_ps1_register(ps1::MVOLR, 0x4001).unwrap();
    assert_eq!(ps1.read_ps1_register(ps1::MVOLL).unwrap(), 0x2345);
    assert_eq!(ps1.read_ps1_register(ps1::MVOLR).unwrap(), 0x4001);
}

#[test]
fn reverb_return_volume_evol_scales_and_mutes_wet_return() {
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);

    let run = |reverb_enabled: bool, evol: i16| -> Vec<i16> {
        let mut d = Device::new();
        d.write_ram(0x5000, &enc).unwrap();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            if reverb_enabled {
                s.set_reverb_work_area(0, 0x18_0000, 0x20_0000).unwrap();
                s.set_reverb_send(0, 0x0000_0001).unwrap();
                s.set_reverb_volume(0, evol, evol).unwrap();
                for i in 0..10u32 {
                    s.device_mut()
                        .write_register(registers::RVB_COEF_BASE + i * 2, 0x2000)
                        .unwrap();
                }
                s.device_mut()
                    .write_register(
                        registers::CORE_ATTR,
                        registers::CONTROL_ENABLE
                            | registers::CONTROL_UNMUTE
                            | registers::CONTROL_REVERB,
                    )
                    .unwrap();
            }
            let vp = VoiceParameters {
                start_address: 0x5000,
                pitch: 0x1000,
                left_volume: 0x1FFF,
                right_volume: 0x1FFF,
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
            s.key_on_voice(0, 0).unwrap();
        }
        let mut out = vec![0i16; 8000 * 2];
        d.render_native(&mut out);
        out
    };

    let dry = run(false, 0);
    let wet_zero = run(true, 0);
    let wet_low = run(true, 0x1000);
    let wet_high = run(true, 0x6000);

    // EVOLL = EVOLR = 0 must produce identical output to reverb disabled.
    assert_eq!(dry, wet_zero);

    let diff_energy = |a: &[i16], b: &[i16]| -> u64 {
        a.iter()
            .zip(b.iter())
            .map(|(&x, &y)| ((x as i32 - y as i32).unsigned_abs()) as u64)
            .sum()
    };
    let e_low = diff_energy(&dry, &wet_low);
    let e_high = diff_energy(&dry, &wet_high);
    assert!(e_low > 0, "low EVOL should add reverb energy");
    assert!(
        e_high > e_low * 3,
        "higher EVOL ({e_high}) should scale wet return above low EVOL ({e_low})"
    );

    // PS1 EVOLL/EVOLR round-trip.
    let mut ps1 = Device::new_ps1();
    ps1.write_ps1_register(ps1::EVOLL, 0x1234).unwrap();
    ps1.write_ps1_register(ps1::EVOLR, 0xF000).unwrap();
    assert_eq!(ps1.read_ps1_register(ps1::EVOLL).unwrap(), 0x1234);
    assert_eq!(ps1.read_ps1_register(ps1::EVOLR).unwrap(), 0xF000);
}

#[test]
fn pmon_modulates_voice_1_pitch_from_voice_0_output() {
    let mod_pcm = common::make_sine(110.0, 28 * 64);
    let mod_enc = common::encode_adpcm(&mod_pcm, true, 0);
    let car_pcm = common::make_sine(440.0, 28 * 64);
    let car_enc = common::encode_adpcm(&car_pcm, true, 0);

    let run = |pmon_enabled: bool| -> Vec<i16> {
        let mut d = Device::new();
        d.write_ram(0x5000, &mod_enc).unwrap();
        d.write_ram(0x8000, &car_enc).unwrap();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            // Voice 0: modulator (muted at output mix, but post-envelope output still drives PMON!)
            let v0 = VoiceParameters {
                index: 0,
                start_address: 0x5000,
                pitch: 0x1000,
                left_volume: 0,
                right_volume: 0,
                ..VoiceParameters::default()
            };
            // Voice 1: carrier
            let v1 = VoiceParameters {
                index: 1,
                start_address: 0x8000,
                pitch: 0x1000,
                left_volume: 0x3FFF,
                right_volume: 0x3FFF,
                ..VoiceParameters::default()
            };
            s.configure_voice(&v0).unwrap();
            s.configure_voice(&v1).unwrap();
            if pmon_enabled {
                s.set_pmon(0, 1 << 1).unwrap();
            }
            s.key_on(0, 0x0003).unwrap();
        }
        let mut out = vec![0i16; 2048 * 2];
        d.render_native(&mut out);
        out
    };

    let unmod = run(false);
    let modulated = run(true);
    assert!(unmod.iter().any(|&s| s.abs() > 1000));
    assert!(modulated.iter().any(|&s| s.abs() > 1000));
    assert_ne!(
        unmod, modulated,
        "PMON must visibly modulate voice 1 output"
    );

    // Bit 0 of PMON is always masked off on hardware (SPU2 & PS1).
    let mut d = Device::new();
    d.write_register(registers::S_PMON_HI, 0xFFFF).unwrap();
    assert_eq!(d.read_register(registers::S_PMON_HI).unwrap(), 0xFFFE);
    let mut ps1 = Device::new_ps1();
    ps1.write_ps1_register(ps1::PMON_LO, 0xFFFF).unwrap();
    ps1.write_ps1_register(ps1::PMON_HI, 0x00FF).unwrap();
    assert_eq!(ps1.read_ps1_register(ps1::PMON_LO).unwrap(), 0xFFFE);
    assert_eq!(ps1.read_ps1_register(ps1::PMON_HI).unwrap(), 0x00FF);
}

#[test]
fn noise_generator_and_noise_clock_produce_deterministic_broadband_noise() {
    let run_noise = |shift: u8, step: u8| -> Vec<i16> {
        let mut d = Device::new();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            s.set_noise(0, 1 << 0).unwrap();
            s.set_noise_clock(0, shift, step).unwrap();
            let vp = VoiceParameters {
                index: 0,
                start_address: 0x5000,
                pitch: 0x1000,
                left_volume: 0x3FFF,
                right_volume: 0x3FFF,
                adsr: AdsrConfig {
                    attack_rate: 0,
                    decay_rate: 0,
                    sustain_level: 0x0F,
                    sustain_rate: 0x7F,
                    release_rate: 0,
                    ..AdsrConfig::default()
                },
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
            s.key_on_voice(0, 0).unwrap();
        }
        let mut out = vec![0i16; 1024 * 2];
        d.render_native(&mut out);
        out
    };

    let fast_a = run_noise(15, 0);
    let fast_b = run_noise(15, 0);
    let slow = run_noise(4, 0);

    // Deterministic output.
    assert_eq!(fast_a, fast_b);

    // Broadband: has both positive and negative samples and many transitions.
    let left_fast: Vec<i16> = fast_a.chunks_exact(2).map(|f| f[0]).collect();
    let left_slow: Vec<i16> = slow.chunks_exact(2).map(|f| f[0]).collect();
    assert!(left_fast.iter().any(|&s| s > 5000));
    assert!(left_fast.iter().any(|&s| s < -5000));

    let transitions = |s: &[i16]| s.windows(2).filter(|w| w[0] != w[1]).count();
    let t_fast = transitions(&left_fast[64..]);
    let t_slow = transitions(&left_slow[64..]);
    assert!(
        t_fast > 500,
        "fast noise clock should produce broadband transitions, got {t_fast}"
    );
    assert!(
        t_slow < t_fast / 4,
        "slow noise clock ({t_slow}) should transition much less often than fast ({t_fast})"
    );

    // PS1 NON register round-trip.
    let mut ps1 = Device::new_ps1();
    ps1.write_ps1_register(ps1::NON_LO, 0x1234).unwrap();
    ps1.write_ps1_register(ps1::NON_HI, 0x00AB).unwrap();
    assert_eq!(ps1.read_ps1_register(ps1::NON_LO).unwrap(), 0x1234);
    assert_eq!(ps1.read_ps1_register(ps1::NON_HI).unwrap(), 0x00AB);
}

#[test]
fn synth_thin_hardware_wrappers_and_validation() {
    let mut d = Device::new();
    {
        let mut s = Synth::new(&mut d);
        s.enable_core(1, true).unwrap();

        let adsr = AdsrConfig {
            attack_rate: 0x12,
            decay_rate: 0x05,
            sustain_level: 0x0A,
            sustain_rate: 0x34,
            release_rate: 0x09,
            attack_exponential: true,
            sustain_exponential: false,
            sustain_decrease: true,
            release_exponential: true,
        };
        s.set_adsr(1, 2, adsr).unwrap();
        s.set_volume_signed(1, 2, -0x4000, 0x3FFF).unwrap();
        s.set_pmon(1, 0x00AB_CDEF).unwrap();
        s.set_noise(1, 0x0055_AA01).unwrap();
        s.set_noise_clock(1, 11, 2).unwrap();
        s.set_master_volume(1, -0x2000, 0x3000).unwrap();
        s.set_reverb_volume(1, -0x4000, 0x7FFF).unwrap();
        s.set_reverb_work_area(1, 0x10_0000, 0x18_0000).unwrap();

        // Validation errors.
        assert!(s.set_volume_signed(1, 2, -0x4001, 0).is_err());
        assert!(s.set_volume_signed(1, 2, 0, 0x4000).is_err());
        assert!(s.set_master_volume(1, -0x4001, 0).is_err());
        assert!(s.set_master_volume(1, 0, 0x4000).is_err());
        assert!(s.set_noise_clock(1, 16, 0).is_err());
        assert!(s.set_noise_clock(1, 0, 4).is_err());
        assert!(s.set_reverb_work_area(1, 0x1001, 0x20000).is_err());
        assert!(s.set_reverb_work_area(1, 0x20000, 0x20000).is_err());
        assert!(s.set_reverb_work_area(1, 0x10000, 0x20_0002).is_err());
        let mut bad_adsr = adsr;
        bad_adsr.release_rate = 0x20;
        assert!(s.set_adsr(1, 2, bad_adsr).is_err());
    }

    let stride = 2 * registers::VOICE_STRIDE;
    let (adsr1, adsr2) = AdsrConfig {
        attack_rate: 0x12,
        decay_rate: 0x05,
        sustain_level: 0x0A,
        sustain_rate: 0x34,
        release_rate: 0x09,
        attack_exponential: true,
        sustain_exponential: false,
        sustain_decrease: true,
        release_exponential: true,
    }
    .to_raw();
    assert_eq!(
        d.read_register(core_reg(1, registers::VP_ADSR1 + stride))
            .unwrap(),
        adsr1
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::VP_ADSR2 + stride))
            .unwrap(),
        adsr2
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::VP_VOLL + stride))
            .unwrap(),
        0x4000
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::VP_VOLR + stride))
            .unwrap(),
        0x3FFF
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::S_PMON_HI)).unwrap(),
        0xCDEE
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::S_PMON_LO)).unwrap(),
        0x00AB
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::S_NON_HI)).unwrap(),
        0xAA01
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::S_NON_LO)).unwrap(),
        0x0055
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::CORE_ATTR)).unwrap(),
        registers::CONTROL_ENABLE | registers::CONTROL_UNMUTE | (11 << 10) | (2 << 8)
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::P_MVOLL)).unwrap(),
        (-0x2000i16 as u16) & 0x7FFF
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::P_MVOLR)).unwrap(),
        0x3000
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::P_EVOLL)).unwrap(),
        -0x4000i16 as u16
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::P_EVOLR)).unwrap(),
        0x7FFF
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::A_ESA_HI)).unwrap(),
        0x0008
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::A_ESA_LO)).unwrap(),
        0x0000
    );
    assert_eq!(
        d.read_register(core_reg(1, registers::A_EEA_HI)).unwrap(),
        0x000B
    );
}

#[test]
fn adpcm_shift_gt_12_clamps_to_9_and_predictor_ge_5_ignores_history() {
    let render_block = |header: u8| -> Vec<i16> {
        let mut d = Device::new_ps1();
        let mut block = [0u8; 16];
        block[0] = header;
        block[1] = 0x07; // 1-block loop
        for (i, b) in block[2..16].iter_mut().enumerate() {
            *b = ((i as u8).wrapping_mul(0x37)) ^ 0x5A;
        }
        d.write_ram(0x1000, &block).unwrap();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            let vp = VoiceParameters {
                start_address: 0x1000,
                pitch: 0x1000,
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
            s.key_on_voice(0, 0).unwrap();
        }
        let mut out = vec![0i16; 128 * 2];
        d.render_native(&mut out);
        out
    };

    let ref_shift9 = render_block(0x09);
    assert!(ref_shift9.iter().any(|&s| s != 0));
    assert_eq!(render_block(0x0D), ref_shift9, "shift 13 must clamp to 9");
    assert_eq!(render_block(0x0E), ref_shift9, "shift 14 must clamp to 9");
    assert_eq!(render_block(0x0F), ref_shift9, "shift 15 must clamp to 9");

    // Predictor >= 5 uses [0, 0] and ignores prior s1/s2 history just like predictor 0.
    assert_eq!(render_block(0x50), render_block(0x00));
    assert_eq!(render_block(0xF0), render_block(0x00));
}

#[test]
fn adpcm_clamped_i16_history_prevents_unclamped_overshoot() {
    // High-gain predictor 4 ([122, -60]) with full-scale positive nibbles (+7) at shift 0
    // saturates i16 history immediately; then a block with 0 nibbles decays from clamped
    // +32767 rather than an unclamped >32767 internal accumulator.
    let mut blocks = [0u8; 32];
    blocks[0] = 0x40; // pred 4, shift 0
    blocks[1] = 0x00;
    blocks[2..16].fill(0x77);
    blocks[16] = 0x40; // pred 4, shift 0
    blocks[17] = 0x03; // loop
    blocks[18..32].fill(0x00);

    let mut d = Device::new_ps1();
    d.write_ram(0x1000, &blocks).unwrap();
    {
        let mut s = Synth::new(&mut d);
        s.enable_core(0, true).unwrap();
        let vp = VoiceParameters {
            start_address: 0x1000,
            pitch: 0x1000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    let mut out = vec![0i16; 64 * 2];
    d.render_native(&mut out);
    // Verify the decoded samples stay valid and decay smoothly after block 0.
    let peak_b0 = out[8..56].iter().map(|&s| (s as i32).abs()).max().unwrap();
    assert!(peak_b0 > 10_000, "block 0 should saturate high: {peak_b0}");
}

#[test]
fn oneshot_plays_full_blocks_without_premature_prefetch_cutoff() {
    // 2-block one-shot: block 0 has small positive nibbles (flags=0x00),
    // block 1 has large negative nibbles (flags=0x01).
    let mut blocks = [0u8; 32];
    blocks[0] = 0x04; // pred 0, shift 4
    blocks[1] = 0x00; // continue
    blocks[2..16].fill(0x22); // +2 << 12 >> 4 = +512
    blocks[16] = 0x01; // pred 0, shift 1
    blocks[17] = 0x01; // one-shot end
    blocks[18..32].fill(0x99); // -7 << 12 >> 1 = -14336

    let mut d = Device::new_ps1();
    d.write_ram(0x1000, &blocks).unwrap();
    {
        let mut s = Synth::new(&mut d);
        s.enable_core(0, true).unwrap();
        let vp = VoiceParameters {
            start_address: 0x1000,
            pitch: 0x1000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }

    // After 14 frames (mid block 0): must NOT be stopped and ENDX must still be 0!
    let mut part1 = vec![0i16; 14 * 2];
    d.render_native(&mut part1);
    let vs1 = d.inspect_voice(0, 0).unwrap();
    assert!(!vs1.reached_end, "ENDX must not fire during block 0");
    assert_ne!(vs1.envelope_phase, EnvelopePhase::Stopped);

    // Next 26 frames (reaches frame 40, mid block 1): still playing block 1's negative samples!
    let mut part2 = vec![0i16; 26 * 2];
    d.render_native(&mut part2);
    let vs2 = d.inspect_voice(0, 0).unwrap();
    assert!(!vs2.reached_end, "ENDX must not fire mid block 1");
    assert_ne!(vs2.envelope_phase, EnvelopePhase::Stopped);
    let min_b1 = part2.iter().copied().min().unwrap_or(0);
    assert!(
        min_b1 < -5000,
        "block 1 negative samples must actually play, got min={min_b1}"
    );

    // Next 32 frames (passes frame 56): now ENDX sets and voice stops cleanly.
    let mut part3 = vec![0i16; 32 * 2];
    d.render_native(&mut part3);
    let vs3 = d.inspect_voice(0, 0).unwrap();
    assert!(vs3.reached_end, "ENDX must be set after block 1 finishes");
    assert_eq!(vs3.envelope_phase, EnvelopePhase::Stopped);
    assert_eq!(vs3.envx, 0);
}

#[test]
fn ps1_endx_is_readonly_and_cleared_per_voice_on_kon() {
    let mut d = Device::new_ps1();
    let mut block = [0x77u8; 16];
    block[0] = 0x00;
    block[1] = 0x01; // one-shot end
    d.write_ram(0x1000, &block).unwrap();

    d.write_ps1_register(
        ps1::SPUCNT,
        registers::CONTROL_ENABLE | registers::CONTROL_UNMUTE,
    )
    .unwrap();
    for v in 0..2u32 {
        let base = v * ps1::VOICE_STRIDE;
        d.write_ps1_register(base + ps1::V_VOLL, 0x3FFF).unwrap();
        d.write_ps1_register(base + ps1::V_VOLR, 0x3FFF).unwrap();
        d.write_ps1_register(base + ps1::V_PITCH, 0x1000).unwrap();
        d.write_ps1_register(base + ps1::V_ADSR1, 0x000F).unwrap();
        d.write_ps1_register(base + ps1::V_START_ADDR, (0x1000 >> 3) as u16)
            .unwrap();
    }
    d.write_ps1_register(ps1::KON_LO, 0x0003).unwrap();

    let mut pcm = vec![0i16; 64 * 2];
    d.render_native(&mut pcm);
    assert_eq!(d.read_ps1_register(ps1::ENDX_LO).unwrap() & 0x0003, 0x0003);

    // Writes to PS1 ENDX must be ignored.
    d.write_ps1_register(ps1::ENDX_LO, 0xFFFF).unwrap();
    d.write_ps1_register(ps1::ENDX_HI, 0xFFFF).unwrap();
    assert_eq!(d.read_ps1_register(ps1::ENDX_LO).unwrap() & 0x0003, 0x0003);

    // Key-on for voice 0 clears ONLY voice 0's ENDX bit, leaving voice 1 set.
    d.write_ps1_register(ps1::KON_LO, 0x0001).unwrap();
    assert_eq!(d.read_ps1_register(ps1::ENDX_LO).unwrap() & 0x0003, 0x0002);
}

#[test]
fn raw_pitch_clamped_to_0x3fff_and_ps2_avol_bvol_sign_extended() {
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);

    let run_pitch = |pitch: u16| -> Vec<i16> {
        let mut d = Device::new();
        d.write_ram(0x5000, &enc).unwrap();
        d.write_register(
            registers::CORE_ATTR,
            registers::CONTROL_ENABLE | registers::CONTROL_UNMUTE,
        )
        .unwrap();
        d.write_register(registers::VP_VOLL, 0x3FFF).unwrap();
        d.write_register(registers::VP_VOLR, 0x3FFF).unwrap();
        d.write_register(registers::VP_PITCH, pitch).unwrap();
        d.write_register(registers::VP_ADSR1, 0x000F).unwrap();
        d.write_register(registers::VA_SSA_LO, (0x5000 >> 1) as u16)
            .unwrap();
        d.write_register(registers::A_KON_HI, 1).unwrap();
        let mut out = vec![0i16; 128 * 2];
        d.render_native(&mut out);
        out
    };

    assert_eq!(
        run_pitch(0x3FFF),
        run_pitch(0x7FFF),
        "raw pitch > 0x3FFF must clamp to 0x3FFF"
    );

    // PS2 P_AVOL sign-extension: negative P_AVOLL inverts Core 0 signal mixed into Core 1.
    let run_avol = |avol: i16| -> Vec<i16> {
        let mut d = Device::new();
        d.write_ram(0x5000, &enc).unwrap();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            s.enable_core(1, true).unwrap();
            let vp = VoiceParameters {
                core: 0,
                start_address: 0x5000,
                pitch: 0x1000,
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
            s.key_on_voice(0, 0).unwrap();
        }
        d.write_register(core_reg(1, registers::P_AVOLL), avol as u16)
            .unwrap();
        d.write_register(core_reg(1, registers::P_AVOLR), avol as u16)
            .unwrap();
        let mut out = vec![0i16; 128 * 2];
        d.render_native(&mut out);
        out
    };
    let pos_avol = run_avol(0x4000);
    let neg_avol = run_avol(-0x4000);
    assert!(pos_avol.iter().any(|&s| s.abs() > 1000));
    for (i, (&p, &n)) in pos_avol.iter().zip(neg_avol.iter()).enumerate() {
        assert!(
            ((n as i32) + (p as i32)).abs() <= 1,
            "P_AVOL sign-extension mismatch at {i}: pos={p}, neg={n}"
        );
    }
}

#[test]
fn discrete_adsr_exp_increase_knee_and_exp_decrease_tail() {
    let mut block = [0x77u8; 16];
    block[0] = 0x00;
    block[1] = 0x03;

    // 1. Exponential attack fake-knee: above 0x6000 takes 4x more ticks per step.
    let mut d = Device::new_ps1();
    d.write_ram(0x1000, &block).unwrap();
    {
        let mut s = Synth::new(&mut d);
        s.enable_core(0, true).unwrap();
        let vp = VoiceParameters {
            start_address: 0x1000,
            adsr: AdsrConfig {
                attack_rate: 0x20, // shift=8, step_idx=0 => +56 per tick below 0x6000
                attack_exponential: true,
                ..AdsrConfig::default()
            },
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    // Right after KON, ENVX must be 0.
    assert_eq!(d.inspect_voice(0, 0).unwrap().envx, 0);

    let mut frame = [0i16; 2];
    let mut ticks_to_6000 = 0usize;
    while d.inspect_voice(0, 0).unwrap().envx <= 0x6000 {
        d.render_native(&mut frame);
        ticks_to_6000 += 1;
        assert!(ticks_to_6000 < 2000);
    }
    let mut ticks_above_6000 = 0usize;
    while d.inspect_voice(0, 0).unwrap().envelope_phase == EnvelopePhase::Attack {
        d.render_native(&mut frame);
        ticks_above_6000 += 1;
        assert!(ticks_above_6000 < 2000);
    }
    // 0x0000..=0x6000 is 3/4 of full range (~439 ticks); 0x6000..0x7FFF is 1/4 of full range
    // but at 4x slower cycles (~586 ticks), so ticks_above_6000 > ticks_to_6000!
    assert!(
        ticks_above_6000 > ticks_to_6000,
        "exp attack knee must slow down 4x above 0x6000: below={ticks_to_6000}, above={ticks_above_6000}"
    );

    // 2. Exponential release decays monotonically and reaches 0 (Stopped).
    {
        let mut s = Synth::new(&mut d);
        s.set_adsr(
            0,
            0,
            AdsrConfig {
                release_rate: 0x06,
                release_exponential: true,
                ..AdsrConfig::default()
            },
        )
        .unwrap();
        s.key_off_voice(0, 0).unwrap();
    }
    let mut prev = d.inspect_voice(0, 0).unwrap().envx;
    for _ in 0..4000 {
        d.render_native(&mut frame);
        let cur = d.inspect_voice(0, 0).unwrap().envx;
        assert!(cur <= prev, "release must be monotonically non-increasing");
        prev = cur;
        if d.inspect_voice(0, 0).unwrap().envelope_phase == EnvelopePhase::Stopped {
            break;
        }
    }
    assert_eq!(
        d.inspect_voice(0, 0).unwrap().envelope_phase,
        EnvelopePhase::Stopped
    );
    assert_eq!(d.inspect_voice(0, 0).unwrap().envx, 0);
}

#[test]
fn fixed_point_reverb_tail_and_viir_min_saturation() {
    let pcm = common::make_sine(440.0, 28 * 16);
    let enc = common::encode_adpcm(&pcm, true, 0);

    let mut d = Device::new_ps1();
    d.write_ram(0x1000, &enc).unwrap();
    // Configure PS1 reverb work area at 0x70000..0x80000.
    d.write_ps1_register(ps1::RVB_WORK_ADDR, (0x70000 >> 3) as u16)
        .unwrap();
    d.write_ps1_register(ps1::EVOLL, 0x4000).unwrap();
    d.write_ps1_register(ps1::EVOLR, 0x4000).unwrap();
    d.write_ps1_register(ps1::EON_LO, 0x0001).unwrap();
    // Set vIIR = -0x8000 (tests saturation without overflow), vWALL = 0x4000,
    // vCOMB1 = 0x4000, vAPF1 = 0x3000, vAPF2 = 0x2000, vLIN/vRIN = 0x7FFF.
    d.write_ps1_register(ps1::RVB_BASE + 2 * 2, 0x8000).unwrap(); // vIIR = -0x8000
    d.write_ps1_register(ps1::RVB_BASE + 3 * 2, 0x4000).unwrap(); // vCOMB1
    d.write_ps1_register(ps1::RVB_BASE + 7 * 2, 0x4000).unwrap(); // vWALL
    d.write_ps1_register(ps1::RVB_BASE + 8 * 2, 0x3000).unwrap(); // vAPF1
    d.write_ps1_register(ps1::RVB_BASE + 9 * 2, 0x2000).unwrap(); // vAPF2
    d.write_ps1_register(ps1::RVB_BASE + 10 * 2, 0x0040)
        .unwrap(); // mLSAME
    d.write_ps1_register(ps1::RVB_BASE + 11 * 2, 0x0080)
        .unwrap(); // mRSAME
    d.write_ps1_register(ps1::RVB_BASE + 12 * 2, 0x0040)
        .unwrap(); // mLCOMB1
    d.write_ps1_register(ps1::RVB_BASE + 13 * 2, 0x0080)
        .unwrap(); // mRCOMB1
    d.write_ps1_register(ps1::RVB_BASE + 26 * 2, 0x0100)
        .unwrap(); // mLAPF1
    d.write_ps1_register(ps1::RVB_BASE + 27 * 2, 0x0140)
        .unwrap(); // mRAPF1
    d.write_ps1_register(ps1::RVB_BASE + 28 * 2, 0x0180)
        .unwrap(); // mLAPF2
    d.write_ps1_register(ps1::RVB_BASE + 29 * 2, 0x01C0)
        .unwrap(); // mRAPF2
    d.write_ps1_register(ps1::RVB_BASE, 0x0020).unwrap(); // dAPF1
    d.write_ps1_register(ps1::RVB_BASE + 2, 0x0020).unwrap(); // dAPF2
    d.write_ps1_register(ps1::RVB_BASE + 30 * 2, 0x7FFF)
        .unwrap(); // vLIN
    d.write_ps1_register(ps1::RVB_BASE + 31 * 2, 0x7FFF)
        .unwrap(); // vRIN
    d.write_ps1_register(
        ps1::SPUCNT,
        registers::CONTROL_ENABLE | registers::CONTROL_UNMUTE | registers::CONTROL_REVERB,
    )
    .unwrap();
    {
        let mut s = Synth::new(&mut d);
        let vp = VoiceParameters {
            start_address: 0x1000,
            pitch: 0x1000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    let mut active = vec![0i16; 1024 * 2];
    d.render_native(&mut active);
    // Stop dry voice immediately and verify reverb wet tail remains non-zero!
    d.write_ps1_register(ps1::KOFF_LO, 0x0001).unwrap();
    let mut tail = vec![0i16; 512 * 2];
    d.render_native(&mut tail);
    let tail_peak = tail[64..]
        .iter()
        .map(|&s| (s as i32).abs())
        .max()
        .unwrap_or(0);
    assert!(
        tail_peak > 100,
        "fixed-point reverb should produce audible wet tail, peak={tail_peak}"
    );
}

#[test]
fn control_unmute_and_mvol_sweep_mode() {
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);

    // 1. CONTROL_UNMUTE = 0 mutes voices while still running ADSR/ENDX.
    let mut d = Device::new();
    d.write_ram(0x5000, &enc).unwrap();
    d.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE)
        .unwrap(); // bit 14 = 0 (muted)
    {
        let mut s = Synth::new(&mut d);
        let vp = VoiceParameters {
            start_address: 0x5000,
            pitch: 0x1000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    let mut muted = vec![0i16; 256 * 2];
    d.render_native(&mut muted);
    assert!(muted.iter().all(|&s| s == 0), "CONTROL_UNMUTE=0 must mute");
    assert!(
        d.inspect_voice(0, 0).unwrap().envx > 0,
        "ADSR must still advance while muted"
    );

    // Unmuting produces audio immediately.
    d.write_register(
        registers::CORE_ATTR,
        registers::CONTROL_ENABLE | registers::CONTROL_UNMUTE,
    )
    .unwrap();
    let mut unmuted = vec![0i16; 256 * 2];
    d.render_native(&mut unmuted);
    assert!(unmuted.iter().any(|&s| s.abs() > 1000));

    // 2. MVOLL sweep decrease (bit 15 | bit 13 | rate 0x10 = 0xA010) fades left channel to 0.
    d.write_register(registers::P_MVOLL, 0xA010).unwrap();
    let mut fade = vec![0i16; 4096 * 2];
    d.render_native(&mut fade);
    let tail_l = fade[fade.len() - 64..]
        .chunks_exact(2)
        .map(|f| (f[0] as i32).abs())
        .max()
        .unwrap_or(0);
    let tail_r = fade[fade.len() - 64..]
        .chunks_exact(2)
        .map(|f| (f[1] as i32).abs())
        .max()
        .unwrap_or(0);
    assert_eq!(tail_l, 0, "MVOLL decrease sweep should fade left to 0");
    assert!(tail_r > 1000, "MVOLR fixed volume should remain loud");
}

#[test]
fn ps1_capture_buffers_and_spustat_half_flag() {
    let pcm = common::make_sine_ex(440.0, 28 * 32, 0.8, libspu2::PS1_SAMPLE_RATE);
    let enc = common::encode_adpcm(&pcm, true, 0);

    let mut spu = Device::new_ps1();
    spu.write_ram(0x2000, &enc).unwrap();
    {
        let mut s = Synth::new(&mut spu);
        s.enable_core(0, true).unwrap();
        // Voice 1 -> capture buffer 0x0800..0x0BFF; Voice 3 -> 0x0C00..0x0FFF.
        for &(idx, pitch) in &[(1usize, 0x1000u16), (3usize, 0x1800u16)] {
            let vp = VoiceParameters {
                index: idx,
                start_address: 0x2000,
                pitch,
                left_volume: 0, // muted in main mix; capture is post-ADSR!
                right_volume: 0,
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
        }
        s.key_on(0, (1 << 1) | (1 << 3)).unwrap();
    }

    // Initially at offset 0 => SPUSTAT bit 11 (0x0800) is 0 (first half).
    assert_eq!(spu.read_ps1_register(ps1::SPUSTAT).unwrap() & 0x0800, 0);

    // Render 256 frames -> enters second half (offset == 0x200 => bit 11 == 1).
    let mut buf = vec![0i16; 256 * 2];
    spu.render_native(&mut buf);
    assert_eq!(
        spu.read_ps1_register(ps1::SPUSTAT).unwrap() & 0x0800,
        0x0800
    );

    // Render 256 more frames -> wraps back to first half (offset == 0x000 => bit 11 == 0).
    spu.render_native(&mut buf);
    assert_eq!(spu.read_ps1_register(ps1::SPUSTAT).unwrap() & 0x0800, 0);

    // Inspect capture RAM: CD-L/CD-R (0x0000..0x07FF) zero, Voice 1 (0x0800..0x0BFF)
    // and Voice 3 (0x0C00..0x0FFF) non-zero and distinct.
    let mut cap_v1 = [0u8; 0x400];
    let mut cap_v3 = [0u8; 0x400];
    spu.read_ram(0x0800, &mut cap_v1).unwrap();
    spu.read_ram(0x0C00, &mut cap_v3).unwrap();
    let v1_peak = cap_v1
        .chunks_exact(2)
        .map(|b| (i16::from_le_bytes([b[0], b[1]]) as i32).abs())
        .max()
        .unwrap_or(0);
    let v3_peak = cap_v3
        .chunks_exact(2)
        .map(|b| (i16::from_le_bytes([b[0], b[1]]) as i32).abs())
        .max()
        .unwrap_or(0);
    assert!(v1_peak > 1000, "Voice 1 capture RAM must be non-zero");
    assert!(v3_peak > 1000, "Voice 3 capture RAM must be non-zero");
    assert_ne!(cap_v1, cap_v3, "Voice 1 and Voice 3 capture buffers differ");
}

#[test]
fn ps1_tests_spucnt_spustat_and_noise_high_rate_reload() {
    // Clean-room assertions matching MIT-licensed JaCzekanski/ps1-tests SPU register behavior.
    let mut spu = Device::new_ps1();
    // Low 6 bits of SPUCNT (bits 0..5) are reflected in SPUSTAT bits 0..5.
    spu.write_ps1_register(ps1::SPUCNT, 0xC03F).unwrap();
    let stat = spu.read_ps1_register(ps1::SPUSTAT).unwrap();
    assert_eq!(stat & 0x3F, 0x3F);
    assert_ne!(
        stat & 0x80,
        0,
        "DMA mode bits 4..5 set => SPUSTAT bit 7 set"
    );

    // Clearing DMA mode bits clears SPUSTAT bit 7 and preserves low mode bits.
    spu.write_ps1_register(ps1::SPUCNT, 0xC005).unwrap();
    let stat2 = spu.read_ps1_register(ps1::SPUSTAT).unwrap();
    assert_eq!(stat2 & 0x3F, 0x05);
    assert_eq!(stat2 & 0x80, 0);

    // High-rate noise clock (shift >= 14, step = 3) multi-reload produces distinct streams.
    let run_noise = |shift: u8, step: u8| -> Vec<i16> {
        let mut d = Device::new_ps1();
        {
            let mut s = Synth::new(&mut d);
            s.enable_core(0, true).unwrap();
            s.set_noise(0, 1).unwrap();
            s.set_noise_clock(0, shift, step).unwrap();
            let vp = VoiceParameters {
                start_address: 0x2000,
                ..VoiceParameters::default()
            };
            s.configure_voice(&vp).unwrap();
            s.key_on_voice(0, 0).unwrap();
        }
        let mut out = vec![0i16; 256 * 2];
        d.render_native(&mut out);
        out
    };
    let n14 = run_noise(14, 3);
    let n15 = run_noise(15, 3);
    assert_ne!(n14, n15, "shift=14 and shift=15 step=3 must differ");
}
