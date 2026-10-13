//! Observation vector builder (§7.2). **Versioned**: any change to the layout,
//! the encodings, or the action-repeat cadence is a `VERSION` bump, and every
//! champion file records it (§8).
//!
//! `N_INPUTS = 90`. The patch is deliberately 9×9 rather than 13×13: Togelius
//! 2009 showed small controllers beating large ones, and input size multiplies
//! the search space. Widening the patch is the documented upgrade path, gated on
//! evolution visibly plateauing on levels that should be solvable.

use crate::level::{Level, Tile};
use crate::physics::{AgentAction, Physics, PlayerState, Walker};

/// Sensor layout version. Bump on *any* change below (§8).
pub const VERSION: u32 = 1;

/// Tiles per side of the local patch (`9×9 = 81` inputs).
pub const PATCH_RADIUS: i32 = 4;
/// Number of inputs in the observation vector.
pub const N_INPUTS: usize = 81 + 2 + 1 + 1 + 2 + 1 + 1 + 1;

/// Normalizer for the nearest-enemy vector (tiles).
pub const ENEMY_VIEW_DISTANCE: f32 = 4.0;
/// How far ahead the gap probe looks, in tiles.
pub const GAP_LOOKAHEAD: i32 = 3;

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
/// | 83  | 1  | gap ahead of the feet (1.0 = no ground within [`GAP_LOOKAHEAD`] tiles) |
/// | 84  | 1  | grounded |
/// | 85  | 2  | `vx`, `vy` normalised by `run_speed` / `max_fall` |
/// | 87  | 1  | can jump (grounded or coyote) |
/// | 88  | 1  | horizontal sign toward the goal (−1 / +1) |
/// | 89  | 1  | bias, constant 1.0 |
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

    // ---- gap ahead of the feet -----------------------------------------
    let facing = player.facing();
    let foot_row = (player.y + player.h + 0.05).floor() as i32;
    let foot_col = if facing > 0.0 {
        (player.x + player.w).floor() as i32
    } else {
        player.x.floor() as i32
    };
    let mut ground_found = false;
    for step in 1..=GAP_LOOKAHEAD {
        let col = if facing > 0.0 {
            foot_col + step
        } else {
            foot_col - step
        };
        if level.is_solid(col, foot_row) {
            ground_found = true;
            break;
        }
    }
    out[83] = if ground_found { 0.0 } else { 1.0 };

    // ---- grounded, velocity, can-jump, goal direction, bias -------------
    out[84] = if player.grounded { 1.0 } else { 0.0 };
    out[85] = (player.vx / ph.run_speed).clamp(-1.0, 1.0);
    out[86] = (player.vy / ph.max_fall).clamp(-1.0, 1.0);
    out[87] = if player.can_jump() { 1.0 } else { 0.0 };
    out[88] = goal_direction_sign(level, player);
    out[89] = 1.0;

    debug_assert!(out.iter().all(|v| v.is_finite()));
    out
}

/// `-1` or `+1` toward the nearest goal; `+1` when there is nothing to aim at.
fn goal_direction_sign(level: &Level, player: &PlayerState) -> f32 {
    let (cx, _) = player.aabb().center();
    let mut best = f32::INFINITY;
    for g in &level.goals {
        let d = g.x - cx;
        if d.abs() < best.abs() {
            best = d;
        }
    }
    if best < 0.0 {
        -1.0
    } else {
        1.0
    }
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
    names.push("gap_ahead".into());
    names.push("grounded".into());
    names.push("vx".into());
    names.push("vy".into());
    names.push("can_jump".into());
    names.push("goal_dir".into());
    names.push("bias".into());
    debug_assert_eq!(names.len(), N_INPUTS);
    names
}

/// Convenience for tools that want the action table next to the input table.
pub fn action_summary() -> Vec<(&'static str, f32, bool)> {
    AgentAction::ALL
        .iter()
        .map(|a| (a.name(), a.input_x(), a.wants_jump()))
        .collect()
}
