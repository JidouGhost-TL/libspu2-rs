// SPDX-License-Identifier: BSD-2-Clause

mod common;

use libspu2::registers::{self, ps1};
use libspu2::{
    AdsrConfig, Device, EnvelopePhase, Error, StereoI16, Synth, VoiceParameters, RAM_SIZE,
};

#[test]
fn ps1_registers_reverb_transfer_irq_and_spustat() {
    let mut spu = Device::new_ps1();

    // Error bounds on PS1 register space
    assert!(spu.read_ps1_register(ps1::REGISTER_SPACE_SIZE).is_err());
    assert!(spu.read_ps1_register(1).is_err());
    assert!(spu.write_ps1_register(ps1::REGISTER_SPACE_SIZE, 0).is_err());
    assert!(spu.write_ps1_register(3, 0).is_err());

    // Voice 0 register round-trips
    spu.write_ps1_register(ps1::V_VOLL, 0x2222).unwrap();
    spu.write_ps1_register(ps1::V_VOLR, 0x3333).unwrap();
    spu.write_ps1_register(ps1::V_PITCH, 0x1234).unwrap();
    spu.write_ps1_register(ps1::V_START_ADDR, 0x0200).unwrap();
    spu.write_ps1_register(ps1::V_ADSR1, 0x8123).unwrap();
    spu.write_ps1_register(ps1::V_ADSR2, 0xC456).unwrap();
    spu.write_ps1_register(ps1::V_REPEAT_ADDR, 0x0300).unwrap();
    assert_eq!(spu.read_ps1_register(ps1::V_VOLL).unwrap(), 0x2222);
    assert_eq!(spu.read_ps1_register(ps1::V_VOLR).unwrap(), 0x3333);
    assert_eq!(spu.read_ps1_register(ps1::V_PITCH).unwrap(), 0x1234);
    assert_eq!(spu.read_ps1_register(ps1::V_START_ADDR).unwrap(), 0x0200);
    assert_eq!(spu.read_ps1_register(ps1::V_ADSR1).unwrap(), 0x8123);
    assert_eq!(spu.read_ps1_register(ps1::V_ADSR2).unwrap(), 0xC456);
    assert_eq!(spu.read_ps1_register(ps1::V_REPEAT_ADDR).unwrap(), 0x0300);

    // PS1 reverb work area & all 32 reverb parameter registers
    spu.write_ps1_register(ps1::RVB_WORK_ADDR, 0xE000).unwrap();
    assert_eq!(spu.read_ps1_register(ps1::RVB_WORK_ADDR).unwrap(), 0xE000);
    for i in 0..32u32 {
        let off = ps1::RVB_BASE + i * 2;
        let val = (0x0100 + i) as u16;
        spu.write_ps1_register(off, val).unwrap();
        assert_eq!(spu.read_ps1_register(off).unwrap(), val);
    }

    // EON (reverb enable), KON_HI, KOFF_LO/HI
    spu.write_ps1_register(ps1::EON_LO, 0xA5A5).unwrap();
    spu.write_ps1_register(ps1::EON_HI, 0x005A).unwrap();
    assert_eq!(spu.read_ps1_register(ps1::EON_LO).unwrap(), 0xA5A5);
    assert_eq!(spu.read_ps1_register(ps1::EON_HI).unwrap(), 0x005A);
    spu.write_ps1_register(ps1::KON_HI, 0x0001).unwrap();
    spu.write_ps1_register(ps1::KOFF_LO, 0x0001).unwrap();
    spu.write_ps1_register(ps1::KOFF_HI, 0x0001).unwrap();

    // Manual RAM transfer via TRANSFER_ADDR / TRANSFER_DATA
    spu.write_ps1_register(ps1::TRANSFER_ADDR, (0x2000 >> 3) as u16)
        .unwrap();
    assert_eq!(
        spu.read_ps1_register(ps1::TRANSFER_ADDR).unwrap(),
        (0x2000 >> 3) as u16
    );
    spu.write_ps1_register(ps1::TRANSFER_DATA, 0xBEEF).unwrap();
    let mut ram_bytes = [0u8; 2];
    spu.read_ram(0x2000, &mut ram_bytes).unwrap();
    assert_eq!(u16::from_le_bytes(ram_bytes), 0xBEEF);

    // Empty RAM read/write are no-ops
    spu.read_ram(0, &mut []).unwrap();
    spu.write_ram(0, &[]).unwrap();

    // PS1 IRQ + SPUSTAT + ENDX (3-block one-shot so block 2 triggers ENDX after prefetch)
    let mut one_shot = [0x77u8; 48];
    one_shot[0] = 0x00;
    one_shot[1] = 0x00;
    one_shot[16] = 0x00;
    one_shot[17] = 0x00;
    one_shot[32] = 0x00;
    one_shot[33] = 0x01; // block 2: end, no loop
    spu.write_ram(0x1000, &one_shot).unwrap();
    spu.write_ps1_register(ps1::IRQ_ADDR, (0x1000 >> 3) as u16)
        .unwrap();
    assert_eq!(
        spu.read_ps1_register(ps1::IRQ_ADDR).unwrap(),
        (0x1000 >> 3) as u16
    );
    spu.write_ps1_register(
        ps1::SPUCNT,
        registers::CONTROL_ENABLE | registers::CONTROL_IRQ | registers::CONTROL_DMA_WRITE,
    )
    .unwrap();
    assert_eq!(
        spu.read_ps1_register(ps1::SPUCNT).unwrap(),
        registers::CONTROL_ENABLE | registers::CONTROL_IRQ | registers::CONTROL_DMA_WRITE
    );
    spu.write_ps1_register(ps1::V_START_ADDR, (0x1000 >> 3) as u16)
        .unwrap();
    spu.write_ps1_register(ps1::V_PITCH, 0x1000).unwrap();
    spu.write_ps1_register(ps1::V_ADSR1, 0x000F).unwrap();
    spu.write_ps1_register(ps1::V_ADSR2, 0x0000).unwrap();
    spu.write_ps1_register(ps1::KON_LO, 0x0001).unwrap();

    let mut pcm = vec![0i16; 256 * 2];
    spu.render_native(&mut pcm);
    assert_eq!(spu.irq_pending(), 1 << 2);
    let stat = spu.read_ps1_register(ps1::SPUSTAT).unwrap();
    assert_ne!(stat & 0x40, 0, "IRQ pending bit should be set in SPUSTAT");
    assert_ne!(stat & 0x80, 0, "DMA bit should be set in SPUSTAT");
    assert_ne!(spu.read_ps1_register(ps1::ENDX_LO).unwrap() & 1, 0);
    assert_eq!(spu.read_ps1_register(ps1::ENDX_HI).unwrap(), 0);
    // PS1 ENDX is read-only: writes are ignored; cleared per-voice on KON.
    spu.write_ps1_register(ps1::ENDX_LO, 0xFFFF).unwrap();
    assert_ne!(spu.read_ps1_register(ps1::ENDX_LO).unwrap() & 1, 0);
    spu.write_ps1_register(ps1::KON_LO, 0x0001).unwrap();
    assert_eq!(spu.read_ps1_register(ps1::ENDX_LO).unwrap() & 1, 0);
    // Unknown PS1 offset returns 0 and ignores writes
    assert_eq!(spu.read_ps1_register(0x1F0).unwrap(), 0x0118); // inside RVB
    assert_eq!(spu.read_ps1_register(0x240).unwrap(), 0);
    spu.write_ps1_register(0x240, 0x1234).unwrap();
}

#[test]
fn autodma_block_streaming_and_dma_read_and_spdif_bypass() {
    let mut d = Device::default();

    // 1. DMA stopped mode returns 0 blocks
    let mut buf = [0u8; 64];
    d.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE)
        .unwrap();
    assert_eq!(d.receive_dma(0, &mut buf, 16, 2).unwrap(), 0);

    // 2. DMA write wrapping across the end of 2 MiB RAM, then DMA readback
    let wrap_addr = (RAM_SIZE as u32) - 16;
    d.write_register(registers::A_TSA_HI, ((wrap_addr >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(registers::A_TSA_LO, ((wrap_addr >> 1) & 0xFFFF) as u16)
        .unwrap();
    assert_eq!(
        d.read_register(registers::A_TSA_HI).unwrap(),
        ((wrap_addr >> 17) & 0xFFFF) as u16
    );
    d.write_register(
        registers::CORE_ATTR,
        registers::CONTROL_ENABLE | registers::CONTROL_DMA_WRITE,
    )
    .unwrap();
    let mut tx = [0x5Au8; 32];
    assert_eq!(d.receive_dma(0, &mut tx, 16, 2).unwrap(), 2);

    // Read back via DMA read mode (0x0030)
    d.write_register(registers::A_TSA_HI, ((wrap_addr >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(registers::A_TSA_LO, ((wrap_addr >> 1) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE | 0x0030)
        .unwrap();
    let mut rx = [0u8; 32];
    assert_eq!(d.receive_dma(0, &mut rx, 16, 2).unwrap(), 2);
    assert_eq!(rx, tx);

    // 3. AutoDMA block input streaming on Core 0 (TRANSFER_MODE_BLOCK_CORE0IN = 1)
    d.reset();
    d.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE)
        .unwrap();
    d.write_register(registers::A_TS_MODE, 0x0001).unwrap();
    assert_eq!(d.read_register(registers::A_TS_MODE).unwrap(), 0x0001);
    d.write_register(registers::P_BVOLL, 0x7FFF).unwrap();
    d.write_register(registers::P_BVOLR, 0x7FFF).unwrap();
    assert_eq!(d.read_register(registers::P_BVOLL).unwrap(), 0x7FFF);
    assert_eq!(d.read_register(registers::P_BVOLR).unwrap(), 0x7FFF);
    d.write_register(registers::P_AVOLL, 0x4000).unwrap();
    d.write_register(registers::P_AVOLR, 0x4000).unwrap();
    assert_eq!(d.read_register(registers::P_AVOLL).unwrap(), 0x4000);
    assert_eq!(d.read_register(registers::P_AVOLR).unwrap(), 0x4000);

    // Feed 1024 bytes (SOUND_INPUT_DATA_SIZE) of non-zero stereo 16-bit PCM into AutoDMA
    let mut block_pcm = vec![0u8; 1024];
    for i in 0..512 {
        let sample: i16 = if i % 2 == 0 { 12000 } else { -12000 };
        let bytes = sample.to_le_bytes();
        block_pcm[i * 2] = bytes[0];
        block_pcm[i * 2 + 1] = bytes[1];
    }
    let blocks = d.receive_dma(0, &mut block_pcm, 512, 2).unwrap();
    assert_eq!(blocks, 2);

    let mut out = vec![0i16; 128 * 2];
    d.render_native(&mut out);
    assert!(
        out.iter().any(|&s| s != 0),
        "AutoDMA block input should produce non-zero PCM"
    );

    // S/PDIF global registers & bypass flag
    d.write_register(registers::C_SPDIF_OUT, 0x0100).unwrap();
    d.write_register(registers::C_SPDIF_MODE, 0x0042).unwrap();
    d.write_register(registers::C_SPDIF_MEDIA, 0x0084).unwrap();
    assert_eq!(d.read_register(registers::C_SPDIF_OUT).unwrap(), 0x0100);
    assert_eq!(d.read_register(registers::C_SPDIF_MODE).unwrap(), 0x0042);
    assert_eq!(d.read_register(registers::C_SPDIF_MEDIA).unwrap(), 0x0084);
    assert_eq!(d.read_register(0x7F0).unwrap(), 0);
    d.write_register(0x7F0, 0x1234).unwrap();
    // Render another block with S/PDIF bypass enabled
    let mut block_pcm2 = vec![0x20u8; 1024];
    let _ = d.receive_dma(0, &mut block_pcm2, 512, 2).unwrap();
    d.render_native(&mut out);
}

#[test]
fn spu2_reverb_registers_and_all_envelope_phases() {
    let mut d = Device::new();

    // Write all SPU2 reverb address & coefficient registers + readback ESA/EEA
    d.write_register(registers::A_ESA_HI, 0x0008).unwrap();
    d.write_register(registers::A_ESA_LO, 0x0000).unwrap();
    d.write_register(registers::A_EEA_HI, 0x000F).unwrap();
    assert_eq!(d.read_register(registers::A_ESA_HI).unwrap(), 0x0008);
    assert_eq!(d.read_register(registers::A_ESA_LO).unwrap(), 0x0000);
    assert_eq!(d.read_register(registers::A_EEA_HI).unwrap(), 0x000F);
    for off in (registers::RVB_ADDR_BASE..registers::RVB_ADDR_END).step_by(2) {
        d.write_register(off, 0x0010).unwrap();
    }
    for off in (registers::RVB_COEF_BASE..registers::RVB_COEF_END).step_by(2) {
        d.write_register(off, 0x1000).unwrap();
    }

    // Drive a voice through Attack -> Decay -> Sustain -> Release -> Stopped
    // using exponential attack, decreasing exponential sustain, and exponential release.
    let pcm = common::make_sine(440.0, 28 * 32);
    let enc = common::encode_adpcm(&pcm, true, 0);
    d.write_ram(0x5000, &enc).unwrap();

    let mut phases_seen = [false; 6];
    {
        let mut s = Synth::new(&mut d);
        s.enable_core(0, true).unwrap();
        let vp = VoiceParameters {
            core: 0,
            index: 18, // also exercises high voice mask (16..23)
            start_address: 0x5000,
            adsr: AdsrConfig {
                attack_rate: 0x20,
                decay_rate: 0x04,
                sustain_level: 0x04,
                sustain_rate: 0x20,
                release_rate: 0x04,
                attack_exponential: true,
                sustain_exponential: true,
                sustain_decrease: true,
                release_exponential: true,
            },
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 18).unwrap();
        phases_seen[s.device().inspect_voice(0, 18).unwrap().envelope_phase as usize] = true;

        let mut buf = [0i16; 16 * 2];
        for _ in 0..800 {
            s.render_native(&mut buf);
            let p = s.device().inspect_voice(0, 18).unwrap().envelope_phase;
            phases_seen[p as usize] = true;
            if p == EnvelopePhase::Sustain {
                break;
            }
        }
        s.key_off_voice(0, 18).unwrap();
        let mut frames = [StereoI16::default(); 16];
        for _ in 0..800 {
            s.render_stereo(&mut frames);
            let p = s.device().inspect_voice(0, 18).unwrap().envelope_phase;
            phases_seen[p as usize] = true;
            if p == EnvelopePhase::Stopped {
                break;
            }
        }
        // Exercise disable core & device_mut
        s.enable_core(0, false).unwrap();
        assert_eq!(s.device_mut().core_count(), 2);
    }

    assert!(phases_seen[EnvelopePhase::KeyOn as usize]);
    assert!(phases_seen[EnvelopePhase::Attack as usize]);
    assert!(phases_seen[EnvelopePhase::Decay as usize]);
    assert!(phases_seen[EnvelopePhase::Sustain as usize]);
    assert!(phases_seen[EnvelopePhase::Release as usize]);
    assert!(phases_seen[EnvelopePhase::Stopped as usize]);
}

#[test]
fn synth_error_branches_and_error_display() {
    let e1 = Error::InvalidArgument("pitch");
    let e2 = Error::OutOfRange("core");
    assert!(e1.to_string().contains("invalid argument"));
    assert!(e2.to_string().contains("out of range"));

    let mut d = Device::new();
    let mut s = Synth::new(&mut d);

    // Invalid volume (above +0x3FFF and below -0x4000)
    let mut vp = VoiceParameters {
        left_volume: 0x4000,
        ..VoiceParameters::default()
    };
    assert!(s.configure_voice(&vp).is_err());
    vp.left_volume = -0x4001;
    assert!(s.configure_voice(&vp).is_err());
    // Valid negative (phase-inverted) volume
    vp.left_volume = -0x4000;
    vp.right_volume = -0x2000;
    assert!(s.configure_voice(&vp).is_ok());
    assert_eq!(
        s.device().inspect_voice(0, 0).unwrap().volume_left,
        i32::MIN
    );

    // Invalid pitch
    vp.left_volume = 0x3FFF;
    vp.pitch = 0x4000;
    assert!(s.configure_voice(&vp).is_err());
    assert!(s.set_pitch(0, 0, 0x4000).is_err());
    assert!(s.set_volume(0, 0, 0x4000, 0).is_err());

    // Invalid ADSR
    vp.pitch = 0x1000;
    vp.adsr.attack_rate = 0x80;
    assert!(s.configure_voice(&vp).is_err());

    // Unaligned loop address
    vp.adsr.attack_rate = 0;
    vp.loop_address = Some(0x5004);
    assert!(s.configure_voice(&vp).is_err());

    // Odd render_core buffer
    assert!(d.render_core(0, &mut [0i16; 3]).is_err());
    assert!(d.write_register(0x801, 0).is_err());
}

#[test]
fn sample_cache_eviction_under_large_working_set() {
    let mut d = Device::new();
    // Write 4200 distinct ADPCM blocks (exceeds MAX_CACHE_ENTRIES = 4096) and play through them
    let block_count = 4200usize;
    let mut adpcm = vec![0u8; block_count * 16];
    for i in 0..block_count {
        adpcm[i * 16] = 0x00;
        adpcm[i * 16 + 1] = if i + 1 == block_count { 0x01 } else { 0x00 };
        adpcm[i * 16 + 2] = (i & 0xFF) as u8;
    }
    d.write_ram(0x5000, &adpcm).unwrap();
    d.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE)
        .unwrap();
    {
        let mut s = Synth::new(&mut d);
        let vp = VoiceParameters {
            start_address: 0x5000,
            pitch: 0x3FFF, // ~4x speed so 4200 blocks decode quickly
            ..VoiceParameters::default()
        };
        s.configure_voice(&vp).unwrap();
        s.key_on_voice(0, 0).unwrap();
    }
    let mut buf = vec![0i16; 1024 * 2];
    for _ in 0..40 {
        d.render_native(&mut buf);
        if d.inspect_voice(0, 0).unwrap().reached_end {
            break;
        }
    }
    assert!(d.inspect_voice(0, 0).unwrap().reached_end);
}

#[test]
fn round2_polish_dma_straddle_wrap_ram_bounds_adsr_raw_and_output_rate() {
    // 1. Empty-slice RAM bounds check
    let mut d = Device::new();
    assert!(d.read_ram(0xFFFF_FFFF, &mut []).is_err());
    assert!(d.write_ram(0xFFFF_FFFF, &[]).is_err());

    // 2. Single DMA block straddling end of 2 MiB RAM wraps cleanly on both write & read
    let straddle = (RAM_SIZE as u32) - 8;
    d.write_register(registers::A_TSA_HI, ((straddle >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(registers::A_TSA_LO, ((straddle >> 1) & 0xFFFF) as u16)
        .unwrap();
    assert_eq!(
        d.read_register(registers::A_TSA_LO).unwrap(),
        ((straddle >> 1) & 0xFFFF) as u16
    );
    d.write_register(
        registers::CORE_ATTR,
        registers::CONTROL_ENABLE | registers::CONTROL_DMA_WRITE,
    )
    .unwrap();
    let mut tx = [0u8; 32];
    for (i, b) in tx.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7).wrapping_add(1);
    }
    assert_eq!(d.receive_dma(0, &mut tx, 32, 1).unwrap(), 1);
    // Tail 24 bytes should have wrapped to RAM[0..24]
    let mut wrapped_head = [0u8; 24];
    d.read_ram(0, &mut wrapped_head).unwrap();
    assert_eq!(&wrapped_head, &tx[8..32]);
    // DMA read across the same straddle boundary
    d.write_register(registers::A_TSA_HI, ((straddle >> 17) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(registers::A_TSA_LO, ((straddle >> 1) & 0xFFFF) as u16)
        .unwrap();
    d.write_register(
        registers::CORE_ATTR,
        registers::CONTROL_ENABLE | registers::CONTROL_DMA_READ,
    )
    .unwrap();
    let mut rx = [0u8; 32];
    assert_eq!(d.receive_dma(0, &mut rx, 32, 1).unwrap(), 1);
    assert_eq!(rx, tx);

    // 3. Unhandled transfer mode returns 0 blocks (even with block_amount == 0)
    d.write_register(registers::A_TS_MODE, registers::TRANSFER_MODE_BLOCK_READ)
        .unwrap();
    assert_eq!(d.receive_dma(0, &mut rx, 16, 0).unwrap(), 0);
    assert_eq!(d.receive_dma(0, &mut rx, 16, 1).unwrap(), 0);

    // 4. AdsrConfig::from_raw / to_raw roundtrip (bit 13 of ADSR2 is unused)
    let cfg = AdsrConfig::from_raw(0xABCD, 0xCF12);
    assert_eq!(cfg.to_raw(), (0xABCD, 0xCF12));

    // 5. Device::set_output_sample_rate + Synth::render_f32_planar
    let mut ps1 = Device::new_ps1();
    assert!(ps1.set_output_sample_rate(0).is_err());
    ps1.set_output_sample_rate(48_000).unwrap();
    assert_eq!(ps1.sample_rate(), 48_000);
    let mut l = [0.0f32; 32];
    let mut r = [0.0f32; 32];
    {
        let mut s = Synth::new(&mut ps1);
        s.render_f32_planar(&mut l, &mut r).unwrap();
    }
    ps1.reset();
    assert_eq!(ps1.sample_rate(), libspu2::PS1_SAMPLE_RATE);

    // 6. Extreme low output sample rate + max pitch never panics in SampleReader::get_sample
    let mut low_rate = Device::new();
    let mut blk = [0x77u8; 16];
    blk[0] = 0;
    blk[1] = 0x03;
    low_rate.write_ram(0x5000, &blk).unwrap();
    low_rate.set_output_sample_rate(1_000).unwrap();
    low_rate
        .write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE)
        .unwrap();
    low_rate.write_register(registers::VP_VOLL, 0x3FFF).unwrap();
    low_rate.write_register(registers::VP_VOLR, 0x3FFF).unwrap();
    low_rate
        .write_register(registers::VP_PITCH, 0xFFFF)
        .unwrap();
    low_rate
        .write_register(registers::VP_ADSR1, 0x000F)
        .unwrap();
    low_rate
        .write_register(registers::VA_SSA_LO, (0x5000u32 >> 1) as u16)
        .unwrap();
    low_rate
        .write_register(registers::VA_LSAX_LO, (0x5000u32 >> 1) as u16)
        .unwrap();
    low_rate.write_register(registers::A_KON_HI, 1).unwrap();
    let mut low_pcm = [0i16; 32 * 2];
    low_rate.render_native(&mut low_pcm);

    // 7. RAM-wrapping unaligned ADPCM block cache invalidation when writing RAM[0..15]
    let mut wrap_dev = Device::new_ps1();
    let wrap_addr = (libspu2::PS1_RAM_SIZE as u32) - 8;
    wrap_dev
        .write_ram(wrap_addr, &[0x00, 0x03, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11])
        .unwrap();
    wrap_dev.write_ram(0, &[0x11; 8]).unwrap();
    wrap_dev
        .write_ps1_register(
            ps1::SPUCNT,
            registers::CONTROL_ENABLE | registers::CONTROL_UNMUTE,
        )
        .unwrap();
    wrap_dev.write_ps1_register(ps1::V_VOLL, 0x3FFF).unwrap();
    wrap_dev.write_ps1_register(ps1::V_VOLR, 0x3FFF).unwrap();
    wrap_dev.write_ps1_register(ps1::V_PITCH, 0x1000).unwrap();
    wrap_dev.write_ps1_register(ps1::V_ADSR1, 0x000F).unwrap();
    wrap_dev
        .write_ps1_register(ps1::V_START_ADDR, (wrap_addr >> 3) as u16)
        .unwrap();
    wrap_dev
        .write_ps1_register(ps1::V_REPEAT_ADDR, (wrap_addr >> 3) as u16)
        .unwrap();
    wrap_dev.write_ps1_register(ps1::KON_LO, 1).unwrap();
    let mut before_wrap = [0i16; 64 * 2];
    wrap_dev.render_native(&mut before_wrap);
    // Now overwrite bytes 0..8 (the wrapped tail of the ADPCM block) and re-key-on.
    wrap_dev.write_ram(0, &[0x77; 8]).unwrap();
    wrap_dev.write_ps1_register(ps1::KON_LO, 1).unwrap();
    let mut after_wrap = [0i16; 64 * 2];
    wrap_dev.render_native(&mut after_wrap);
    assert_ne!(before_wrap, after_wrap);
}
