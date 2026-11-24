# Changelog

All notable changes to `libspu2` are documented in this file.

## Unreleased (targeting `0.2.0`)

### Hardware Accuracy Upgrades (Clean-Room from Nocash PSX-SPX)
- **4-tap Gaussian interpolation:** Replaced Play!'s 2-point linear
  interpolation with the 512-entry PS1/PS2 hardware ROM Gaussian table and
  3-sample cross-block history.
- **ADPCM decoding:** Switched `s1`/`s2` history to clamped `i16` with separate
  truncation `((s1 * k0) >> 6) + ((s2 * k1) >> 6)`, added `shift > 12 -> 9`
  hardware clamping, fixed 1-block loop flags (`flags == 0x07`), added
  per-half-block `ENDX`/`done` tracking (fixing premature one-shot cutoffs),
  and made PS1 `ENDX` read-only (cleared per-voice on `KON`).
- **ADSR envelopes:** Replaced Play!'s 8-bucket `ADSR_LOG_TABLE` approximation
  with the exact discrete 15-bit cycle-counter state machine (`>= 0x6000`
  exponential-increase fake knee with rate split, Decay-before-Sustain check,
  apply-then-tick order, no spurious `KON` preload).
- **Volumes & mute:** Fixed two's-complement phase-inverting voice volumes
  (`fixed_phase` bit 14), replaced `/ 0x7FFF` with arithmetic `>> 15` scaling,
  wired `MVOLL`/`MVOLR` (fixed + sweep), `EVOLL`/`EVOLR`, and `CONTROL_UNMUTE`
  (bit 14), and fixed PS2 `P_AVOL`/`P_BVOL` sign-extension.
- **Pitch modulation & Noise:** Implemented hardware `PMON` (`VxOUTX(i-1)`
  modulation + `0x3FFF` pitch clamp) and 2-stage `NoiseDivider`/`NoiseCounter`
  16-bit XNOR LFSR `NON` generator.
- **Fixed-point 39-tap FIR reverb:** Replaced Play!'s float reverb with
  alternating-cycle 16-bit saturating fixed-point IIR/Comb/APF1->APF2 reverb,
  39-tap half-band downsample/upsample FIR filters, `EVOLL`/`EVOLR` wet return
  scaling, and fixed the PS2 `A_EEA_HI` end-address boundary.
- **PS1 capture buffers:** Added circular RAM writeback (`0x0000..0x0FFF` for
  CD-L, CD-R, Voice 1, Voice 3) and `SPUSTAT` bit 11.

### Additive `Synth` Register Helpers
- Added `Synth::set_adsr`, `set_volume_signed`, `set_pmon`, `set_noise`,
  `set_noise_clock`, `set_master_volume`, `set_reverb_volume`, and
  `set_reverb_work_area`.

### Verification
- Verified locally via unshipped differential testing against
  hardware-validated PS1 SPU references (Mednafen/Beetle-PSX & DuckStation) to
  100.00% (`0 LSB`) steady-state parity on ADPCM, ADSR, `NON`, signed volumes,
  mute, and PS1 capture RAM.

## `0.1.0`

- Initial standalone pure-Rust port of Play!'s BSD-2-Clause SPU2 core with PS1
  compatibility mode (`Device::new_ps1`), `Synth` helper, zero dependencies,
  and `#![forbid(unsafe_code)]`.
