//! Sound, synthesised: every effect is generated sample by sample in this
//! module, so the game ships no audio assets and the sounds match the pixel
//! art's register (square waves, short envelopes, nothing sampled).
//!
//! Two halves. [`Ear`] watches a [`Sim`] and turns what changed between two
//! ticks into effects: the episode stats already count jumps, stomps, springs
//! and wall jumps, so an ear is a diff, not a second physics. [`play`] sends
//! an effect to the speakers through one process-wide player; before the app
//! shell calls [`init`] (or on a machine with no audio device, where init
//! fails quietly) every call is a no-op, which is what lets the headless
//! tests and the screenshot runs construct sessions without a sound card.
//!
//! The sim is never touched: an ear only reads, so determinism, replays and
//! state hashes are exactly what they were in silence.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use sim::{Outcome, Sim};

/// Sample rate for everything generated here. Half of CD rate is plenty for
/// square waves and halves the synthesis cost of the music loop.
const RATE: u32 = 22_050;

/// One sound the game can make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Jump,
    WallJump,
    Land,
    Stomp,
    Spring,
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
    /// audible under the bass. What the melody is played on, because a square
    /// lead over a marching bass is a circus and the game is not one.
    Triangle,
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
    let len = parts.iter().map(|(off, p)| off + p.len()).max().unwrap_or(0);
    let mut out = vec![0.0f32; len];
    for (off, part) in parts {
        for (i, s) in part.iter().enumerate() {
            out[off + i] = (out[off + i] + s).clamp(-1.0, 1.0);
        }
    }
    out
}

/// A melody note with a sustain envelope: fast attack, a held body, and the
/// release saved for the end. The effects all decay (a blip should), but a
/// tune made of decays is a tune made of hiccups: the lead has to *hold* its
/// notes the way a whistled one does.
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

/// The menu music, second arrangement. The first was a bassline and then a
/// tune, and both read as a circus: a square lead marching in quarter notes
/// over a root-and-fifth oom-pah bass is the carnival recipe, whatever the
/// notes are. This one is built for flow instead:
///
/// - the lead is a triangle wave (round, flute-ish) playing a mostly
///   stepwise eighth-note line with syncopation in it, phrases that run
///   two bars before they breathe;
/// - the bass rolls root–fifth–octave–fifth in eighths under it, a pulse
///   rather than a march;
/// - the arpeggio is quieter and smoother, chord tones cycling without
///   accent;
/// - the tempo is up a touch, 140 BPM, and the key is G, which sits
///   brighter than C on small speakers.
///
/// Still an original melody, still two phrases where the second opens like
/// the first and answers it differently, still exactly sixty-four steps.
fn music_loop() -> Vec<f32> {
    const STEP_SECS: f32 = 60.0 / 140.0 / 2.0;
    let step = (STEP_SECS * RATE as f32) as usize;
    let mut parts: Vec<(usize, Vec<f32>)> = Vec::new();

    // (start step, MIDI note, length in steps). G major: B4=71, D5=74,
    // E5=76, F#5=78, G5=79, A5=81, B5=83, D6=86.
    let lead: [(usize, i32, usize); 40] = [
        // Bars 1-2 (G, D): the line spills upward, then settles stepwise.
        (0, 74, 1),
        (1, 76, 1),
        (2, 79, 2),
        (4, 78, 1),
        (5, 76, 1),
        (6, 74, 2),
        (8, 71, 1),
        (9, 74, 1),
        (10, 78, 2),
        (12, 76, 1),
        (13, 74, 1),
        (14, 71, 2),
        // Bars 3-4 (Em, C): one leap to the top, then a long way down by steps.
        (16, 76, 1),
        (17, 79, 1),
        (18, 83, 2),
        (20, 81, 1),
        (21, 79, 1),
        (22, 76, 2),
        (24, 74, 1),
        (25, 76, 1),
        (26, 79, 2),
        (28, 76, 4),
        // Bars 5-6 (G, D): the same opening, a tail that lingers instead.
        (32, 74, 1),
        (33, 76, 1),
        (34, 79, 2),
        (36, 81, 1),
        (37, 79, 1),
        (38, 76, 2),
        (40, 78, 1),
        (41, 76, 1),
        (42, 74, 2),
        (44, 71, 4),
        // Bars 7-8 (C, G): the climb once more, ending on the tonic, held.
        (48, 76, 1),
        (49, 79, 1),
        (50, 81, 2),
        (52, 79, 1),
        (53, 78, 1),
        (54, 76, 1),
        (55, 74, 1),
        (56, 79, 6),
    ];
    for (at, m, len) in lead {
        parts.push((
            at * step,
            note(midi(m), STEP_SECS * len as f32 * 0.98, 0.23, Wave::Triangle),
        ));
        // A sine shadow at the same pitch takes the remaining edge off.
        parts.push((
            at * step,
            note(midi(m), STEP_SECS * len as f32 * 0.98, 0.07, Wave::Sine),
        ));
    }

    // One chord per bar: G, D, Em, C, twice. Roots in MIDI (G2=43, D2=38,
    // E2=40, C3=48) and the third's quality for the arpeggio.
    let bars: [(i32, bool); 8] = [
        (43, true),
        (38, true),
        (40, false),
        (48, true),
        (43, true),
        (38, true),
        (48, true),
        (43, true),
    ];
    for (bar, (root, major)) in bars.iter().enumerate() {
        let base = bar * 8;
        // The rolling bass: root, fifth, octave, fifth, in eighths, triangle
        // so it pulses rather than stamps.
        for (i, off) in [0, 7, 12, 7, 0, 7, 12, 7].iter().enumerate() {
            parts.push((
                (base + i) * step,
                note(midi(root + off), STEP_SECS * 0.95, 0.17, Wave::Triangle),
            ));
        }
        // The arpeggio: chord tones cycling smoothly two octaves up, sine,
        // quieter than everything else on purpose.
        let third = if *major { 4 } else { 3 };
        let tones = [0, third, 7, 12, third, 7, 12, third];
        for (i, off) in tones.iter().enumerate() {
            parts.push((
                (base + i) * step,
                note(midi(root + 24 + off), STEP_SECS * 0.95, 0.04, Wave::Sine),
            ));
        }
    }

    // Exactly sixty-four steps, padded: a loop whose length is not a whole
    // number of steps restarts mid-note and clicks once a cycle, forever.
    let mut out = mix(&parts);
    out.resize(step * 64, 0.0);
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
    SetMuted(bool),
}

struct Player {
    // Kept alive for the stream's lifetime; dropping it would close the device.
    _stream: OutputStream,
    handle: OutputStreamHandle,
    music: Sink,
}

impl Player {
    fn open() -> Option<Player> {
        let (stream, handle) = OutputStream::try_default().ok()?;
        let music = Sink::try_new(&handle).ok()?;
        let looped = SamplesBuffer::new(1, RATE, music_loop()).repeat_infinite();
        music.append(looped);
        music.set_volume(0.5);
        Some(Player {
            _stream: stream,
            handle,
            music,
        })
    }

    fn run(self, rx: std::sync::mpsc::Receiver<Cmd>) {
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Cmd::Play(kind, gain) => {
                    let source = SamplesBuffer::new(1, RATE, render(kind))
                        .amplify(gain.clamp(0.0, 1.0));
                    let _ = self.handle.play_raw(source.convert_samples::<f32>());
                }
                Cmd::SetMuted(true) => self.music.pause(),
                Cmd::SetMuted(false) => self.music.play(),
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

/// Flip the mute; returns the new state. Bound to `M` in the app shell.
pub fn toggle_mute() -> bool {
    let muted = !MUTED.load(Ordering::Relaxed);
    MUTED.store(muted, Ordering::Relaxed);
    send(Cmd::SetMuted(muted));
    muted
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
        let path = format!(
            "{}/../../levels/{slug}.json",
            env!("CARGO_MANIFEST_DIR")
        );
        Level::from_json_file(path).unwrap()
    }

    #[test]
    fn every_effect_renders_a_bounded_nonempty_buffer() {
        for kind in [
            Sfx::Jump,
            Sfx::WallJump,
            Sfx::Land,
            Sfx::Stomp,
            Sfx::Spring,
            Sfx::Checkpoint,
            Sfx::Crumble,
            Sfx::Shot,
            Sfx::Death,
            Sfx::Win,
            Sfx::MenuMove,
            Sfx::MenuConfirm,
        ] {
            let samples = render(kind);
            assert!(!samples.is_empty(), "{kind:?} rendered nothing");
            assert!(
                samples.iter().all(|s| s.abs() <= 1.0),
                "{kind:?} exceeds the unit range"
            );
        }
    }

    #[test]
    fn the_music_loop_is_bounded_and_loops_on_a_whole_step() {
        let samples = music_loop();
        assert!(samples.iter().all(|s| s.abs() <= 1.0));
        let step = (60.0 / 140.0 / 2.0 * RATE as f32) as usize;
        assert_eq!(
            samples.len(),
            step * 64,
            "the loop is exactly sixty-four eighth-note steps"
        );
    }

    #[test]
    fn the_lead_hook_repeats_with_a_variation() {
        // The tune's claim is a hook that comes back changed: the two phrases
        // open on the same three notes, so the loop's first and fifth bars
        // should sound alike in the lead's register but not identical. This
        // compares the rendered mix's energy in the two windows rather than
        // the note table, so a rewrite that loses the repetition fails here
        // even if every note is still in range.
        let samples = music_loop();
        let step = (60.0 / 140.0 / 2.0 * RATE as f32) as usize;
        let energy = |from: usize, to: usize| -> f32 {
            samples[from..to].iter().map(|s| s * s).sum()
        };
        let a = energy(0, step * 8);
        let b = energy(step * 32, step * 40);
        assert!(a > 0.0 && b > 0.0, "both phrases carry the lead");
        let ratio = a / b;
        assert!(
            (0.4..=2.5).contains(&ratio),
            "phrases are the same hook at similar weight (ratio {ratio:.2})"
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
        // No app shell has run in a test, so there is no player: playing must
        // neither panic nor require a device.
        play(Sfx::Jump, 1.0);
        let _ = toggle_mute();
        let _ = toggle_mute();
    }
}
