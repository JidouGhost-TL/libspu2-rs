# libspu2-rs

`libspu2` is a standalone, dependency-free, `#![forbid(unsafe_code)]` Rust
library that emulates the PlayStation 2's SPU2 audio-synthesis hardware (plus a
PS1 SPU compatibility mode): two cores, 24 ADPCM voices each, 2 MiB shared SPU
RAM, pitch resampling, ADSR envelopes, channel volume sweeps, looping,
key-on/key-off, RAM/DMA transfers, IRQ watching, and reverb. It renders
deterministic 48 kHz stereo PCM (or 44.1 kHz in PS1 SPU mode, with optional
output-rate resampling) as interleaved `i16`, `StereoI16`, or planar `f32`,
with zero heap allocations in the render loop.

Originally ported from the C++ `libspu2` library derived from the
[Play!](https://github.com/jpd002/Play-) emulator's BSD-2-Clause SPU2
implementation (commit `50aedca2639521bc498ace0b2be1ea012801a86a`), the
synthesis core has since been substantially upgraded from hardware
specifications (**Martin Korth's Nocash PSX-SPX SPU documentation**) to match
physical PS1 (`CXD2922Q`/`CXD2925Q`) and PS2 (`CXD2938Q`/`CXD2947R`) silicon
rather than Play!'s original approximations.

## Usage

### Low-level register API

```rust
use libspu2::{Device, registers};

let mut device = Device::new();
let mut block = [0x77u8; 16];
block[0] = 0x00; // shift 0, predictor 0
block[1] = 0x03; // loop, no mute
device.write_ram(0x5000, &block).unwrap();

device.write_register(registers::CORE_ATTR, registers::CONTROL_ENABLE | registers::CONTROL_UNMUTE).unwrap();
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

## Hardware accuracy & scope

`libspu2` implements PlayStation 1 SPU and PlayStation 2 SPU2 hardware synthesis
with zero runtime dependencies and deterministic PCM output.

- **Intended use:** Standalone SPU2/PS1 synthesis, PSF/PSF2/sequence playback,
  WebAssembly `AudioWorklet` players, and audio tooling. While
  `Device::receive_dma` and non-destructive `Device::irq_pending` are exposed,
  full commercial PS2 `libsd` hardware emulation (e.g. `STATX` mode echo,
  cycle-accurate IRQ9, per-bus `P_MMIX` dry/wet muting) is outside scope.
- **Implemented hardware features:** 2×24 ADPCM voices (clamped `i16` history,
  `shift > 12 -> 9` clamp, 1-block `flags == 0x07` loops, per-half-block
  `ENDX`), 512-entry 4-tap PS1/PS2 hardware Gaussian ROM interpolation,
  signed/phase-inverted fixed voice volumes, discrete cycle-counter ADSR
  envelopes (including the `>= 0x6000` exponential-increase fake knee), channel
  & master volume sweeps, voice pitch modulation (`PMON`), 2-stage hardware LFSR
  noise generator and noise clock (`NON` + `CORE_ATTR`/`SPUCNT` bits 8–13),
  `CONTROL_UNMUTE` (bit 14), master output volume (`P_MVOLL`/`P_MVOLR` and PS1
  `MVOLL`/`MVOLR`), reverb wet return volume (`P_EVOLL`/`P_EVOLR` and PS1
  `EVOLL`/`EVOLR`), 16-bit fixed-point alternating-cycle hardware reverb with
  39-tap half-band downsample/upsample FIR filters, PS1 capture-buffer RAM
  writeback (`0x0000..0x0FFF` + `SPUSTAT` bit 11), manual & DMA RAM transfers,
  AutoDMA block input, IRQ watching, and inter-core mixing.
- **PS1 SPU mode (`Device::new_ps1`):** Runs Core 0 at 44.1 kHz over 512 KiB
  RAM with PS1 register offsets (`0x1F801C00 + offset`), 8-byte address
  scaling, read-only `ENDX` (cleared per-voice on `KON`), and circular
  capture-buffer RAM (`0x0000..0x0FFF`); Core 1 register, DMA, voice, and
  rendering operations return `Error::OutOfRange`.
- **Real-time & portability:** Zero dependencies, `#![forbid(unsafe_code)]`,
  `Send + Sync`, MSRV 1.70, and a flat fixed-size ADPCM cache with no heap
  allocations during rendering or RAM invalidation.

## Differences from upstream Play! (`50aedca2`)

While `libspu2` retains its BSD-2-Clause structural heritage and register/DMA
framework from Play!, `v0.2.0` intentionally diverges from Play!'s simplified
synthesis math to match documented PS1/PS2 silicon behavior (sourced clean-room
from Martin Korth's [Nocash PSX-SPX SPU specification](https://psx-spx.consoledev.net/soundprocessingunitspu/)):

| Subsystem | Upstream Play! (`50aedca2`) | `libspu2` (`v0.2.0`+) |
| :--- | :--- | :--- |
| **Interpolation** | 2-point linear interpolation | 512-entry PS1/PS2 hardware ROM 4-tap Gaussian interpolation |
| **ADPCM decode** | Unclamped 64× `s1`/`s2` with `/ 64`; no `shift > 12` clamp; eager double-buffer prefetch cuts one-shot voices early; `flags == 0x07` stops after 1 block | Clamped `i16` `s1`/`s2` with `((s1*k0)>>6) + ((s2*k1)>>6)`; `shift > 12 -> 9`; per-half-block `ENDX`/`done`; 1-block `0x07` loops continuously |
| **ADSR envelopes** | Continuous 31-bit `ADSR_LOG_TABLE` 8-bucket approximation + `KON` preload | Exact discrete 15-bit cycle-counter state machine (`>= 0x6000` exp-attack knee with rate split, Decay-before-Sustain check, apply-then-tick order) |
| **Voice & bus volumes** | `fixed_phase` bit 14 positive (`0x3FFF - vol`); `/ 0x7FFF` scaling; `MVOLL`/`EVOLL` & `CONTROL_UNMUTE` ignored | Signed two's-complement phase-inverting volumes; arithmetic `>> 15` scaling; active `MVOLL`/`MVOLR` (fixed + sweep), `EVOLL`/`EVOLR`, and `CONTROL_UNMUTE` |
| **Pitch mod & Noise** | `PMON` and `NON` no-ops | Hardware `PMON` (`VxOUTX(i-1)` modulation, `0x3FFF` clamp) and 2-stage `NoiseDivider`/`NoiseCounter` 16-bit XNOR LFSR (`NON`) |
| **Reverb DSP** | Float approximation, even-sample drop / zero-order hold, hardcoded `0.333` wet mix, simplified APF2 | 16-bit saturating fixed-point alternating L/R cycles, 39-tap half-band downsample/upsample FIR, cascaded APF1->APF2, `EVOLL`/`EVOLR` wet scaling, fixed PS2 `A_EEA_HI` boundary |
| **PS1 Capture RAM** | Not implemented | Circular writeback to `0x0000..0x0FFF` (CD-L, CD-R, Voice 1, Voice 3) + `SPUSTAT` bit 11 |

## Verification & Testing

- **Shipped test suite (`cargo test`):** 76 unit, integration, register/DMA,
  fuzz-regression, and deterministic golden PCM tests plus 3 doc-tests,
  including hardware-specification tests (`tests/unit_hardware_accuracy.rs`)
  covering every subsystem above.
- **Independent differential verification (local, unshipped):** The synthesis
  core was differential-tested outside this repository against hardware-validated
  PS1 SPU reference behavior (Mednafen/Beetle-PSX and DuckStation) across
  identical register and SPU RAM scripts:
  - **100.00% (`0 LSB`)** bit-exact steady-state parity on ADPCM decoding
    (all 5 filters, 1-block `0x07` & 2-block loops), discrete ADSR (`ENVX` and
    PCM across linear/exponential attack with `0x6000` knee, decay, sustain,
    and release), `NON` LFSR noise (slow, mid, and high-rate `shift = 14`),
    signed phase-inverted volumes, `CONTROL_UNMUTE`, and PS1 capture-buffer RAM
    (`0x0800..0x0FFF`).
  - **Documented intentional timing delta:** `libspu2` begins decoding
    immediately on frame 0 after `KON` rather than modeling the hardware's
    4-sample `KON` pipeline delay, preserving compatibility with the existing
    register/timing test suite.

## License

BSD 2-Clause (`LICENSE`). See `NOTICE`, `THIRD_PARTY.md`, and `CHANGELOG.md`
for attribution and release history.
