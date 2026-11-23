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
        assert_eq!(n as i32, -(p as i32), "sample {i}: pos={p}, neg={n}");
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
        assert_eq!(n as i32, -(p as i32), "sample {i}: full={p}, inv={n}");
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
                        registers::CONTROL_ENABLE | registers::CONTROL_REVERB,
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
        registers::CONTROL_ENABLE | (11 << 10) | (2 << 8)
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
