// SPDX-License-Identifier: BSD-2-Clause

mod common;

use libspu2::registers::{self, core_reg, ps1, VOICE_ADDR_STRIDE, VOICE_STRIDE};
use libspu2::{
    Device, DeviceMode, EnvelopePhase, Error, StereoI16, Synth, VoiceParameters, PS1_RAM_SIZE,
    PS1_SAMPLE_RATE, RAM_SIZE,
};

#[test]
fn api_lifecycle_and_errors() {
    let mut d = Device::new();
    assert_eq!(d.read_register(0x801), Err(Error::OutOfRange("register offset")));
    assert_eq!(d.read_register(0x003), Err(Error::OutOfRange("register offset")));
    assert_eq!(
        d.write_ram(RAM_SIZE as u32, b"x"),
        Err(Error::OutOfRange("ram write"))
    );
    d.write_register(registers::VP_PITCH, 0x1234).unwrap();
    assert_eq!(d.read_register(registers::VP_PITCH).unwrap(), 0x1234);
    d.reset();
    assert_eq!(d.read_register(registers::VP_PITCH).unwrap(), 0);
}

#[test]
fn inspect_voice_reflects_state() {
    let mut d = Device::new();
    let mut block = [0x77u8; 16];
    block[0] = 0;
    block[1] = 0x03;
    d.write_ram(0x5000, &block).unwrap();
    d.write_register(registers::CORE_ATTR, 0x8000).unwrap();
    d.write_register(registers::VP_PITCH, 0x1000).unwrap();
    d.write_register(registers::VP_ADSR1, 0x000F).unwrap();
    d.write_register(registers::VA_SSA_LO, 0x5000 >> 1).unwrap();
    d.write_register(registers::A_KON_HI, 1).unwrap();

    let vs = d.inspect_voice(0, 0).unwrap();
    assert_eq!(vs.envelope_phase, EnvelopePhase::KeyOn);
    assert_eq!(vs.pitch, 0x1000);
    assert_eq!(vs.start_address, 0x5000);

    let mut pcm = vec![0i16; 128 * 2];
    d.render_native(&mut pcm);
    let vs = d.inspect_voice(0, 0).unwrap();
    assert!(vs.envelope_phase >= EnvelopePhase::Attack);
    assert!(vs.envx > 0);
    assert_eq!(d.frame_counter(), 128);
    assert!(d.inspect_voice(2, 0).is_err());
    assert!(d.inspect_voice(0, 24).is_err());
}

#[test]
fn convenience_voice_encodes_and_plays() {
    let mut device = Device::new();
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);
    device.write_ram(0x5000, &enc).unwrap();
    device.write_register(registers::CORE_ATTR, 0x8000).unwrap();

    let vp = VoiceParameters {
        core: 0,
        index: 3,
        start_address: 0x5000,
        pitch: 0x1000,
        ..VoiceParameters::default()
    };

    {
        let mut synth = Synth::new(&mut device);
        synth.configure_voice(&vp).unwrap();
        synth.key_on(0, 1u32 << 3).unwrap();
    }

    assert_eq!(
        device
            .read_register(core_reg(0, registers::VP_PITCH + 3 * VOICE_STRIDE))
            .unwrap(),
        0x1000
    );
    assert_eq!(
        device
            .read_register(core_reg(0, registers::VP_ADSR1 + 3 * VOICE_STRIDE))
            .unwrap(),
        0x000F
    );
    assert_eq!(
        device
            .read_register(core_reg(0, registers::VA_SSA_HI + 3 * VOICE_ADDR_STRIDE))
            .unwrap(),
        0
    );
    assert_eq!(
        device
            .read_register(core_reg(0, registers::VA_SSA_LO + 3 * VOICE_ADDR_STRIDE))
            .unwrap(),
        0x5000 >> 1
    );

    let mut out = vec![StereoI16::default(); 1024];
    device.render_stereo(&mut out);
    let peak = out
        .iter()
        .map(|f| (f.left as i32).abs().max((f.right as i32).abs()))
        .max()
        .unwrap_or(0);
    assert!(peak > 1000);
}

#[test]
fn convenience_validates_ranges() {
    let mut device = Device::new();
    let mut synth = Synth::new(&mut device);
    let mut bad = VoiceParameters {
        core: 2,
        ..VoiceParameters::default()
    };
    assert!(synth.configure_voice(&bad).is_err());
    bad.core = 0;
    bad.index = 24;
    assert!(synth.configure_voice(&bad).is_err());
    bad.index = 0;
    bad.start_address = 0x0020_0001;
    assert!(synth.configure_voice(&bad).is_err());
}

#[test]
fn ps1_spu_mode_plays_voice_and_enforces_512k_ram() {
    let mut spu = Device::new_ps1();
    assert_eq!(spu.mode(), DeviceMode::Ps1Spu);
    assert_eq!(spu.ram_size(), PS1_RAM_SIZE);
    assert_eq!(spu.sample_rate(), PS1_SAMPLE_RATE);

    // 512 KiB RAM bounds
    assert!(spu.write_ram(PS1_RAM_SIZE as u32, &[0]).is_err());
    assert!(spu.write_ram((PS1_RAM_SIZE - 1) as u32, &[0]).is_ok());

    let pcm = common::make_sine_ex(440.0, 28 * 32, 0.8, PS1_SAMPLE_RATE);
    let enc = common::encode_adpcm(&pcm, true, 0);
    spu.write_ram(0x1000, &enc).unwrap();

    spu.write_ps1_register(ps1::SPUCNT, 0x8000).unwrap();
    spu.write_ps1_register(ps1::V_VOLL, 0x3FFF).unwrap();
    spu.write_ps1_register(ps1::V_VOLR, 0x3FFF).unwrap();
    spu.write_ps1_register(ps1::V_PITCH, 0x1000).unwrap();
    spu.write_ps1_register(ps1::V_START_ADDR, (0x1000 >> 3) as u16)
        .unwrap();
    spu.write_ps1_register(ps1::V_ADSR1, 0x000F).unwrap();
    spu.write_ps1_register(ps1::V_ADSR2, 0x0000).unwrap();
    assert_eq!(
        spu.read_ps1_register(ps1::V_START_ADDR).unwrap(),
        (0x1000 >> 3) as u16
    );
    spu.write_ps1_register(ps1::KON_LO, 0x0001).unwrap();

    let mut out = vec![0i16; 1024 * 2];
    spu.render_native(&mut out);
    let peak = out.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    assert!(peak > 1000);
    assert_eq!(spu.frame_counter(), 1024);

    // PS1 V_ENVX round-trip (stored in top 16 bits of internal 32-bit ADSR volume).
    spu.write_ps1_register(ps1::V_ENVX, 0x4321).unwrap();
    assert_eq!(spu.read_ps1_register(ps1::V_ENVX).unwrap(), 0x4321);

    // PS1 mode exposes only Core 0.
    assert_eq!(spu.core_count(), 1);
    assert!(spu.inspect_voice(1, 0).is_err());
    let mut buf = [0i16; 8];
    assert!(spu.render_core(1, &mut buf).is_err());
    let mut synth = Synth::new(&mut spu);
    assert!(synth.key_on(1, 1).is_err());
    assert!(synth.key_off(1, 1).is_err());
    let bad_vp = VoiceParameters {
        core: 1,
        ..VoiceParameters::default()
    };
    assert!(synth.configure_voice(&bad_vp).is_err());
    // PS1 mode also rejects Core 1 SPU2 registers (both < 0x760 and volume block).
    assert!(spu
        .read_register(core_reg(1, registers::VP_PITCH))
        .is_err());
    assert!(spu
        .write_register(core_reg(1, registers::VP_PITCH), 0x1000)
        .is_err());
    assert!(spu
        .read_register(core_reg(1, registers::P_MVOLL))
        .is_err());
    assert!(spu
        .write_register(core_reg(1, registers::P_MVOLL), 0x3FFF)
        .is_err());
    assert!(spu.receive_dma(1, &mut [0u8; 16], 16, 1).is_err());
}

#[test]
fn device_is_send_sync_and_debug() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Device>();

    let mut d = Device::new();
    let dbg = format!("{d:?}");
    assert!(dbg.contains("Device"));
    assert!(dbg.contains("Spu2"));
    let synth = Synth::new(&mut d);
    let sdbg = format!("{synth:?}");
    assert!(sdbg.contains("Synth"));
}

#[test]
fn reset_syncs_base_sampling_rate_without_core_attr_write() {
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);

    // Device A: configure & play WITHOUT writing CORE_ATTR first.
    let mut a = Device::new();
    a.write_ram(0x5000, &enc).unwrap();
    {
        let mut s = Synth::new(&mut a);
        let vp = VoiceParameters {
            start_address: 0x5000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    let mut out_a = vec![0i16; 512 * 2];
    a.render_native(&mut out_a);

    // Device B: write CORE_ATTR before configuring.
    let mut b = Device::new();
    b.write_ram(0x5000, &enc).unwrap();
    b.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE)
        .unwrap();
    {
        let mut s = Synth::new(&mut b);
        let vp = VoiceParameters {
            start_address: 0x5000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    let mut out_b = vec![0i16; 512 * 2];
    b.render_native(&mut out_b);

    assert_eq!(out_a, out_b);
}

#[test]
fn spu2_envx_roundtrip_and_loop_clear_and_adsr_default() {
    assert_eq!(libspu2::AdsrConfig::default().sustain_rate, 0x7F);

    let mut d = Device::new();
    d.write_register(registers::VP_ENVX, 0x5678).unwrap();
    assert_eq!(d.read_register(registers::VP_ENVX).unwrap(), 0x5678);

    // Configure with explicit loop address, then reconfigure with loop_address: None.
    {
        let mut s = Synth::new(&mut d);
        let vp_loop = VoiceParameters {
            start_address: 0x5000,
            loop_address: Some(0x6000),
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp_loop).unwrap();
    }
    assert_eq!(d.inspect_voice(0, 0).unwrap().repeat_address, 0x6000);

    {
        let mut s = Synth::new(&mut d);
        let vp_no_loop = VoiceParameters {
            start_address: 0x5000,
            loop_address: None,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp_no_loop).unwrap();
    }
    assert_eq!(d.inspect_voice(0, 0).unwrap().repeat_address, 0);
}

#[test]
fn render_core_advances_frame_counter_and_planar_f32_matches_native() {
    let mut d1 = Device::new();
    let mut d2 = Device::new();
    let pcm = common::make_sine(440.0, 28 * 16);
    let enc = common::encode_adpcm(&pcm, true, 0);
    for d in [&mut d1, &mut d2] {
        d.write_ram(0x5000, &enc).unwrap();
        let mut s = Synth::new(d);
        s.enable_core(0, true).unwrap();
        let vp = VoiceParameters {
            start_address: 0x5000,
            left_volume: 0x3FFF,
            right_volume: 0x1FFF,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }

    let mut interleaved = vec![0i16; 300 * 2];
    d1.render_native(&mut interleaved);

    let mut left = vec![0.0f32; 300];
    let mut right = vec![0.0f32; 300];
    d2.render_f32_planar(&mut left, &mut right).unwrap();
    for i in 0..300 {
        assert_eq!(left[i], interleaved[i * 2] as f32 / 32768.0);
        assert_eq!(right[i], interleaved[i * 2 + 1] as f32 / 32768.0);
    }
    assert!(d2.render_f32_planar(&mut left[..10], &mut right).is_err());

    let mut core_buf = vec![0i16; 64 * 2];
    d1.render_core(0, &mut core_buf).unwrap();
    assert_eq!(d1.frame_counter(), 364);
}

#[test]
fn synth_live_controls_and_dma_and_non_destructive_irq_peek() {
    let mut d = Device::new();

    // DMA write + readback via receive_dma
    let payload = [0xABu8; 32];
    let mut tx = payload;
    d.write_register(registers::A_TSA_HI, 0).unwrap();
    d.write_register(registers::A_TSA_LO, (0x5000u32 >> 1) as u16)
        .unwrap();
    d.write_register(
        registers::CORE_ATTR,
        registers::CONTROL_ENABLE | registers::CONTROL_DMA_WRITE,
    )
    .unwrap();
    let written = d.receive_dma(0, &mut tx, 16, 2).unwrap();
    assert_eq!(written, 2);
    let mut verify = [0u8; 32];
    d.read_ram(0x5000, &mut verify).unwrap();
    assert_eq!(verify, payload);
    assert!(d.receive_dma(0, &mut tx[..8], 16, 2).is_err());

    // Live Synth controls
    {
        let mut s = Synth::new(&mut d);
        s.enable_core(0, true).unwrap();
        s.set_pitch(0, 2, 0x2000).unwrap();
        s.set_volume(0, 2, 0x1234, 0x2345).unwrap();
        s.set_reverb_send(0, 0x0000_0004).unwrap();
        s.key_on_voice(0, 2).unwrap();
        s.key_off_voice(0, 2).unwrap();
    }
    assert_eq!(d.inspect_voice(0, 2).unwrap().pitch, 0x2000);

    // Non-destructive irq_pending vs C_IRQINFO clear
    let mut block = [0x77u8; 16];
    block[0] = 0;
    block[1] = 0x03;
    d.write_ram(0x6000, &block).unwrap();
    d.write_register(registers::A_IRQA_HI, 0).unwrap();
    d.write_register(registers::A_IRQA_LO, (0x6000u32 >> 1) as u16)
        .unwrap();
    d.write_register(
        registers::CORE_ATTR,
        registers::CONTROL_ENABLE | registers::CONTROL_IRQ,
    )
    .unwrap();
    {
        let mut s = Synth::new(&mut d);
        let vp = VoiceParameters {
            start_address: 0x6000,
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    let mut pcm = vec![0i16; 128 * 2];
    d.render_native(&mut pcm);
    assert_eq!(d.irq_pending(), 1 << 2);
    // Still set on second peek!
    assert_eq!(d.irq_pending(), 1 << 2);
    // Reading C_IRQINFO reports and clears it.
    assert_eq!(d.read_register(registers::C_IRQINFO).unwrap(), 1 << 2);
    assert_eq!(d.irq_pending(), 0);
}
