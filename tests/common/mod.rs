// SPDX-License-Identifier: BSD-2-Clause

#![allow(dead_code)]

use std::fs;
use std::path::Path;

use libspu2::{registers, Device};

/// 64-bit FNV-1a hash over little-endian PCM bytes (matches C++ `fnv1a64`).
pub fn fnv1a64_pcm(samples: &[i16]) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &s in samples {
        for b in s.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01B3);
        }
    }
    h
}

pub fn make_sine(freq_hz: f64, count: usize) -> Vec<i16> {
    make_sine_ex(freq_hz, count, 0.8, 48_000)
}

pub fn make_sine_ex(freq_hz: f64, count: usize, amplitude: f64, sample_rate: u32) -> Vec<i16> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let phase =
            2.0 * std::f64::consts::PI * freq_hz * (i as f64) / (sample_rate as f64);
        out.push((amplitude * 32767.0 * phase.sin()) as i16);
    }
    out
}

/// PlayStation ADPCM encoder mirroring `tools/adpcm/adpcm_encode.cpp`.
pub fn encode_adpcm(pcm: &[i16], loop_sample: bool, loop_start_sample: u32) -> Vec<u8> {
    const K0: [i32; 5] = [0, 60, 115, 98, 122];
    const K1: [i32; 5] = [0, 0, -52, -55, -60];

    struct BlockTry {
        nibbles: [u8; 28],
        s1: i32,
        s2: i32,
        error: f64,
    }

    fn try_encode(x: &[i16], filter: usize, shift: u32, s1: i32, s2: i32) -> BlockTry {
        let mut t = BlockTry {
            nibbles: [0; 28],
            s1,
            s2,
            error: 0.0,
        };
        for (nib_slot, &sample) in t.nibbles.iter_mut().zip(x.iter()) {
            let target = (sample as i32) * 64;
            let predicted = (t.s1 * K0[filter]) / 64 + (t.s2 * K1[filter]) / 64;
            let residual = target - predicted;
            let unit = 64i32 << (12 - shift);
            let round = if residual >= 0 { unit / 2 } else { -(unit / 2) };
            let nib = ((residual + round) / unit).clamp(-8, 7);
            *nib_slot = (nib & 0xF) as u8;
            let work = ((((nib & 0xF) as u16) << 12) as i16 as i32) >> shift;
            let cur = work * 64 + predicted;
            t.s2 = t.s1;
            t.s1 = cur;
            let decoded = ((cur + 32) / 64).clamp(-32768, 32767);
            let d = (decoded as f64) - (sample as f64);
            t.error += d * d;
        }
        t
    }

    let mut padded = pcm.to_vec();
    while padded.len() % 28 != 0 {
        padded.push(0);
    }
    let blocks = padded.len() / 28;
    let mut out = vec![0u8; blocks * 16];
    let mut s1 = 0i32;
    let mut s2 = 0i32;

    for b in 0..blocks {
        let mut best = BlockTry {
            nibbles: [0; 28],
            s1,
            s2,
            error: 1e300,
        };
        let mut best_filter = 0usize;
        let mut best_shift = 0u32;
        for filter in 0..5 {
            for shift in 0..=12 {
                let t = try_encode(&padded[b * 28..(b + 1) * 28], filter, shift, s1, s2);
                if t.error < best.error {
                    best = t;
                    best_filter = filter;
                    best_shift = shift;
                }
            }
        }
        s1 = best.s1;
        s2 = best.s2;

        let mut flags = 0u8;
        if loop_sample && (loop_start_sample as usize / 28) == b {
            flags |= 0x04;
        }
        if b == blocks - 1 {
            flags |= if loop_sample { 0x03 } else { 0x01 };
        }

        let dst = &mut out[b * 16..(b + 1) * 16];
        dst[0] = ((best_filter as u8) << 4) | (best_shift as u8);
        dst[1] = flags;
        for i in 0..14 {
            dst[2 + i] = best.nibbles[i * 2] | (best.nibbles[i * 2 + 1] << 4);
        }
    }
    out
}

pub struct FixtureResult {
    pub pcm_hash: u64,
    pub frames_rendered: usize,
}

/// Executes a `.fixture` file against `device` (mirrors `tests/support/fixture.cpp`).
pub fn run_fixture(path: &Path, device: &mut Device) -> FixtureResult {
    let text = fs::read_to_string(path).expect("failed to read fixture file");
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut hash: u64 = 0xCBF2_9CE4_8422_2325;
    let mut frames = 0usize;
    let mut buffer: Vec<i16> = Vec::new();

    for raw_line in text.lines() {
        let line = match raw_line.find('#') {
            Some(pos) => &raw_line[..pos],
            None => raw_line,
        };
        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else {
            continue;
        };
        match cmd {
            "reset" => device.reset(),
            "ram_file" => {
                let addr = u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
                let rel = parts.next().unwrap();
                let bytes = fs::read(dir.join(rel)).expect("failed to read ram_file");
                device.write_ram(addr, &bytes).expect("ram_file out of range");
            }
            "ram_hex" => {
                let addr = u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
                let hex = parts.next().unwrap();
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                let hex_bytes = hex.as_bytes();
                let mut i = 0;
                while i + 1 < hex_bytes.len() {
                    let s = std::str::from_utf8(&hex_bytes[i..i + 2]).unwrap();
                    bytes.push(u8::from_str_radix(s, 16).unwrap());
                    i += 2;
                }
                device.write_ram(addr, &bytes).expect("ram_hex out of range");
            }
            "reg" => {
                let core_str = parts.next().unwrap();
                let off = u32::from_str_radix(parts.next().unwrap(), 16).unwrap();
                let val = u16::from_str_radix(parts.next().unwrap(), 16).unwrap();
                if core_str == "g" {
                    device.write_register(off, val).unwrap();
                } else {
                    let core: usize = core_str.parse().unwrap();
                    device
                        .write_register(registers::core_reg(core, off), val)
                        .unwrap();
                }
            }
            "render" => {
                let n: usize = parts.next().unwrap().parse().unwrap();
                buffer.resize(n * 2, 0);
                device.render_native(&mut buffer);
                for &s in &buffer {
                    for b in s.to_le_bytes() {
                        hash ^= b as u64;
                        hash = hash.wrapping_mul(0x0100_0000_01B3);
                    }
                }
                frames += n;
            }
            other => panic!("unknown fixture command: {other}"),
        }
    }

    FixtureResult {
        pcm_hash: hash,
        frames_rendered: frames,
    }
}

/// Port of Play! `tools/SpuTest/Test.{h,cpp}` fixture onto [`Device`].
pub struct SpuTestFixture {
    pub device: Device,
}

impl SpuTestFixture {
    pub const CORE_COUNT: u32 = 2;
    pub const VOICE_COUNT: u32 = 24;

    pub fn new() -> Self {
        Self {
            device: Device::new(),
        }
    }

    pub fn run_spu(&mut self, ticks: usize) {
        let mut samples = vec![0i16; ticks * 2];
        self.device.render_native(&mut samples);
    }

    pub fn get_core_register(&mut self, core: u32, offset: u32) -> u16 {
        self.device
            .read_register(registers::core_reg(core as usize, offset))
            .unwrap()
    }

    pub fn set_core_register(&mut self, core: u32, offset: u32, value: u16) {
        self.device
            .write_register(registers::core_reg(core as usize, offset), value)
            .unwrap();
    }

    pub fn get_core_address(&mut self, core: u32, offset: u32) -> u32 {
        let hi = self.get_core_register(core, offset) as u32;
        let lo = self.get_core_register(core, offset + 2) as u32;
        (hi << 17) | (lo << 1)
    }

    pub fn set_core_address(&mut self, core: u32, offset: u32, target: u32) {
        self.set_core_register(core, offset, ((target >> 17) & 0xFFFF) as u16);
        self.set_core_register(core, offset + 2, ((target >> 1) & 0xFFFF) as u16);
    }

    pub fn get_voice_register(&mut self, core: u32, voice: u32, offset: u32) -> u16 {
        self.get_core_register(core, offset + voice * registers::VOICE_STRIDE)
    }

    pub fn set_voice_register(&mut self, core: u32, voice: u32, offset: u32, value: u16) {
        self.set_core_register(core, offset + voice * registers::VOICE_STRIDE, value);
    }

    pub fn get_voice_address(&mut self, core: u32, voice: u32, offset: u32) -> u32 {
        self.get_core_address(core, offset + voice * registers::VOICE_ADDR_STRIDE)
    }

    pub fn set_voice_address(&mut self, core: u32, voice: u32, offset: u32, target: u32) {
        self.set_core_address(core, offset + voice * registers::VOICE_ADDR_STRIDE, target);
    }

    pub fn poke_ram(&mut self, addr: u32, byte0: u8, byte1: u8) {
        self.device.write_ram(addr, &[byte0, byte1]).unwrap();
    }

    pub fn read_irq_info(&mut self) -> u16 {
        self.device.read_register(registers::C_IRQINFO).unwrap()
    }
}
