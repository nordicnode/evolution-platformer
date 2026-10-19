//! Sound, synthesised: every effect is generated sample by sample in this
//! module, so the game ships no audio assets and the sounds match the pixel
//! art's register (square waves, short envelopes, nothing sampled).
//!
//! Three parts. [`Ear`] watches a [`Sim`] and turns what changed between two
//! ticks into effects: the episode stats already count jumps, stomps, springs
//! and wall jumps, so an ear is a diff, not a second physics. [`play`] sends
//! an effect to the speakers through one process-wide player; before the app
//! shell calls [`init`] (or on a machine with no audio device, where init
//! fails quietly) every call is a no-op, which is what lets the headless
//! tests and the screenshot runs construct sessions without a sound card.
//! And the music: one original theme, written as score data and rendered by
//! the same synth, long enough (a minute and a quarter) and arranged in
//! enough sections that it does not wear out on the second pass.
//!
//! The sim is never touched: an ear only reads, so determinism, replays and
//! state hashes are exactly what they were in silence.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use sim::{Level, Outcome, Sim, Tile};

/// Sample rate for everything generated here. Half of CD rate is plenty for
/// square waves and halves the synthesis cost of the music loop.
const RATE: u32 = 22_050;

/// How loud the music plays on its own sink.
///
/// The music is the bed the effects sit on, not the other way round: a jump
/// is a transient and a song is a floor, and a floor that goes quiet under
/// every blip is a song nobody hears. Kept above [`SFX_LEVEL`] on purpose.
const MUSIC_VOLUME: f32 = 0.62;

/// The trim on every one-shot effect. Effects are mixed straight onto the
/// output and can overlap each other and the music, so they are played a
/// little under it rather than at the level they render at.
const SFX_LEVEL: f32 = 0.50;

/// One sound the game can make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Jump,
    WallJump,
    Land,
    Stomp,
    Spring,
    /// Entering water: a splash and the surface closing over the body.
    Splash,
    /// One swim stroke: the soft push of an arm through the water.
    Swim,
    /// A lava vent throwing a ball: a low thump and a hiss of steam.
    LavaBurst,
    Checkpoint,
    Crumble,
    Shot,
    Death,
    Win,
    MenuMove,
    MenuConfirm,
}

/// Render an effect to samples. Pure function of the kind, which is what the
/// tests pin: a sound that changes shape should change here on purpose.
fn render(kind: Sfx) -> Vec<f32> {
    match kind {
        // The platformer jump: a square wave climbing a fifth.
        Sfx::Jump => sweep(280.0, 660.0, 0.14, 0.32, Wave::Square),
        Sfx::WallJump => sweep(420.0, 840.0, 0.09, 0.26, Wave::Square),
        // A landing is a thud, not a note: low sine plus a breath of noise.
        Sfx::Land => mix(&[
            (0, sweep(150.0, 60.0, 0.07, 0.36, Wave::Sine)),
            (0, noise(0.03, 0.12, 7)),
        ]),
        Sfx::Stomp => mix(&[
            (0, sweep(320.0, 70.0, 0.12, 0.4, Wave::Square)),
            (0, noise(0.06, 0.2, 13)),
        ]),
        Sfx::Spring => sweep(180.0, 1250.0, 0.16, 0.34, Wave::Square),
        // Water closes over a body rather than hitting it: a bright spray
        // that darkens as the surface swallows it, a low gloop of the air
        // pocket, and the droplets and bubbles it throws clear.
        Sfx::Splash => splash_voice(),
        // A stroke is the water moving past the arm, not a click of the arm
        // through it: a soft band of noise swelling and falling, with a
        // bubble trailing it. The Ear spaces these into a rhythm.
        Sfx::Swim => swim_voice(),
        // The vent: a low thump as the ball breaks the surface, then the
        // steam and grit of it. Direction is the picture's job; this is the
        // sound of something being thrown upward from below.
        Sfx::LavaBurst => mix(&[
            (0, sweep(120.0, 38.0, 0.24, 0.42, Wave::Sine)),
            (0, brighten(noise(0.30, 0.25, 83), 0.72)),
            (0, noise(0.06, 0.15, 89)),
        ]),
        Sfx::Checkpoint => tone(990.0, 0.07, 0.22, Wave::Sine),
        Sfx::Crumble => noise(0.09, 0.18, 29),
        // A turret shot is a tick, deliberately quiet: a busy level fires a
        // lot of them, and the ear throttles them on top of this (see Ear).
        Sfx::Shot => sweep(1500.0, 900.0, 0.045, 0.12, Wave::Square),
        Sfx::Death => mix(&[
            (0, sweep(620.0, 70.0, 0.34, 0.36, Wave::Saw)),
            (0, noise(0.2, 0.14, 41)),
        ]),
        // Four rising notes, the oldest win jingle there is.
        Sfx::Win => arpeggio(&[523.25, 659.25, 783.99, 1046.5], 0.09, 0.3),
        Sfx::MenuMove => tone(740.0, 0.03, 0.18, Wave::Square),
        Sfx::MenuConfirm => arpeggio(&[659.25, 987.77], 0.05, 0.24),
    }
}

#[derive(Clone, Copy)]
enum Wave {
    Square,
    Sine,
    Saw,
    /// The soft one: most of a sine's roundness with enough edge to stay
    /// audible under the bass. What the counterline is played on, because a
    /// square lead over a marching bass is a circus and the game is not one.
    Triangle,
    /// An eighth-duty pulse: bright and thin, used for the arpeggio sparkle.
    Pulse12,
}

impl Wave {
    fn at(self, phase: f32) -> f32 {
        let p = phase.fract();
        match self {
            Wave::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            Wave::Sine => (phase * std::f32::consts::TAU).sin(),
            Wave::Saw => 2.0 * p - 1.0,
            Wave::Triangle => 2.0 * (2.0 * p - 1.0).abs() - 1.0,
            Wave::Pulse12 => {
                if p < 0.125 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

/// A frequency glide with a fast attack and an exponential release: the
/// envelope is most of what makes a blip read as an event rather than a beep.
fn sweep(f0: f32, f1: f32, secs: f32, vol: f32, wave: Wave) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n.max(1) as f32;
        let freq = f0 + (f1 - f0) * t;
        phase += freq / RATE as f32;
        let attack = (i as f32 / (RATE as f32 * 0.005)).min(1.0);
        let release = (1.0 - t).powf(1.6);
        out.push(wave.at(phase) * vol * attack * release);
    }
    out
}

fn tone(freq: f32, secs: f32, vol: f32, wave: Wave) -> Vec<f32> {
    sweep(freq, freq, secs, vol, wave)
}

/// A short arpeggio of square notes, each with the sweep envelope.
fn arpeggio(notes: &[f32], note_secs: f32, vol: f32) -> Vec<f32> {
    let step = (note_secs * RATE as f32) as usize;
    let parts: Vec<(usize, Vec<f32>)> = notes
        .iter()
        .enumerate()
        .map(|(i, &f)| (i * step, tone(f, note_secs * 1.6, vol, Wave::Square)))
        .collect();
    mix(&parts)
}

/// Deterministic white noise (an LCG, so a test can rely on the shape and no
/// run sounds different from another).
fn noise(secs: f32, vol: f32, seed: u32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    (0..n)
        .map(|i| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = (state >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
            let t = i as f32 / n.max(1) as f32;
            v * vol * (1.0 - t)
        })
        .collect()
}

/// Overlay parts at sample offsets, clipped to the unit range.
fn mix(parts: &[(usize, Vec<f32>)]) -> Vec<f32> {
    let len = parts
        .iter()
        .map(|(off, p)| off + p.len())
        .max()
        .unwrap_or(0);
    let mut out = vec![0.0f32; len];
    for (off, part) in parts {
        for (i, s) in part.iter().enumerate() {
            out[off + i] = (out[off + i] + s).clamp(-1.0, 1.0);
        }
    }
    out
}

/// A sustained note: fast attack, a held body, and the release saved for the
/// end. The effects all decay (a blip should), but a line made of decays is a
/// line made of hiccups: the counterline has to *hold* its notes the way a
/// bowed one does.
fn note(freq: f32, secs: f32, vol: f32, wave: Wave) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    let release_at = 0.8;
    for i in 0..n {
        let t = i as f32 / n.max(1) as f32;
        phase += freq / RATE as f32;
        let attack = (i as f32 / (RATE as f32 * 0.008)).min(1.0);
        let body = if t < release_at {
            1.0
        } else {
            1.0 - (t - release_at) / (1.0 - release_at)
        };
        out.push(wave.at(phase) * vol * attack * body);
    }
    out
}

/// Equal-temperament frequency of a MIDI note number.
fn midi(m: i32) -> f32 {
    440.0 * 2f32.powf((m - 69) as f32 / 12.0)
}

// ---------------------------------------------------------------------------
// The music: one song, arranged in sections.
// ---------------------------------------------------------------------------
//
// The first version was a fourteen-second two-phrase loop, and fourteen
// seconds is a jingle, not a song: the ear maps it in two passes and then has
// nothing left to hear. This is forty-eight bars at 152 BPM -- about a minute
// and a quarter -- in the form the console classics use: a groove intro, an
// eight-bar A section, the same harmony answered differently, a lift into the
// relative major, a bridge that turns to the minor, the last chorus and a
// turnaround home.
//
// **There is deliberately no lead voice.** The high pulse melody was the part
// that wore thin, and a busy lead is the first thing an ear tires of. What
// carries the song instead is the arrangement, which is how a console track
// with no melody channel still holds a room:
//
// - the arpeggio spells the harmony, cycling chord tones two octaves up;
// - root-fifth-octave bass, the NES triangle's job, steps in eighths so the
//   harmony moves without block chords;
// - a counterline answers in the B section, the one place a voice sits above
//   the arpeggio, and gets out of the way everywhere else;
// - noise-channel drums: short filtered bursts for kick, snare and hats,
//   fills at the phrase ends, a crash where a section opens;
// - variation by texture rather than by key: half-time drums and a held
//   counterline in the B section, sixteenth-note hats under the bridge.
//
// Every section is written on the same sixteenth-note grid, so the song is a
// whole number of bars and the loop returns on a bar line.

/// Music tempo. Brisk enough for a platformer; slow enough that the
/// sixteenth grid leaves room for a groove instead of a march.
const MUSIC_BPM: f32 = 152.0;

/// Sixteenth notes in a bar: the grid every part is written on.
const STEPS_PER_BAR: usize = 16;

/// Bars in the song. Four of intro, then four eight-bar sections, a bridge,
/// the hook again and a four-bar outro.
const MUSIC_BARS: usize = 48;

/// Where each section starts, in bars.
const A_BAR: usize = 4;
const A2_BAR: usize = 12;
const B_BAR: usize = 20;
const BRIDGE_BAR: usize = 28;
const A3_BAR: usize = 36;
const OUTRO_BAR: usize = 44;

fn step_secs() -> f32 {
    60.0 / MUSIC_BPM / 4.0
}

fn grid_samples() -> usize {
    (step_secs() * RATE as f32) as usize
}

/// One bar of harmony. `root` is a MIDI note in the bass octave; the third
/// and the seventh are what the arpeggio and the chord-tone tests need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Chord {
    root: i32,
    minor: bool,
    seventh: bool,
}

const fn major(root: i32) -> Chord {
    Chord {
        root,
        minor: false,
        seventh: false,
    }
}

const fn minor(root: i32) -> Chord {
    Chord {
        root,
        minor: true,
        seventh: false,
    }
}

const fn dominant(root: i32) -> Chord {
    Chord {
        root,
        minor: false,
        seventh: true,
    }
}

impl Chord {
    fn third(self) -> i32 {
        if self.minor {
            3
        } else {
            4
        }
    }

    /// Chord tones as semitones above the root, in the octave the arpeggio
    /// uses: root, third, fifth, and the seventh in place of the octave when
    /// the chord is a dominant seventh.
    fn tones(self) -> [i32; 4] {
        if self.seventh {
            [0, self.third(), 7, 10]
        } else {
            [0, self.third(), 7, 12]
        }
    }
}

/// The harmony, one chord per bar: A minor with a harmonic-minor dominant at
/// the cadences, a lift to the relative major for the B section, and a bridge
/// that walks down through Dm and Bb before the dominant brings it home.
fn harmony() -> Vec<Chord> {
    let a = [
        minor(45),    // Am
        major(41),    // F
        major(48),    // C
        major(43),    // G
        minor(45),    // Am
        major(41),    // F
        major(43),    // G
        dominant(40), // E7
    ];
    let b = [
        major(48), // C
        major(43), // G
        minor(45), // Am
        major(41), // F
        major(48), // C
        major(43), // G
        major(41), // F
        major(43), // G
    ];
    let bridge = [
        minor(50),    // Dm
        minor(45),    // Am
        major(46),    // Bb
        major(41),    // F
        minor(50),    // Dm
        dominant(40), // E7
        minor(45),    // Am
        dominant(40), // E7
    ];
    let outro = [minor(45), major(41), major(43), dominant(40)];

    let mut bars = Vec::with_capacity(MUSIC_BARS);
    // Intro: two bars on the tonic, then the subdominant and the dominant,
    // so the hook has a progression under it when it arrives.
    bars.extend([minor(45), minor(45), major(41), dominant(40)]);
    bars.extend(a);
    bars.extend(a);
    bars.extend(b);
    bars.extend(bridge);
    bars.extend(a);
    bars.extend(outro);
    debug_assert_eq!(bars.len(), MUSIC_BARS);
    bars
}

/// One written note: its bar within a section, its sixteenth step in that
/// bar, its pitch, and how many steps it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Note {
    bar: usize,
    step: usize,
    midi: i32,
    len: usize,
}

const fn n(bar: usize, step: usize, midi: i32, len: usize) -> Note {
    Note {
        bar,
        step,
        midi,
        len,
    }
}

/// The counterline: held chord tones two octaves below the B section's lead,
/// answering it where it breathes. It plays nowhere else on purpose -- a
/// second voice everywhere is not counterpoint, it is doubling.
fn counter_notes() -> Vec<Note> {
    let b = B_BAR;
    vec![
        n(b, 0, 64, 8),
        n(b, 8, 67, 8),
        n(b + 1, 0, 62, 8),
        n(b + 1, 8, 59, 8),
        n(b + 2, 0, 60, 8),
        n(b + 2, 8, 57, 8),
        n(b + 3, 0, 57, 8),
        n(b + 3, 8, 60, 8),
        n(b + 4, 0, 64, 8),
        n(b + 4, 8, 67, 8),
        n(b + 5, 0, 62, 8),
        n(b + 5, 8, 59, 8),
        n(b + 6, 0, 60, 8),
        n(b + 6, 8, 65, 8),
        n(b + 7, 0, 59, 8),
        n(b + 7, 8, 62, 8),
    ]
}

/// A short plucked pulse for the arpeggio: fast attack, exponential decay.
/// Quiet by design -- it is the sparkle on top of the harmony, not a voice.
fn pluck_voice(freq: f32, secs: f32, vol: f32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n.max(1) as f32;
        phase += freq / RATE as f32;
        let attack = (i as f32 / (RATE as f32 * 0.004)).min(1.0);
        let decay = (-4.5 * t).exp();
        out.push(Wave::Pulse12.at(phase) * vol * attack * decay);
    }
    out
}

/// The bass: a triangle stepped to eight levels, the way the NES triangle's
/// four-bit ladder sounds, with a hard attack and a short release. A pure
/// triangle is too round to cut through the drums.
fn bass_voice(freq: f32, secs: f32, vol: f32) -> Vec<f32> {
    let n = (secs * RATE as f32) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    let release_at = (n as f32 * 0.85) as usize;
    for i in 0..n {
        phase += freq / RATE as f32;
        let triangle = 2.0 * (2.0 * phase.fract() - 1.0).abs() - 1.0;
        let stepped = (triangle * 7.0).round() / 7.0;
        let attack = (i as f32 / (RATE as f32 * 0.004)).min(1.0);
        let body = if i < release_at {
            1.0
        } else {
            (n - i) as f32 / (n - release_at).max(1) as f32
        };
        out.push(stepped * vol * attack * body);
    }
    out
}

/// The kick: a low sine falling fast, with a click of noise for the attack.
fn kick_voice() -> Vec<f32> {
    mix(&[
        (0, sweep(140.0, 46.0, 0.12, 0.7, Wave::Sine)),
        (0, noise(0.012, 0.15, 53)),
    ])
}

/// The snare: filtered noise over a short pitched thump.
fn snare_voice() -> Vec<f32> {
    mix(&[
        (0, brighten(noise(0.10, 0.55, 71), 0.9)),
        (0, tone(180.0, 0.05, 0.30, Wave::Triangle)),
    ])
}

/// A hat: a few milliseconds of bright noise. `open` lets it ring long
/// enough to mark the end of a bar.
fn hat_voice(open: bool) -> Vec<f32> {
    let secs = if open { 0.13 } else { 0.03 };
    let vol = if open { 0.22 } else { 0.30 };
    brighten(noise(secs, vol, if open { 117 } else { 97 }), 0.94)
}

/// A one-pole high-pass, for turning white noise into something that reads
/// as a hat or a snare rather than a hiss.
fn brighten(samples: Vec<f32>, a: f32) -> Vec<f32> {
    let mut prev_in = 0.0f32;
    let mut prev_out = 0.0f32;
    samples
        .into_iter()
        .map(|x| {
            let y = a * (prev_out + x - prev_in);
            prev_in = x;
            prev_out = y;
            y
        })
        .collect()
}

/// Add a rendered voice into a track at an absolute sample, wrapping past the
/// end of the loop back to the start. A note or an echo tail that crosses the
/// loop point keeps ringing when the song restarts, which is what makes the
/// seam inaudible instead of a click.
fn place(track: &mut [f32], at: usize, samples: Vec<f32>, gain: f32) {
    let n = track.len();
    for (i, s) in samples.iter().enumerate() {
        track[(at + i) % n] += s * gain;
    }
}

/// Master a rendered loop to a headroom peak: down when it is hot, up when
/// the arrangement is quiet. Everything that reaches this is synthesised from
/// known voices, so the peak is a real level rather than one stray sample, and
/// a song that lost its lead (and with it a third of its level) still comes
/// out at the volume it is mixed at.
fn normalize(samples: &mut [f32], peak: f32) {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 0.0 {
        let gain = peak / max;
        for s in samples.iter_mut() {
            *s *= gain;
        }
    }
}

// ---------------------------------------------------------------------------
// The elements' beds: the water and lava the body is near, not events in them.
// ---------------------------------------------------------------------------
//
// A pool is not an event. "Near water" is a *place*, and a place has a sound
// that runs while you are in it: the alternative -- a lap every second or two
// -- is a metronome, and after a minute beside a pool it is worse than silence.
// So each element gets one seamless loop, played on its own sink and faded up
// and down by proximity. The loops are filtered noise rather than notes,
// because they are the room the effects happen in.

/// Crossfade the tail of a loop over its head, so the wrap is a continuation
/// rather than a cut. `fade` samples are taken from the end and mixed into the
/// first `fade`, then the tail is dropped: the start now contains the end's
/// own decay, which is what makes the join inaudible.
fn loopify(mut samples: Vec<f32>, fade: usize) -> Vec<f32> {
    let n = samples.len().saturating_sub(fade);
    if n == 0 || fade == 0 {
        return samples;
    }
    for i in 0..fade {
        let t = i as f32 / fade as f32;
        samples[i] = samples[i] * t + samples[n + i] * (1.0 - t);
    }
    samples.truncate(n);
    samples
}

// -- the water voices -------------------------------------------------------
//
// Water is not a hiss. A hiss has one spectrum and no events in it, which is
// exactly what a pool does not sound like: water is **moving bands of noise**
// with **bubbles in them**. A low-passed noise bed reads as wind (or a distant
// road) because all of its energy sits still at the bottom of the spectrum and
// nothing in it ever changes shape. The voices below are built the other way
// round: a pair of band-passes whose cutoffs wander, so the surface is always
// sliding between bright and dull; a low rumble under them for the body of the
// pool; and bubbles -- short sine chirps that **rise** in pitch, because a
// bubble's resonance climbs as it shrinks -- scattered through the bed and the
// splash. The rise is the cue the ear reads as "water" rather than "noise".

/// A topology-preserving state-variable filter's band-pass output.
///
/// One filter is enough for a voice that has to stay stable while its cutoff
/// sweeps: the TPT form is unconditionally stable, so a bubble or a splash can
/// move the cutoff from 3 kHz to 400 Hz inside a tenth of a second without the
/// filter blowing up, which is what a naive one-pole cascade does when pushed.
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn band(&mut self, x: f32, freq: f32, q: f32) -> f32 {
        let g = (std::f32::consts::PI * freq.clamp(20.0, RATE as f32 * 0.45) / RATE as f32).tan();
        let k = 1.0 / q.max(0.05);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        v1
    }
}

/// White noise through a band-pass whose cutoff and level are functions of
/// time in seconds. The envelope closure is applied per sample, so a voice can
/// attack, swell and die inside one call.
fn moving_noise<F, A>(secs: f32, seed: u32, q: f32, mut freq: F, mut amp: A) -> Vec<f32>
where
    F: FnMut(f32) -> f32,
    A: FnMut(f32) -> f32,
{
    let n = (secs * RATE as f32).max(1.0) as usize;
    let mut state = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let mut filter = Svf::default();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let white = (state >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
        let t = i as f32 / RATE as f32;
        let band = filter.band(white, freq(t), q);
        out.push(band * amp(t));
    }
    out
}

/// One bubble: a sine chirp rising from `f0` to `f1`, with a fast attack and
/// an exponential-ish tail. The rise is the whole cue -- a bubble that falls
/// sounds like a drip hitting a drum, and one that holds still sounds like a
/// beep.
fn bubble(f0: f32, f1: f32, secs: f32, vol: f32) -> Vec<f32> {
    let n = (secs * RATE as f32).max(1.0) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let freq = f0 * (f1 / f0).powf(t);
        phase = (phase + freq / RATE as f32).fract();
        let attack = (i as f32 / (RATE as f32 * 0.0015)).min(1.0);
        let env = attack * (1.0 - t).powf(2.0);
        // A touch of second harmonic gives the bubble a little body; a pure
        // sine at this length is a test tone, not a pocket of air.
        let tone = (std::f32::consts::TAU * phase).sin()
            + 0.18 * (std::f32::consts::TAU * 2.0 * phase).sin();
        out.push(tone * vol * env);
    }
    out
}

/// A deterministic hash, for scattering bubbles at irregular intervals without
/// an RNG that would make two runs of the same level sound different.
fn scatter(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

/// The body entering water: spray, the pool closing over it, and the bubbles
/// that come back up.
fn splash_voice() -> Vec<f32> {
    let at = |secs: f32| (secs * RATE as f32) as usize;
    // The hit and the water it moves, in one go. The impact is bright spray
    // that attacks in two milliseconds -- a splash begins at its loudest --
    // and darkens as the surface swallows it; a raw, unfiltered crack sits on
    // top for the first few milliseconds, because spray is broadband and a
    // band-pass alone would round off the edge that says "impact". Under it
    // the pool takes the weight (a duller band a moment later) and the air
    // pocket glides down.
    let mut parts: Vec<(usize, Vec<f32>)> = vec![
        (
            0,
            moving_noise(
                0.30,
                61,
                0.7,
                |t| 3000.0 - 2200.0 * t,
                |t| 0.70 * (1.0 - t).powf(1.7) * (t / 0.003).min(1.0),
            ),
        ),
        (0, brighten(noise(0.055, 0.30, 61), 0.45)),
        (
            at(0.025),
            moving_noise(
                0.55,
                67,
                0.55,
                |t| 950.0 - 550.0 * t,
                |t| 0.30 * (1.0 - t).powf(2.0) * (t / 0.02).min(1.0),
            ),
        ),
        (0, sweep(340.0, 85.0, 0.26, 0.30, Wave::Sine)),
        // A second, smaller slap as the surface rebounds.
        (
            at(0.16),
            moving_noise(
                0.18,
                71,
                0.7,
                |t| 1800.0 - 1200.0 * t,
                |t| 0.22 * (1.0 - t).powf(2.0) * (t / 0.004).min(1.0),
            ),
        ),
    ];
    // Droplets thrown clear, and the bubbles they leave: each rises in pitch
    // and is placed off the beat so the splash does not sound sequenced. They
    // carry most of the splash's tail, so they are mixed high enough to be
    // heard over the body rather than buried under it.
    for (i, (offset, f)) in [
        (0.055, 900.0),
        (0.105, 1250.0),
        (0.175, 760.0),
        (0.245, 1080.0),
        (0.330, 860.0),
        (0.440, 1180.0),
    ]
    .into_iter()
    .enumerate()
    {
        let len = if i % 2 == 0 { 0.05 } else { 0.065 };
        parts.push((at(offset), bubble(f, f * 1.9, len, 0.26)));
    }
    // The body going under: deeper bubbles, a beat after the hit and again as
    // the water settles.
    parts.push((at(0.13), bubble(210.0, 470.0, 0.16, 0.30)));
    parts.push((at(0.30), bubble(260.0, 540.0, 0.12, 0.18)));

    mix(&parts)
}

/// One swim stroke: a soft band of water moving past the arm, with a bubble
/// trailing it. Short and quiet on purpose -- the Ear repeats it while the
/// body swims, so it has to be a texture rather than an announcement.
fn swim_voice() -> Vec<f32> {
    let at = |secs: f32| (secs * RATE as f32) as usize;
    let swell = moving_noise(
        0.24,
        73,
        0.7,
        |t| 620.0 + 520.0 * (std::f32::consts::PI * t / 0.24).sin(),
        |t| 0.62 * (std::f32::consts::PI * t / 0.24).sin().max(0.0).powf(1.2),
    );
    mix(&[
        (0, swell),
        (at(0.05), bubble(380.0, 820.0, 0.07, 0.20)),
        (at(0.15), bubble(560.0, 1150.0, 0.05, 0.13)),
        (0, sweep(230.0, 160.0, 0.10, 0.14, Wave::Sine)),
    ])
}

/// The pool as a place: bands of noise that slide, a rumble underneath, and
/// bubbles coming up through it.
fn water_bed() -> Vec<f32> {
    let secs = 6.0;
    let fade = (0.35 * RATE as f32) as usize;
    let n = (secs * RATE as f32) as usize + fade;
    let mut out = vec![0.0f32; n];
    let mut state = 0x51ED_2B7Bu32;
    let mut body = Svf::default();
    let mut surface = Svf::default();
    let mut low = 0.0f32;
    let mut hp_in = 0.0f32;
    let mut hp_out = 0.0f32;

    for (i, sample) in out.iter_mut().enumerate() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let white = (state >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
        let t = i as f32 / RATE as f32;
        let tau = std::f32::consts::TAU;

        // The body of the water: a band around 400-600 Hz that wanders, so
        // the pool is never one steady note. A slow lap (about one every
        // three seconds) lifts it and lets it fall again, and a fast, slightly
        // irregular chop rides on top -- turbulence, not a tremolo. The chop
        // is what stops the band sounding like wind: air moves in long
        // swells, water also moves in short ones.
        let body_f = 430.0 + 140.0 * (tau * 0.23 * t).sin() + 60.0 * (tau * 0.09 * t + 1.3).sin();
        let lap = (tau * 0.345 * t + 0.5).sin().max(0.0).powf(1.5);
        let chop = 1.0
            + 0.16 * (tau * 8.3 * t + 2.0 * (tau * 0.37 * t).sin()).sin()
            + 0.10 * (tau * 3.7 * t + 0.7).sin();
        let body_a = (0.55 + 0.20 * (tau * 0.19 * t).sin()) * (0.65 + 0.55 * lap) * chop;
        let wet = body.band(white, body_f, 0.65) * body_a;

        // The surface: a brighter band that shimmers faster, so the bed has
        // air above the body instead of one dull band. It is deliberately not
        // a whisper: water's top end is a large part of what identifies it.
        let surf_f = 1650.0 + 480.0 * (tau * 0.41 * t).sin() + 220.0 * (tau * 0.17 * t + 0.8).sin();
        let surf_a = (0.24 + 0.08 * (tau * 0.53 * t + 1.0).sin().max(0.0))
            * (1.0 + 0.20 * (tau * 5.9 * t + 1.1).sin());
        let shimmer = surface.band(white, surf_f, 0.8) * surf_a;

        // Spray: a breath of high-passed noise over everything, so droplets
        // and splash are implied even when nothing is entering the pool.
        hp_out = 0.35 * (hp_out + white - hp_in);
        hp_in = white;
        let air = hp_out * 0.07 * (0.75 + 0.25 * (tau * 0.29 * t).sin());

        // The depth underneath: a one-pole low-pass, slow and quiet, so the
        // pool has weight rather than only a surface.
        low += (white - low) * 0.02;
        let depth = low * 0.45 * (0.80 + 0.20 * (tau * 0.11 * t).sin());

        *sample = wet + shimmer + air + depth;
    }

    // Bubbles rising through the bed. Scheduled at irregular gaps by `scatter`
    // rather than a fixed period: a pool that pops in time is a metronome.
    let mut k = 0u32;
    let mut at = 0.18f32;
    let end = n as f32 / RATE as f32;
    while at < end {
        let h = scatter(k.wrapping_add(0x9E37_79B9));
        let f0 = 250.0 + (h % 300) as f32;
        let len = 0.035 + ((h >> 8) & 31) as f32 * 0.0012;
        let vol = 0.28 + ((h >> 13) & 15) as f32 * 0.012;
        // Every third bubble is a deeper gloop that holds longer: a pool
        // coming up in ones and twos reads as alive, one that only ever
        // emits the same pop reads as a machine.
        let (f0, len, vol) = if k % 3 == 2 {
            (170.0 + (h % 90) as f32, 0.09, vol + 0.25)
        } else {
            (f0, len, vol)
        };
        place(
            &mut out,
            (at * RATE as f32) as usize,
            bubble(f0, f0 * 2.05, len, vol),
            1.0,
        );
        at += 0.20 + ((h >> 16) % 34) as f32 * 0.01;
        k = k.wrapping_add(1);
    }

    // Droplets off the surface: high, short chirps that give the top of the
    // bed the sparkle a body of water has and a band of noise cannot.
    let mut at = 0.42f32;
    let mut d = 0u32;
    while at < end {
        let h = scatter(d.wrapping_add(0x51ED_2701));
        let f0 = 1000.0 + (h % 700) as f32;
        place(
            &mut out,
            (at * RATE as f32) as usize,
            bubble(f0, f0 * 1.6, 0.028, 0.11),
            1.0,
        );
        at += 0.55 + ((h >> 12) % 60) as f32 * 0.01;
        d = d.wrapping_add(1);
    }

    let mut bed = loopify(out, fade);
    normalize(&mut bed, 0.85);
    bed
}

/// The lava bed: a brown-noise rumble with a low beat and occasional bubbles.
fn lava_bed() -> Vec<f32> {
    let secs = 6.0;
    let fade = (0.35 * RATE as f32) as usize;
    let n = (secs * RATE as f32) as usize;
    let mut state = 0x1234_5678u32;
    let mut brown = 0.0f32;
    let mut low = 0.0f32;
    let mut out = Vec::with_capacity(n + fade);
    for i in 0..n + fade {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let white = (state >> 9) as f32 / (1u32 << 23) as f32 - 1.0;
        // Integrated noise is brown noise; the leak keeps it from walking off
        // to a constant, and the second pole takes the fizz off it.
        brown = (brown + white * 0.015).clamp(-1.0, 1.0) * 0.999;
        low += (brown - low) * 0.08;
        let t = i as f32 / RATE as f32;
        let swell = 0.70 + 0.30 * (std::f32::consts::TAU * 0.21 * t).sin();
        let mut s = low * 1.5 * swell + (std::f32::consts::TAU * 46.0 * t).sin() * 0.05;
        // A bubble every so often, on a deterministic slot so the loop is the
        // same every time. Sparse on purpose: a pool that pops constantly is
        // a frying pan.
        let slot = i as u32 / (RATE / 5);
        let pop = (i as u32 % (RATE / 5)) as f32 / RATE as f32;
        let roll = slot.wrapping_mul(2_654_435_761).wrapping_add(7) >> 29;
        if roll % 5 == 0 && pop < 0.09 {
            let pitch = 110.0 + (roll % 90) as f32;
            s += (std::f32::consts::TAU * pitch * pop).sin() * 0.18 * (-pop * 40.0).exp();
        }
        out.push(s * 0.5);
    }
    let mut bed = loopify(out, fade);
    normalize(&mut bed, 0.85);
    bed
}

/// How close a body is to water and to lava, each in `0..=1`, for the beds.
///
/// Measured in tiles from the body's centre to the nearest cell of each kind
/// within [`ELEMENT_REACH`], with a soft falloff so walking towards a pool
/// brings its sound up rather than switching it on at a line. Being inside
/// water is full, whatever the geometry says.
pub fn proximity(level: &Level, x: f32, y: f32, in_water: bool) -> (f32, f32) {
    let mut water: f32 = if in_water { 1.0 } else { 0.0 };
    let mut lava = 0.0f32;
    let tx = x.floor() as i32;
    let ty = y.floor() as i32;
    for dy in -ELEMENT_REACH..=ELEMENT_REACH {
        for dx in -ELEMENT_REACH..=ELEMENT_REACH {
            let Some(tile) = level.tile(tx + dx, ty + dy) else {
                continue;
            };
            if !matches!(tile, Tile::Water | Tile::Lava) {
                continue;
            }
            // Distance to the cell's *rectangle*, not its centre: a body
            // standing on a pool's rim is beside the water, and it should
            // hear it before it lands in it.
            let gap_x = ((tx + dx) as f32 + 0.5 - x).abs() - 0.5;
            let gap_y = ((ty + dy) as f32 + 0.5 - y).abs() - 0.5;
            let distance = (gap_x.max(0.0).powi(2) + gap_y.max(0.0).powi(2)).sqrt();
            let near = ((1.0 - distance / ELEMENT_REACH as f32).clamp(0.0, 1.0)).powf(1.4);
            if tile == Tile::Water {
                water = water.max(near);
            } else {
                lava = lava.max(near);
            }
        }
    }
    (water, lava)
}

/// How far a body hears an element, in tiles.
const ELEMENT_REACH: i32 = 5;

/// The song's parts, kept separate so the balance is testable: the counter,
/// the bass, the arpeggio and the drums, each rendered on its own.
struct Tracks {
    counter: Vec<f32>,
    bass: Vec<f32>,
    arp: Vec<f32>,
    drums: Vec<f32>,
}

/// Render every part of the song. The arrangement is data: each part is a
/// pass over the harmony or the written notes, so a section can change
/// without touching the synth.
fn music_tracks() -> Tracks {
    let grid = grid_samples();
    let bar_len = grid * STEPS_PER_BAR;
    let loop_len = bar_len * MUSIC_BARS;
    let chords = harmony();

    // Counterline: held triangles under the B section only.
    let mut counter = vec![0.0f32; loop_len];
    for held in counter_notes() {
        let secs = step_secs() * held.len as f32 * 0.98;
        let at = held.bar * bar_len + held.step * grid;
        place(
            &mut counter,
            at,
            note(midi(held.midi), secs, 0.085, Wave::Triangle),
            1.0,
        );
    }

    // Bass: the NES triangle's job. Eighths roll root-octave-root-fifth, the
    // B section halves the rhythm, and the bridge repeats the root to drive.
    let mut bass = vec![0.0f32; loop_len];
    for (index, ch) in chords.iter().enumerate() {
        let at = |step: usize| index * bar_len + step * grid;
        if (B_BAR..BRIDGE_BAR).contains(&index) {
            for (step, off, steps) in [(0usize, 0i32, 5.4f32), (8, 7, 3.6), (12, 12, 3.6)] {
                place(
                    &mut bass,
                    at(step),
                    bass_voice(midi(ch.root + off), step_secs() * steps, 0.20),
                    1.0,
                );
            }
        } else {
            let offsets: [i32; 8] = if (BRIDGE_BAR..A3_BAR).contains(&index) {
                [0, 0, 7, 0, 12, 0, 7, 12]
            } else {
                [0, 12, 0, 7, 0, 12, 0, 7]
            };
            for (j, off) in offsets.iter().enumerate() {
                place(
                    &mut bass,
                    at(j * 2),
                    bass_voice(midi(ch.root + off), step_secs() * 1.8, 0.20),
                    1.0,
                );
            }
        }
    }

    // Arpeggio: chord tones two octaves up. Quarters in the intro and the B
    // section, sixteenths everywhere else, so the texture breathes with the
    // arrangement instead of droning through it.
    let mut arp = vec![0.0f32; loop_len];
    for (index, ch) in chords.iter().enumerate() {
        let at = |step: usize| index * bar_len + step * grid;
        let tones = ch.tones();
        let sparse = index < A_BAR || (B_BAR..BRIDGE_BAR).contains(&index);
        let (pattern, every, vol): (&[usize], usize, f32) = if sparse {
            (&[0, 2, 1, 3], 4, 0.045)
        } else {
            (&[0, 1, 2, 1, 0, 1, 2, 3, 2, 1, 0, 1, 2, 1, 0, 1], 1, 0.05)
        };
        for (j, idx) in pattern.iter().enumerate() {
            let pitch = ch.root + 24 + tones[*idx % tones.len()];
            place(
                &mut arp,
                at(j * every),
                pluck_voice(midi(pitch), step_secs() * 1.4, vol),
                1.0,
            );
        }
    }

    // Drums: noise-channel bursts, styled by section.
    let mut drums = vec![0.0f32; loop_len];
    for index in 0..MUSIC_BARS {
        place_drums(&mut drums, index, grid, bar_len);
    }

    Tracks {
        counter,
        bass,
        arp,
        drums,
    }
}

/// The drum part for one bar: kick and snare on the pulse, hats by section,
/// a fill at every phrase end, and a crash where a section opens.
fn place_drums(track: &mut [f32], index: usize, grid: usize, bar_len: usize) {
    let at = |step: usize| index * bar_len + step * grid;
    let intro = index < A_BAR;
    let half = intro || (B_BAR..BRIDGE_BAR).contains(&index);
    let build = (BRIDGE_BAR..A3_BAR).contains(&index);
    let fill = matches!(index, 3 | 11 | 19 | 27 | 35 | 43 | 47);
    let crash = matches!(
        index,
        A_BAR | A2_BAR | B_BAR | BRIDGE_BAR | A3_BAR | OUTRO_BAR
    );

    if crash {
        place(track, at(0), hat_voice(true), 0.20);
    }

    // The groove proper arrives in the intro's third bar; before that the
    // hats alone count the tempo in.
    if !intro || index >= 2 {
        for step in [0, 8] {
            place(track, at(step), kick_voice(), 0.55);
        }
        if build {
            place(track, at(6), kick_voice(), 0.40);
            place(track, at(14), kick_voice(), 0.40);
        }
        for step in [4, 12] {
            place(track, at(step), snare_voice(), 0.42);
        }
    }

    if build {
        for step in 0..STEPS_PER_BAR {
            let gain = if step % 4 == 0 { 0.30 } else { 0.17 };
            place(track, at(step), hat_voice(false), gain);
        }
    } else if half {
        for step in [0, 4, 8, 12] {
            place(track, at(step), hat_voice(false), 0.28);
        }
    } else {
        for step in (0..STEPS_PER_BAR).step_by(2) {
            let gain = if step % 4 == 0 { 0.30 } else { 0.20 };
            place(track, at(step), hat_voice(false), gain);
        }
        place(track, at(14), hat_voice(true), 0.22);
    }

    if fill {
        for step in 8..16 {
            let gain = 0.22 + (step - 8) as f32 * 0.03;
            place(track, at(step), snare_voice(), gain);
        }
        place(
            track,
            at(12),
            sweep(240.0, 120.0, 0.12, 0.4, Wave::Triangle),
            0.40,
        );
        place(
            track,
            at(14),
            sweep(200.0, 96.0, 0.12, 0.4, Wave::Triangle),
            0.40,
        );
    }
}

/// The song the player hears, as one loop of samples.
fn music_loop() -> Vec<f32> {
    let tracks = music_tracks();
    let mut out = vec![0.0f32; tracks.bass.len()];
    for (track, gain) in [
        (&tracks.counter, 1.1f32),
        (&tracks.bass, 0.95),
        (&tracks.arp, 1.3),
        (&tracks.drums, 1.1),
    ] {
        for (o, s) in out.iter_mut().zip(track.iter()) {
            *o += s * gain;
        }
    }
    normalize(&mut out, 0.9);
    out
}

// ---------------------------------------------------------------------------
// The player: a dedicated thread that owns the audio device.
// ---------------------------------------------------------------------------

/// The device handle is not `Send` on every platform rodio supports, so it
/// cannot live in a static the frame loop touches. It lives here instead: one
/// thread owns the stream, and the game sends it effects down a channel. The
/// frame loop's whole cost per sound is one channel send, and a slow or
/// absent device can never stall a frame.
enum Cmd {
    Play(Sfx, f32),
    /// How loud the water and lava beds should be, each in `0..=1`.
    Ambience(f32, f32),
    SetMuted(bool),
}

/// The most either bed reaches at full proximity. Under the music, because
/// the beds are the room the game is in and the music is the song it plays.
const WATER_BED_GAIN: f32 = 0.32;
const LAVA_BED_GAIN: f32 = 0.36;

struct Player {
    // Kept alive for the stream's lifetime; dropping it would close the device.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    music: Sink,
    water: Sink,
    lava: Sink,
}

impl Player {
    fn open() -> Option<Player> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        let music = Sink::try_new(&handle).ok()?;
        let looped = SamplesBuffer::new(1, RATE, music_loop()).repeat_infinite();
        music.append(looped);
        music.set_volume(MUSIC_VOLUME);

        // The element beds run from the start and are simply silent until the
        // window says a body is near one: pausing and resuming a sink every
        // frame a player walks along a shore would click, so the loop keeps
        // playing and its volume does the work.
        let water = Sink::try_new(&handle).ok()?;
        water.append(SamplesBuffer::new(1, RATE, water_bed()).repeat_infinite());
        water.set_volume(0.0);
        let lava = Sink::try_new(&handle).ok()?;
        lava.append(SamplesBuffer::new(1, RATE, lava_bed()).repeat_infinite());
        lava.set_volume(0.0);

        Some(Player {
            _stream: stream,
            handle,
            music,
            water,
            lava,
        })
    }

    fn run(self, rx: std::sync::mpsc::Receiver<Cmd>) {
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Play(kind, gain) => {
                    let source = SamplesBuffer::new(1, RATE, render(kind))
                        .amplify(gain.clamp(0.0, 1.0) * SFX_LEVEL);
                    let _ = self.handle.play_raw(source.convert_samples::<f32>());
                }
                Cmd::Ambience(water, lava) => {
                    self.water
                        .set_volume(water.clamp(0.0, 1.0) * WATER_BED_GAIN);
                    self.lava.set_volume(lava.clamp(0.0, 1.0) * LAVA_BED_GAIN);
                }
                Cmd::SetMuted(true) => {
                    self.music.pause();
                    self.water.pause();
                    self.lava.pause();
                }
                Cmd::SetMuted(false) => {
                    self.music.play();
                    self.water.play();
                    self.lava.play();
                }
            }
        }
    }
}

static SENDER: OnceLock<Mutex<Option<std::sync::mpsc::Sender<Cmd>>>> = OnceLock::new();
static MUTED: AtomicBool = AtomicBool::new(false);

fn sender() -> &'static Mutex<Option<std::sync::mpsc::Sender<Cmd>>> {
    SENDER.get_or_init(|| Mutex::new(None))
}

fn send(cmd: Cmd) {
    if let Some(tx) = sender().lock().unwrap().as_ref() {
        let _ = tx.send(cmd);
    }
}

/// Start the sound thread and, on it, open the audio device and start the
/// music loop. Fails quietly (no device, a headless VM): the thread simply
/// never opens a player and drains its channel, so sound is a garnish, never
/// a requirement. A game that will not start without speakers has its
/// priorities wrong.
pub fn init() {
    let slot = sender();
    let mut guard = slot.lock().unwrap();
    if guard.is_some() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
    *guard = Some(tx);
    drop(guard);
    std::thread::Builder::new()
        .name("sound".into())
        .spawn(move || {
            if let Some(player) = Player::open() {
                if MUTED.load(Ordering::Relaxed) {
                    player.music.pause();
                    player.water.pause();
                    player.lava.pause();
                }
                player.run(rx);
            }
        })
        .ok();
}

/// Play one effect at a relative gain (a ghost's stomp is quieter than the
/// player's). No-op before [`init`], after a failed init, or while muted.
pub fn play(kind: Sfx, gain: f32) {
    if MUTED.load(Ordering::Relaxed) {
        return;
    }
    send(Cmd::Play(kind, gain));
}

/// Flip the mute; returns the new state. Bound to `M` in the app shell and to
/// the dock's sound plate, which is on every screen.
pub fn toggle_mute() -> bool {
    let muted = !MUTED.load(Ordering::Relaxed);
    MUTED.store(muted, Ordering::Relaxed);
    send(Cmd::SetMuted(muted));
    muted
}

/// Is sound muted? The window draws that state on its sound plate, so it asks
/// rather than keeping a second copy that could disagree with the player.
pub fn muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

/// How loud the water and lava beds should be, each in `0..=1`. Called every
/// frame from the session (or replay) that has a body near an element; sending
/// it when nothing is near is what fades the beds out again.
pub fn set_ambience(water: f32, lava: f32) {
    send(Cmd::Ambience(water.clamp(0.0, 1.0), lava.clamp(0.0, 1.0)));
}

// ---------------------------------------------------------------------------
// The ear: a per-episode diff that turns sim changes into effects.
// ---------------------------------------------------------------------------

/// Watches one episode. Feed it the sim after every tick; it plays what the
/// tick did. One ear per audible episode: the player's own at full gain, a
/// ghost or a replay's star quieter, and the splash's demo not at all (the
/// session that runs it is built muted, see `play::Session::set_audible`).
pub struct Ear {
    jumps: u32,
    wall_jumps: u32,
    stomps: u32,
    springs: u32,
    checkpoints: u32,
    crumbles: u32,
    grounded: bool,
    airborne_ticks: u32,
    outcome: Outcome,
    shots: usize,
    frames: u32,
    last_shot_frame: u32,
    /// Whether the body was in water last tick: entry is the splash.
    in_water: bool,
    /// Last frame a stroke was heard, so a held swim is strokes and not a
    /// continuous roar.
    last_swim: u32,
    /// Last frame a vent's burst was heard, so a row of vents firing on the
    /// same tick is one sound rather than a volley.
    last_burst: u32,
    started: bool,
}

impl Ear {
    pub fn new() -> Self {
        Ear {
            jumps: 0,
            wall_jumps: 0,
            stomps: 0,
            springs: 0,
            checkpoints: 0,
            crumbles: 0,
            grounded: true,
            airborne_ticks: 0,
            outcome: Outcome::Running,
            shots: 0,
            frames: 0,
            last_shot_frame: 0,
            in_water: false,
            last_swim: 0,
            last_burst: 0,
            started: false,
        }
    }

    /// Forget the episode so far and take the current sim as the baseline:
    /// the call for a restart, where the counters reset to zero and a naive
    /// diff would read the new episode's first jump as no jump at all.
    pub fn reset(&mut self, sim: &Sim) {
        let s = sim.stats();
        self.jumps = s.jumps;
        self.wall_jumps = s.wall_jumps;
        self.stomps = s.stomps;
        self.springs = s.springs;
        self.checkpoints = s.checkpoints;
        self.crumbles = s.crumble_burned;
        self.grounded = sim.player().grounded;
        self.airborne_ticks = 0;
        self.outcome = sim.outcome();
        self.shots = sim.shots().len();
        self.frames = s.frames;
        self.last_shot_frame = s.frames;
        self.in_water = sim.player().in_water;
        self.last_swim = s.frames;
        self.last_burst = s.frames;
        self.started = true;
    }

    /// The effects one tick produced, in the order a listener would rank
    /// them. Exposed (rather than playing directly) so the mapping from sim
    /// changes to sounds is testable without an audio device.
    pub fn hear(&mut self, sim: &Sim) -> Vec<Sfx> {
        if !self.started {
            self.reset(sim);
            return Vec::new();
        }
        let s = sim.stats();
        // A restarted episode restarts its frame counter: resync silently
        // rather than diffing across the seam.
        if s.frames < self.frames {
            self.reset(sim);
            return Vec::new();
        }
        self.frames = s.frames;
        let mut out = Vec::new();
        if s.wall_jumps > self.wall_jumps {
            out.push(Sfx::WallJump);
        } else if s.jumps > self.jumps {
            out.push(Sfx::Jump);
        }
        if s.stomps > self.stomps {
            out.push(Sfx::Stomp);
        }
        if s.springs > self.springs {
            out.push(Sfx::Spring);
        }
        if s.checkpoints > self.checkpoints {
            out.push(Sfx::Checkpoint);
        }
        if s.crumble_burned > self.crumbles {
            out.push(Sfx::Crumble);
        }
        let grounded = sim.player().grounded;
        if grounded {
            // Only a real fall lands audibly; stepping off a one-tile lip is
            // not a landing, it is walking.
            if !self.grounded && self.airborne_ticks >= 7 {
                out.push(Sfx::Land);
            }
            self.airborne_ticks = 0;
        } else {
            self.airborne_ticks += 1;
        }
        // Water: the splash of going in, then one stroke at a time while the
        // body is moving through it. A held swim is a rhythm of strokes, not
        // a continuous roar -- the bed carries the room, this carries the
        // body in it.
        let player = sim.player();
        if player.in_water && !self.in_water {
            out.push(Sfx::Splash);
        }
        if player.in_water
            && s.frames >= self.last_swim + 16
            && (player.vy < -0.005 || player.vx.abs() > 0.02)
        {
            out.push(Sfx::Swim);
            self.last_swim = s.frames;
        }
        self.in_water = player.in_water;
        // Lava vents: a ball is a thump on the first tick it is observed,
        // where `ticks` is one (the sim spawns it and ticks it in the same
        // step). Only when the body is close enough to hear it -- a vent
        // across the level must not fizz at the player from nowhere. The
        // frame throttle collapses a row of vents firing together into one
        // sound rather than a volley.
        let fresh_burst = {
            let cx = player.x + player.w * 0.5;
            let cy = player.y + player.h * 0.5;
            sim.lava_balls().iter().any(|b| {
                let dx = b.x + b.w * 0.5 - cx;
                let dy = b.y + b.h * 0.5 - cy;
                b.ticks <= 1 && dx * dx + dy * dy <= 12.0 * 12.0
            })
        };
        if fresh_burst && s.frames >= self.last_burst + 4 {
            out.push(Sfx::LavaBurst);
            self.last_burst = s.frames;
        }
        // Turret fire: throttled, because a corridor of turrets at full rate
        // is a noise, not a soundscape.
        let shots = sim.shots().len();
        if shots > self.shots && s.frames >= self.last_shot_frame + 8 {
            out.push(Sfx::Shot);
            self.last_shot_frame = s.frames;
        }
        self.shots = shots;
        let outcome = sim.outcome();
        if outcome != self.outcome {
            match outcome {
                Outcome::Won => out.push(Sfx::Win),
                Outcome::Died => out.push(Sfx::Death),
                _ => {}
            }
        }
        self.jumps = s.jumps;
        self.wall_jumps = s.wall_jumps;
        self.stomps = s.stomps;
        self.springs = s.springs;
        self.checkpoints = s.checkpoints;
        self.crumbles = s.crumble_burned;
        self.grounded = grounded;
        self.outcome = outcome;
        out
    }

    /// Observe a tick and play what it did at the given gain.
    pub fn observe(&mut self, sim: &Sim, gain: f32) {
        for kind in self.hear(sim) {
            play(kind, gain);
        }
    }
}

impl Default for Ear {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim::{AgentAction, Level};

    fn shipped(slug: &str) -> Level {
        let path = format!("{}/../../levels/{slug}.json", env!("CARGO_MANIFEST_DIR"));
        Level::from_json_file(path).unwrap()
    }

    /// Every effect the game can make, so a new one cannot miss the checks
    /// below without the list being changed on purpose.
    const EVERY_EFFECT: [Sfx; 15] = [
        Sfx::Jump,
        Sfx::WallJump,
        Sfx::Land,
        Sfx::Stomp,
        Sfx::Spring,
        Sfx::Splash,
        Sfx::Swim,
        Sfx::LavaBurst,
        Sfx::Checkpoint,
        Sfx::Crumble,
        Sfx::Shot,
        Sfx::Death,
        Sfx::Win,
        Sfx::MenuMove,
        Sfx::MenuConfirm,
    ];

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn every_effect_renders_a_bounded_nonempty_buffer() {
        for kind in EVERY_EFFECT {
            let samples = render(kind);
            assert!(!samples.is_empty(), "{kind:?} rendered nothing");
            assert!(
                samples.iter().all(|s| s.abs() <= 1.0),
                "{kind:?} exceeds the unit range"
            );
        }
    }

    #[test]
    fn the_music_keeps_headroom_over_the_effects() {
        // The complaint this guards against: effects loud enough that the song
        // underneath them disappears. The music's own peak (after its sink
        // volume) has to stay well above the loudest trimmed effect, and above
        // the element beds, or adding another effect or raising one's render
        // level quietly turns the soundtrack into an afterthought.
        let music = peak(&music_loop()) * MUSIC_VOLUME;
        let effect = EVERY_EFFECT
            .iter()
            .map(|kind| peak(&render(*kind)) * SFX_LEVEL)
            .fold(0.0f32, f32::max);
        assert!(
            music > effect * 1.5,
            "music peak {music:.2} is too close to the loudest effect {effect:.2}"
        );
        let bed = peak(&water_bed()) * WATER_BED_GAIN;
        assert!(
            bed < music * 0.7,
            "the water bed ({bed:.2}) crowds the music ({music:.2})"
        );
    }

    /// One bar of samples, as the song renders it.
    fn bar_samples() -> usize {
        grid_samples() * STEPS_PER_BAR
    }

    fn energy(samples: &[f32], from_bar: usize, bars: usize) -> f32 {
        let bar = bar_samples();
        samples[from_bar * bar..(from_bar + bars) * bar]
            .iter()
            .map(|s| s * s)
            .sum()
    }

    #[test]
    fn the_music_is_a_song_not_a_jingle() {
        // The complaint that started this rewrite: the old loop was fourteen
        // seconds and the ear wore it out in two passes. A song is a minute
        // at least, and it must still end on a bar line so the loop does not
        // restart mid-phrase.
        let samples = music_loop();
        let secs = samples.len() as f32 / RATE as f32;
        assert!(secs >= 70.0, "the loop is only {secs:.1}s long");
        assert_eq!(samples.len(), bar_samples() * MUSIC_BARS);
        assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.5, "the mix reaches a real level (peak {peak:.2})");
    }

    #[test]
    fn every_part_and_section_of_the_song_is_played() {
        let tracks = music_tracks();
        for (name, track) in [
            ("counter", &tracks.counter),
            ("bass", &tracks.bass),
            ("arp", &tracks.arp),
            ("drums", &tracks.drums),
        ] {
            let total: f32 = track.iter().map(|s| s * s).sum();
            assert!(total > 0.0, "{name} is silent");
        }
        // The arrangement changes section by section: the bass, the arpeggio
        // and the drums have to be playing in all of them, or a section has
        // been swallowed rather than varied. The counterline belongs to the B
        // section alone, which is the point of it.
        for start in [A_BAR, A2_BAR, B_BAR, BRIDGE_BAR, A3_BAR, OUTRO_BAR] {
            let bars = 8.min(MUSIC_BARS - start);
            for (name, track) in [
                ("bass", &tracks.bass),
                ("arp", &tracks.arp),
                ("drums", &tracks.drums),
            ] {
                let e = energy(track, start, bars);
                assert!(e > 0.0, "{name} is silent from bar {start}");
            }
        }
        assert!(energy(&tracks.counter, B_BAR, 8) > 0.0);
    }

    /// A pool two tiles deep with a rim, for the water sounds.
    fn pool_level() -> Level {
        let mut level = Level::starter("pool", 14, 10);
        for ty in 5..=8 {
            for tx in 2..=10 {
                level.set_tile(tx, ty, Tile::Water);
            }
        }
        level.set_spawn_tile(6, 3);
        level
    }

    /// A vent one tile from where the body stands: open lava in the floor.
    fn vent_level() -> Level {
        let mut level = Level::starter("vent", 16, 10);
        level.set_tile(8, 9, Tile::Lava);
        level.set_spawn_tile(4, 8);
        level
    }

    #[test]
    fn the_water_and_lava_beds_loop_without_a_click() {
        for (name, bed) in [("water", water_bed()), ("lava", lava_bed())] {
            assert!(!bed.is_empty(), "{name} bed rendered nothing");
            assert!(
                bed.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
                "{name} bed leaves the unit range"
            );
            let rms = (bed.iter().map(|s| s * s).sum::<f32>() / bed.len() as f32).sqrt();
            assert!(rms > 0.01, "{name} bed is inaudible (rms {rms:.4})");
            // The crossfade makes the wrap a continuation: the last sample
            // runs into the first, so a looped bed does not tick once a cycle.
            let jump = (bed[0] - bed[bed.len() - 1]).abs();
            assert!(jump < 0.05, "{name} bed clicks at the loop: {jump:.3}");
        }
    }

    /// How fast a signal changes sample to sample, relative to how loud it
    /// is: a rough stand-in for spectral centroid, and enough to tell a
    /// low rumble from a surface.
    fn brightness(samples: &[f32]) -> f32 {
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        let diff = (samples
            .windows(2)
            .map(|w| (w[1] - w[0]) * (w[1] - w[0]))
            .sum::<f32>()
            / (samples.len() - 1) as f32)
            .sqrt();
        diff / rms.max(1e-9)
    }

    #[test]
    fn the_water_bed_has_a_surface_not_just_a_rumble() {
        let bed = water_bed();
        // The old bed was one-pole low-passed noise: all of its energy sat at
        // the bottom of the spectrum, which is why it read as wind. A water
        // bed has to carry its mid band (a few hundred Hz) and some top.
        let bright = brightness(&bed);
        assert!(
            bright > 0.11,
            "the water bed is too dark to be a surface (brightness {bright:.3})"
        );
        // And it has to move: bubbles are transient peaks well above the
        // noise floor. A flat RMS at the bed's peak means the bed is a hiss.
        let rms = (bed.iter().map(|s| s * s).sum::<f32>() / bed.len() as f32).sqrt();
        let peak = peak(&bed);
        assert!(
            peak / rms > 3.2,
            "the water bed has no bubbles in it (crest {:.2})",
            peak / rms
        );
    }

    #[test]
    fn a_bubble_rises_in_pitch() {
        // The rise is the cue that makes a short chirp read as a bubble
        // rather than a beep, so it is the one property worth pinning.
        let b = bubble(300.0, 900.0, 0.06, 1.0);
        let crossing_rate = |s: &[f32]| {
            s.windows(2)
                .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
                .count() as f32
                / (s.len() as f32 / RATE as f32)
        };
        let head = &b[..b.len() / 4];
        let tail = &b[b.len() * 3 / 4..];
        assert!(
            crossing_rate(tail) > crossing_rate(head) * 1.4,
            "the bubble does not rise: {:.0} Hz then {:.0} Hz",
            crossing_rate(head),
            crossing_rate(tail)
        );
    }

    #[test]
    fn a_splash_is_bright_then_dark_and_keeps_bubbling() {
        let splash = render(Sfx::Splash);
        assert!(
            splash.len() > (0.45 * RATE as f32) as usize,
            "a splash is an event with a tail, not a click"
        );
        let head = &splash[..(0.07 * RATE as f32) as usize];
        assert!(
            brightness(head) > 0.35,
            "the hit should be bright spray, not a thud (brightness {:.3})",
            brightness(head)
        );
        // The tail is not silence: the bubbles and droplets are still coming
        // up after the surface has closed.
        let tail = &splash[splash.len() - (0.12 * RATE as f32) as usize..];
        let tail_rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        assert!(
            tail_rms > 0.01,
            "the splash has no tail, only a hit (rms {tail_rms:.4})"
        );
    }

    #[test]
    fn a_stroke_is_audible_under_the_music() {
        let swim = render(Sfx::Swim);
        // Effects are trimmed by SFX_LEVEL on the sink; a stroke still has to
        // clear the song at its own peak, or swimming is silent.
        assert!(
            peak(&swim) * SFX_LEVEL > 0.12,
            "a stroke is too quiet to hear (peak {:.3})",
            peak(&swim) * SFX_LEVEL
        );
        assert!(brightness(&swim) > 0.2, "a stroke is a hiss, not water");
    }

    #[test]
    fn proximity_falls_off_with_distance_from_water_and_lava() {
        let mut level = Level::starter("elements", 20, 12);
        for tx in 2..=6 {
            level.set_tile(tx, 8, Tile::Water);
        }
        level.set_tile(14, 8, Tile::Lava);

        // Standing over the pool: water loud, lava across the level silent.
        let (water, lava) = proximity(&level, 4.5, 7.0, false);
        assert!(water > 0.5, "the pool should be loud, got {water:.2}");
        assert!(lava < 0.05, "distant lava should be silent, got {lava:.2}");

        // Standing over the vent: the other way round.
        let (water, lava) = proximity(&level, 14.5, 7.0, false);
        assert!(lava > 0.5, "the vent should be loud, got {lava:.2}");
        assert!(
            water < 0.05,
            "distant water should be silent, got {water:.2}"
        );

        // Far above both, out of earshot.
        let (water, lava) = proximity(&level, 4.5, 2.0, false);
        assert_eq!((water, lava), (0.0, 0.0));

        // And being *in* water is full, whatever the cells around say.
        let (water, _) = proximity(&level, 4.5, 8.5, true);
        assert_eq!(water, 1.0);
    }

    #[test]
    fn a_body_is_heard_entering_water_and_swimming_in_it() {
        let level = pool_level();
        let mut sim = Sim::new(&level, 5);
        let mut ear = Ear::new();
        ear.reset(&sim);
        let mut heard = Vec::new();
        for _ in 0..60 {
            sim.step(AgentAction::Walk);
            heard.extend(ear.hear(&sim));
        }
        assert!(sim.player().in_water, "the body should have fallen in");
        assert!(heard.contains(&Sfx::Splash), "heard {heard:?}");

        heard.clear();
        for _ in 0..40 {
            sim.step(AgentAction::JumpInPlace);
            heard.extend(ear.hear(&sim));
        }
        assert!(
            heard.contains(&Sfx::Swim),
            "a held swim should stroke, heard {heard:?}"
        );
    }

    #[test]
    fn a_lava_vent_is_heard_when_it_fires_near_the_body() {
        let level = vent_level();
        let mut sim = Sim::new(&level, 3);
        let mut ear = Ear::new();
        ear.reset(&sim);
        let mut heard = Vec::new();
        for _ in 0..220 {
            sim.step(AgentAction::Walk);
            heard.extend(ear.hear(&sim));
        }
        assert!(
            heard.contains(&Sfx::LavaBurst),
            "the vent should have thrown a ball by now, heard {heard:?}"
        );
    }

    #[test]
    fn a_jump_is_heard_as_a_jump() {
        let level = shipped("l1-flat-run");
        let mut sim = Sim::new(&level, 11);
        let mut ear = Ear::new();
        ear.reset(&sim);
        // Run right and jump: the flat level's opening is floor, so the body
        // leaves the ground by a jump and nothing else.
        let mut heard = Vec::new();
        for _ in 0..40 {
            sim.step(AgentAction::RunJump);
            heard.extend(ear.hear(&sim));
        }
        assert!(heard.contains(&Sfx::Jump), "heard {heard:?}");
        assert!(!heard.contains(&Sfx::Death), "heard {heard:?}");
    }

    #[test]
    fn play_before_init_is_a_quiet_no_op() {
        // No app shell has run in a test, so there is no player: playing,
        // setting ambience and muting must neither panic nor need a device.
        play(Sfx::Jump, 1.0);
        set_ambience(1.0, 1.0);
        let before = muted();
        assert_eq!(toggle_mute(), !before);
        assert_eq!(
            muted(),
            !before,
            "the state the dock reads follows the flip"
        );
        assert_eq!(toggle_mute(), before);
        assert_eq!(muted(), before);
    }
}
