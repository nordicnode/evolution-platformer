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

/// The music loop: eight bars of a minor-key chiptune at low volume, bass and
/// a sparse lead, generated once and looped by the player. Kept deliberately
/// plain: it is a backdrop for watching an agent learn, not a performance.
fn music_loop() -> Vec<f32> {
    // A2 C3 E3 G2 | A2 C3 E3 B2 ... roots of Am, Am, F, G in the second half.
    let bass_line: [f32; 16] = [
        110.0, 110.0, 130.81, 164.81, 110.0, 110.0, 98.0, 123.47, //
        87.31, 87.31, 110.0, 130.81, 98.0, 98.0, 123.47, 146.83,
    ];
    let step_secs = 0.21;
    let step = (step_secs * RATE as f32) as usize;
    let mut parts: Vec<(usize, Vec<f32>)> = Vec::new();
    for (i, &f) in bass_line.iter().enumerate() {
        parts.push((i * step, tone(f, step_secs * 0.95, 0.34, Wave::Square)));
        parts.push((
            i * step + step / 2,
            tone(f * 2.0, step_secs * 0.4, 0.1, Wave::Sine),
        ));
    }
    // The lead answers every other bar with three notes of the chord.
    let lead: [(usize, f32); 6] = [
        (0, 440.0),
        (2, 523.25),
        (4, 659.25),
        (8, 349.23),
        (10, 440.0),
        (12, 523.25),
    ];
    for (i, f) in lead {
        parts.push((i * step, tone(f, step_secs * 1.8, 0.13, Wave::Sine)));
    }
    // Exactly sixteen steps, padded: a loop whose length is not a whole
    // number of steps restarts mid-note and clicks once a cycle, forever.
    let mut out = mix(&parts);
    out.resize(step * 16, 0.0);
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
        let step = (0.21 * RATE as f32) as usize;
        assert_eq!(samples.len() % step, 0, "loop must end on a step boundary");
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
