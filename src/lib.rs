// SPDX-License-Identifier: BSD-2-Clause

#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

mod adpcm;
mod device;
mod envelope;
mod inspect;
mod noise;
mod ps1_regs;
pub mod registers;
mod reverb;
mod spu2;
mod spu2_core;
mod spu_base;
mod synth;

pub use device::{
    Device, DeviceMode, Error, StereoI16, CORE_COUNT, NATIVE_SAMPLE_RATE, PS1_RAM_SIZE,
    PS1_SAMPLE_RATE, RAM_SIZE, REGISTER_SPACE_SIZE, VOICES_PER_CORE,
};
pub use inspect::{EnvelopePhase, VoiceState};
pub use synth::{AdsrConfig, Synth, VoiceParameters};
