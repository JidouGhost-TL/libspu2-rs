// SPDX-License-Identifier: BSD-2-Clause

//! Voice state inspection snapshots.

/// Envelope phase of a voice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum EnvelopePhase {
    /// Voice is idle / silent.
    #[default]
    Stopped = 0,
    /// Key-on requested; awaiting first render step.
    KeyOn = 1,
    /// Attack ramp toward maximum level.
    Attack = 2,
    /// Decay ramp toward sustain level.
    Decay = 3,
    /// Sustain phase (holding, increasing, or decreasing per ADSR2).
    Sustain = 4,
    /// Release ramp toward zero after key-off.
    Release = 5,
}

/// Snapshot of a single voice's internal state as of the last rendered frame
/// (plus immediate write side effects such as the key-on `ENVX` pre-load).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct VoiceState {
    /// Raw 16-bit pitch register value (`0x1000` = unity).
    pub pitch: u16,
    /// Envelope level (`0..=0x7FFF`; top 16 bits of internal 32-bit ADSR volume).
    pub envx: u16,
    /// Current ADSR envelope phase.
    pub envelope_phase: EnvelopePhase,
    /// Whether this voice's `ENDX` flag is currently set.
    pub reached_end: bool,
    /// Voice start address in SPU RAM (bytes).
    pub start_address: u32,
    /// Current ADPCM read address in SPU RAM (bytes).
    pub current_address: u32,
    /// Voice loop/repeat address in SPU RAM (bytes).
    pub repeat_address: u32,
    /// Current internal 32-bit left channel volume level (`fixed_volume << 17`, max `0x7FFE_0000`).
    pub volume_left: i32,
    /// Current internal 32-bit right channel volume level (`fixed_volume << 17`, max `0x7FFE_0000`).
    pub volume_right: i32,
}
