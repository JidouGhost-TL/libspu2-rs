# libspu2-rs

`libspu2` is a standalone, dependency-free, `#![forbid(unsafe_code)]` Rust
library that emulates the PlayStation 2's SPU2 audio-synthesis hardware (plus a
PS1 SPU compatibility mode): two cores, 24 ADPCM voices each, 2 MiB shared SPU
RAM, pitch resampling, ADSR envelopes, channel volume sweeps, looping,
key-on/key-off, RAM/DMA transfers, IRQ watching, and reverb. It renders
deterministic 48 kHz stereo PCM (or 44.1 kHz in PS1 SPU mode, with optional
output-rate resampling) as interleaved `i16`, `StereoI16`, or planar `f32`,
with zero heap allocations in the render loop.

The synthesis core is a pure-Rust port of the C++ `libspu2` library, which is
derived from the [Play!](https://github.com/jpd002/Play-) emulator's
BSD-2-Clause SPU2 implementation (pinned commit
`50aedca2639521bc498ace0b2be1ea012801a86a`).

## Usage

### Low-level register API

```rust
use libspu2::{Device, registers};

let mut device = Device::new();
let mut block = [0x77u8; 16];
block[0] = 0x00; // shift 0, predictor 0
block[1] = 0x03; // loop, no mute
device.write_ram(0x5000, &block).unwrap();

device.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE).unwrap();
device.write_register(registers::VP_VOLL, 0x3FFF).unwrap();
device.write_register(registers::VP_VOLR, 0x3FFF).unwrap();
device.write_register(registers::VP_PITCH, 0x1000).unwrap();
device.write_register(registers::VP_ADSR1, 0x000F).unwrap();
device.write_register(registers::VA_SSA_HI, 0x0000).unwrap();
device.write_register(registers::VA_SSA_LO, 0x2800).unwrap();
device.write_register(registers::A_KON_HI, 0x0001).unwrap();

let mut pcm = vec![0i16; 4800 * 2];
device.render_native(&mut pcm);
assert!(pcm.iter().any(|&s| s != 0));
```

### Convenience `Synth` API

```rust
use libspu2::{Device, StereoI16, Synth, VoiceParameters};

let mut device = Device::new();
let mut block = [0x77u8; 16];
block[0] = 0x00; // shift 0, predictor 0
block[1] = 0x03; // loop, no mute
device.write_ram(0x5000, &block).unwrap();

let mut synth = Synth::new(&mut device);
synth.enable_core(0, true).unwrap();

let mut vp = VoiceParameters::default();
vp.start_address = 0x5000;
vp.loop_address = Some(0x5000);
vp.pitch = 0x1000;
synth.configure_voice(&vp).unwrap();
synth.key_on_voice(0, 0).unwrap();

let mut frames = vec![StereoI16::default(); 512];
synth.render_stereo(&mut frames);
assert!(frames.iter().any(|f| f.left != 0 || f.right != 0));
```

### PS1 SPU compatibility mode (and planar `f32` / WebAudio output)

```rust
use libspu2::{Device, registers::ps1};

let mut spu = Device::new_ps1();
assert_eq!(spu.ram_size(), libspu2::PS1_RAM_SIZE);
assert_eq!(spu.sample_rate(), libspu2::PS1_SAMPLE_RATE);

spu.write_ps1_register(ps1::SPUCNT, 0x8000).unwrap();
spu.set_output_sample_rate(48_000).unwrap();

let mut left = [0.0f32; 128];
let mut right = [0.0f32; 128];
spu.render_f32_planar(&mut left, &mut right).unwrap();
```

## Upstream golden parity & scope

**Intentional upstream parity:** `libspu2` intentionally preserves **bit-exact
output parity** with the upstream C++ `libspu2` / Play! golden PCM fixtures.
Synthesis quirks inherited from Play!'s `Iop_SpuBase` are deliberately kept
intact rather than altered, so outputs remain verifiable against the upstream
reference suite.

- **Intended use:** Standalone SPU2/PS1 synthesis, PSF/PSF2/sequence playback,
  WebAssembly `AudioWorklet` players, and audio tooling. While
  `Device::receive_dma` and non-destructive `Device::irq_pending` are exposed,
  full commercial PS2 `libsd` hardware emulation (e.g. `STATX` mode echo,
  cycle-accurate IRQ9, per-bus `P_MMIX` dry/wet muting) is outside the Play!
  Tier-1 scope.
- **Tier-1 Play! parity:** 2×24 ADPCM voices, linear pitch resampling, full ADSR
  envelopes, channel volume sweeps, ADPCM looping, manual & DMA RAM transfers,
  AutoDMA block input, IRQ watching, inter-core mixing, and hardware reverb.
- **Known omissions (matching C++ `libspu2` / Play!):** 4-tap Gaussian
  interpolation (linear is used), hardware noise generator, voice pitch
  modulation (`PMON`), per-voice dry/wet `P_MMIX`/`S_VMIXL`/`S_VMIXR` gating,
  and master/effect bus volume scaling (`P_MVOLL`/`P_EVOLL` and PS1
  `MVOLL`/`EVOLL` are accepted without altering output gain; reverb wet return
  uses Play!'s fixed `0.333` blend).
- **PS1 SPU mode (`Device::new_ps1`):** Runs Core 0 at 44.1 kHz over 512 KiB
  RAM with PS1 register offsets (`0x1F801C00 + offset`) and 8-byte address
  scaling; Core 1 register, DMA, voice, and rendering operations return
  `Error::OutOfRange`.
- **Real-time & portability:** Zero dependencies, `#![forbid(unsafe_code)]`,
  `Send + Sync`, MSRV 1.70, and a flat fixed-size ADPCM cache with no heap
  allocations during rendering or RAM invalidation.

## License

BSD 2-Clause (`LICENSE`). See `NOTICE` and `THIRD_PARTY.md` for attribution.
