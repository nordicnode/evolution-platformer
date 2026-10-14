//! Observation vector builder (§7.2). **Versioned**: any change to the layout,
//! the encodings, or the action-repeat cadence is a `VERSION` bump, and every
//! champion file records it (§8).
//!
//! `N_INPUTS = 95`. The patch is deliberately 9×9 rather than 13×13: Togelius
//! 2009 showed small controllers beating large ones, and input size multiplies
//! the search space. Widening the patch is the documented upgrade path, gated on
//! evolution visibly plateauing on levels that should be solvable.
//!
//! v2 appends the three inputs the movement abilities need and changes what `vx`
//! means; every earlier index keeps its meaning and its position, so the diff between
//! v1 and v2 is additive and reviewable. An agent cannot reason about a wall it cannot
//! see or a sprint it cannot feel, so both are sensor changes rather than physics-only
//! ones — and that is why the version had to move: a v1 champion asked to drive a body
//! with nine actions and a wall slide would be building a policy on a lie.
//!
//! v3 replaces the horizontal `goal_dir` sign with a two-axis `goal_dx, goal_dy` vector.
//! That single sign was the observation's last assumption that the flag is always to the
//! right, and it is the one assumption a level designer can falsify in ten seconds by
//! stacking a tower: an agent at the foot of one read the same `+1` it reads at the mouth of
//! a corridor. The score had the same assumption baked in and was fixed in the same pass
//! (`EpisodeStats::progress_pixels` is now distance-to-the-flag rather than rightmost x), so
//! the sensor and the objective agree about what the goal is.
//!
//! v4 splits the single `gap_ahead` probe into `gap_right` and `gap_left`. The one probe was
//! the observation's last directional assumption, and it was the subtlest of the three: it
//! asked about the side the body was *moving* toward — `PlayerState::facing`, which reads
//! "right" whenever the body is standing still — so an agent at rest at the lip of a pit on
//! its left was told about the ground under its nose and nothing about the hole it was about
//! to choose. Every other sensor here describes the body or the world; that one described the
//! body's *last decision*, which is the one thing a sensor must not do, because the first
//! decision of an episode is exactly when a direction has to be chosen. Bumped rather than
//! patched in place: index 83 keeps its slot but changes meaning (`gap_ahead` as read by a
//! rightward body) and index 94 is new, which re-scores every champion ever trained.

use crate::level::{Level, Tile};
use crate::physics::{AgentAction, Physics, PlayerState, Walker};

/// Sensor layout version. Bump on *any* change below (§8).
///
/// v2: the appended wall and sprint inputs, and `vx` renormalised from `run_speed` to
/// `sprint_speed` (see [`observe`]).
///
/// v3: the single horizontal `goal_dir` sign became the two-component `goal_dx, goal_dy`
/// vector below. The sign was the last place in the *observation* that assumed the goal is
/// to the right, and it made a level's goal invisible whenever the answer was "up": an
/// agent standing at the foot of a tower read `+1` for a flag directly above its head, which
/// is the same reading as a flag at the end of a corridor.
///
/// v4: `gap_ahead` — a probe whose direction was `sign(vx)`, and therefore "right" for a
/// body at rest — became `gap_right` and `gap_left`, one slot per direction. See the module
/// docs.
pub const VERSION: u32 = 4;

/// Tiles per side of the local patch (`9×9 = 81` inputs).
pub const PATCH_RADIUS: i32 = 4;
/// Number of inputs in the observation vector.
pub const N_INPUTS: usize = 81 + 2 + 1 + 1 + 2 + 1 + 2 + 1 + 3 + 1;

/// Inputs appended in v2: wall side, wall slide, sprint charge.
pub const N_ABILITY_INPUTS: usize = 3;

/// Normalizer for the nearest-enemy vector (tiles).
pub const ENEMY_VIEW_DISTANCE: f32 = 4.0;
/// How far each gap probe looks, in tiles.
///
/// Three, the radius of the patch, so the probe is "the patch, summarised": a hole at the
/// fourth tile is visible in the 9×9 grid but not worth a dedicated scalar, which is the
/// division of labour between the patch and the feature slots.
pub const GAP_LOOKAHEAD: i32 = 3;

/// Normalizer for the goal vector (tiles).
///
/// Much longer than [`ENEMY_VIEW_DISTANCE`], and for a different job: the enemy vector is a
/// *threat* that only matters up close, while the goal is the one thing in the level that is
/// always relevant wherever it is. Sixteen tiles keeps useful resolution over most of a
/// shipped level (60-80 tiles wide, so the far half of a long level saturates) while still
/// reading "directly above me, about a jump away" precisely, which is where the precision
/// matters.
pub const GOAL_VIEW_DISTANCE: f32 = 16.0;

/// Tile encodings inside the patch.
pub const ENC_SOLID: f32 = 1.0;
pub const ENC_SPIKE: f32 = -0.5;
pub const ENC_EMPTY: f32 = 0.0;
pub const ENC_ENEMY: f32 = -1.0;

/// The plan's sensor table omits tiles for spikes, which would leave the single
/// most common hazard invisible to the agent. Spikes therefore get
/// [`ENC_SPIKE`] — distinct from, and less alarming than, an enemy.
#[inline]
fn encode_tile(t: Tile) -> f32 {
    match t {
        // Sand is solid, and an agent is told so: the observation is about what a body can
        // do, not what the world is made of. A material that looked different to the
        // sensors would be a new problem for the population to solve, which is the one
        // thing adding a *colour* of ground must never be.
        Tile::Solid | Tile::Sand => ENC_SOLID,
        Tile::Spike => ENC_SPIKE,
        Tile::Empty | Tile::Goal => ENC_EMPTY,
    }
}

/// Build the observation vector for the current sim state.
///
/// Layout (all values finite; unused slots are `0.0`):
///
/// | offset | count | meaning |
/// |---|---|---|
/// | 0   | 81 | 9×9 tile patch, row-major, top-left first |
/// | 81  | 2  | nearest live enemy `dx, dy`, normalised by [`ENEMY_VIEW_DISTANCE`] |
/// | 83  | 1  | gap to the right of the feet (1.0 = no ground within [`GAP_LOOKAHEAD`] tiles) |
/// | 84  | 1  | grounded |
/// | 85  | 2  | `vx`, `vy` normalised by `sprint_speed` / `max_fall` |
/// | 87  | 1  | can jump (grounded or coyote) |
/// | 88  | 2  | goal `dx, dy` from the body, normalised by [`GOAL_VIEW_DISTANCE`] |
/// | 90  | 1  | bias, constant 1.0 |
/// | 91  | 1  | wall side pressed into (−1 left / +1 right / 0 none) |
/// | 92  | 1  | wall slide active |
/// | 93  | 1  | sprint charge, 0 at `run_speed` and 1 at `sprint_speed` |
/// | 94  | 1  | gap to the left of the feet |
///
/// The two gap slots are a **pair, and neither is "forward"**. What they describe is the
/// ground immediately beside each edge of the body, which is the thing a jump decision is
/// about, and which side that is has nothing to do with where the body is already drifting.
///
/// Two notes on the v2 inputs. `vx` is normalised by `sprint_speed` rather than
/// `run_speed` on purpose: it used to saturate at 1.0 for a running agent, which would
/// have made a sprinting body and a running one *indistinguishable* — the one thing a
/// speed sensor exists to tell apart. Running now reads 0.67 and sprinting 1.0.
///
/// The wall side is the wall the body is *pressed against*, not merely next to. That is
/// deliberate: an agent cannot see "a wall is one tile to my right" from a sensor that
/// only fires on contact, but it can see the wall in its own 9×9 patch, so what it needs
/// from this input is the state of its own cling — and the patch plus this input is
/// enough to learn the press-then-jump rhythm a wall climb is made of.
pub fn observe(
    level: &Level,
    player: &PlayerState,
    walkers: &[Walker],
    ph: &Physics,
) -> [f32; N_INPUTS] {
    let mut out = [0.0f32; N_INPUTS];
    let (cx, cy) = player.aabb().center();
    let ctx = cx.floor() as i32;
    let cty = cy.floor() as i32;

    // ---- 9×9 tile patch -------------------------------------------------
    let mut i = 0;
    for dy in -PATCH_RADIUS..=PATCH_RADIUS {
        for dx in -PATCH_RADIUS..=PATCH_RADIUS {
            out[i] = encode_tile(level.get(ctx + dx, cty + dy));
            i += 1;
        }
    }
    debug_assert_eq!(i, 81);

    // ---- nearest live enemy --------------------------------------------
    // Ties break on walker index, so the observation is order-stable.
    let mut best: Option<(f32, f32)> = None;
    let mut best_d2 = f32::INFINITY;
    for w in walkers.iter().filter(|w| w.alive) {
        let (wx, wy) = w.aabb(ph).center();
        let (dx, dy) = (wx - cx, wy - cy);
        let d2 = dx * dx + dy * dy;
        if d2 < best_d2 {
            best_d2 = d2;
            best = Some((dx, dy));
        }
    }
    if let Some((dx, dy)) = best {
        out[81] = (dx / ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0);
        out[82] = (dy / ENEMY_VIEW_DISTANCE).clamp(-1.0, 1.0);
    }

    // ---- ground beside each edge of the feet ---------------------------
    // Measured from the edge the body would walk off (its right edge going right, its left
    // edge going left), so each probe answers "if I commit to this side, do I leave the
    // ground here?". The row is the one just under the feet, which is the ground itself.
    let foot_row = (player.y + player.h + 0.05).floor() as i32;
    out[83] = gap_beside(level, (player.x + player.w).floor() as i32, 1, foot_row);
    out[94] = gap_beside(level, player.x.floor() as i32, -1, foot_row);

    // ---- grounded, velocity, can-jump, goal direction, bias -------------
    out[84] = if player.grounded { 1.0 } else { 0.0 };
    out[85] = (player.vx / ph.sprint_speed).clamp(-1.0, 1.0);
    out[86] = (player.vy / ph.max_fall).clamp(-1.0, 1.0);
    out[87] = if player.can_jump() { 1.0 } else { 0.0 };

    // ---- v3: where the flag actually is --------------------------------
    let (gdx, gdy) = goal_delta(level, player);
    out[88] = (gdx / GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0);
    out[89] = (gdy / GOAL_VIEW_DISTANCE).clamp(-1.0, 1.0);
    out[90] = 1.0;

    // ---- v2: the movement abilities -------------------------------------
    out[91] = player.wall_dir() as f32;
    out[92] = if player.clinging() { 1.0 } else { 0.0 };
    let span = ph.sprint_speed - ph.run_speed;
    out[93] = if span > 0.0 {
        ((player.speed_cap() - ph.run_speed) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };

    debug_assert!(out.iter().all(|v| v.is_finite()));
    out
}

/// `1.0` when the ground runs out within [`GAP_LOOKAHEAD`] tiles of `foot_col` in `dir`
/// (`+1` right, `-1` left), `0.0` when solid ground is found first.
///
/// Out of bounds reads as solid (the level's side walls), so a body backed against the edge
/// of the world reads "ground continues" — which is true of the wall it is standing against.
fn gap_beside(level: &Level, foot_col: i32, dir: i32, foot_row: i32) -> f32 {
    for step in 1..=GAP_LOOKAHEAD {
        if level.is_solid(foot_col + dir * step, foot_row) {
            return 0.0;
        }
    }
    1.0
}

/// Vector from the body to the goal, in tiles; `(0, 0)` when there is nothing to aim at,
/// which is the honest reading of "no flag in this level" — a document loses its flag between
/// the editor's eraser and the next placement, and an agent observing one should see "nowhere
/// to go" rather than an index panic.
///
/// Signed in both axes, so the same input answers "the flag is up and to the right" and "the
/// flag is below me", and centre-to-centre because this one is a direction to steer by rather
/// than a distance to arrive at.
fn goal_delta(level: &Level, player: &PlayerState) -> (f32, f32) {
    let (cx, cy) = player.aabb().center();
    let mut best = (0.0f32, 0.0f32);
    let mut best_d2 = f32::INFINITY;
    for g in &level.goals {
        let (gx, gy) = g.center();
        let (dx, dy) = (gx - cx, gy - cy);
        let d2 = dx * dx + dy * dy;
        if d2 < best_d2 {
            best_d2 = d2;
            best = (dx, dy);
        }
    }
    best
}

/// Human-readable names for the observation vector, in order. Used by
/// `train-cli`'s `sensors` subcommand and by the snapshot test's failure output.
pub fn input_names() -> Vec<String> {
    let mut names = Vec::with_capacity(N_INPUTS);
    for dy in -PATCH_RADIUS..=PATCH_RADIUS {
        for dx in -PATCH_RADIUS..=PATCH_RADIUS {
            names.push(format!("tile[{dx:+},{dy:+}]"));
        }
    }
    names.push("enemy_dx".into());
    names.push("enemy_dy".into());
    names.push("gap_right".into());
    names.push("grounded".into());
    names.push("vx".into());
    names.push("vy".into());
    names.push("can_jump".into());
    names.push("goal_dx".into());
    names.push("goal_dy".into());
    names.push("bias".into());
    names.push("wall_dir".into());
    names.push("clinging".into());
    names.push("sprint_charge".into());
    names.push("gap_left".into());
    debug_assert_eq!(names.len(), N_INPUTS);
    names
}

/// One row of the action table: name, horizontal input, jump held, sprint asked for.
pub type ActionRow = (&'static str, f32, bool, bool);

/// Convenience for tools that want the action table next to the input table.
pub fn action_summary() -> Vec<ActionRow> {
    AgentAction::ALL
        .iter()
        .map(|a| (a.name(), a.input_x(), a.wants_jump(), a.wants_sprint()))
        .collect()
}
