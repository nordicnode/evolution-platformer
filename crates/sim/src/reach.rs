//! Level validation: the jump-reachability oracle (§9).
//!
//! Answers "is this level beatable at all?" in milliseconds, before any training
//! budget is spent. Deliberately uses the **real** [`PlayerState::tick`] stepper
//! and the real collision query, so the oracle can never disagree with the sim
//! about how far a jump goes: it only disagrees about whether some *particular*
//! jump is expressible in the six-action space, which is exactly the question.
//!
//! Live in `sim` rather than in the editor because it needs the physics bible and
//! the tile grid; the editor is just the first consumer.

use crate::level::{Level, Rect};
use crate::physics::{AgentAction, Physics, PlayerState, PHYSICS};

/// How long a single canonical jump trajectory may run.
pub const MAX_TRAJECTORY_TICKS: u32 = 240;

/// Sub-tile take-off offsets tried at every column.
///
/// An agent can stand anywhere, not just on a lattice: with a 0.6-wide body its
/// left edge ranges over `col + [0.0, 0.4]`. The L3 spike patch is only clearable
/// from a narrow band inside that range, so sampling a single offset made the
/// oracle call a beatable level unbeatable. Cost is linear in this list.
pub const TAKEOFF_OFFSETS: [f32; 5] = [0.0, 0.1, 0.2, 0.3, 0.4];

/// A maximal horizontal run of solid tiles with open space above it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Platform {
    pub id: usize,
    /// Inclusive tile column range of the surface.
    pub x0: i32,
    pub x1: i32,
    /// Tile row of the solid surface. Its top edge is at world `y = surface_row`.
    pub surface_row: i32,
}

impl Platform {
    pub fn center_x(&self) -> f32 {
        (self.x0 + self.x1) as f32 * 0.5 + 0.5
    }

    /// World-space surface box, for drawing the validation overlay.
    pub fn surface_rect(&self) -> Rect {
        Rect::new(
            self.x0 as f32,
            self.surface_row as f32,
            (self.x1 - self.x0 + 1) as f32,
            1.0,
        )
    }

    pub fn contains_col(&self, col: i32) -> bool {
        col >= self.x0 && col <= self.x1
    }
}

/// How long the jump button is held during a canonical trajectory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpHold {
    /// No jump at all: walk or run off the edge.
    None,
    /// Released as early as the sim allows (`min_jump_hold`).
    Short,
    /// Held through the whole ascent.
    Full,
}

impl JumpHold {
    pub const JUMPING: [JumpHold; 2] = [JumpHold::Short, JumpHold::Full];

    pub fn name(self) -> &'static str {
        match self {
            JumpHold::None => "walk-off",
            JumpHold::Short => "short-jump",
            JumpHold::Full => "full-jump",
        }
    }
}

/// Take-off direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeoffDir {
    Left,
    Still,
    Right,
}

impl TakeoffDir {
    pub const ALL: [TakeoffDir; 3] = [TakeoffDir::Left, TakeoffDir::Still, TakeoffDir::Right];

    pub fn sign(self) -> f32 {
        match self {
            TakeoffDir::Left => -1.0,
            TakeoffDir::Still => 0.0,
            TakeoffDir::Right => 1.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TakeoffDir::Left => "left",
            TakeoffDir::Still => "still",
            TakeoffDir::Right => "right",
        }
    }
}

/// The outcome of one canonical jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trajectory {
    /// Platform the body came to rest on, if any.
    pub landed_on: Option<usize>,
    pub touched_goal: bool,
    pub hit_spike: bool,
    pub fell_out: bool,
    pub ticks: u32,
}

/// A single canonical jump at a single spot: enough to name the culprit in the
/// editor when a level is unreachable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CulpritJump {
    pub from: usize,
    pub takeoff_x: i32,
    /// Sub-tile offset of the body's left edge within the take-off column.
    pub takeoff_offset: f32,
    pub dir: TakeoffDir,
    pub hold: JumpHold,
    /// Aimed-at platform, when one exists to the goal-ward side.
    pub to: Option<usize>,
}

/// Answer to "can this level be beaten in principle?".
#[derive(Clone, Debug)]
pub struct ReachReport {
    pub platforms: Vec<Platform>,
    /// Platform the player spawns on.
    pub start: Option<usize>,
    /// Platform index containing the goal, when the goal sits on one.
    pub goal_platform: Option<usize>,
    pub goal_reached: bool,
    /// Index-aligned with `platforms`: reachable from the spawn platform.
    pub reached: Vec<bool>,
    /// The stuck jump to highlight when `goal_reached` is false.
    pub culprit: Option<CulpritJump>,
    pub diagnosis: Option<String>,
    /// Tiles covered by the reachable set, for a "could not get past here" marker.
    pub frontier_x: Option<i32>,
}

impl ReachReport {
    pub fn reachable_platforms(&self) -> usize {
        self.reached.iter().filter(|r| **r).count()
    }
}

/// Extract platforms from the tile grid (§9.1).
///
/// Ground counts as one long platform. Spikes are not platforms: they are lethal
/// and non-solid, so you can neither stand on one nor be carried by one.
pub fn extract_platforms(level: &Level) -> Vec<Platform> {
    let mut out = Vec::new();
    for row in 0..level.height {
        let mut x = 0;
        while x < level.width {
            let is_surface = level.is_solid(x, row) && !level.is_solid(x, row - 1);
            if !is_surface {
                x += 1;
                continue;
            }
            let start = x;
            let mut end = x;
            while end + 1 < level.width
                && level.is_solid(end + 1, row)
                && !level.is_solid(end + 1, row - 1)
            {
                end += 1;
            }
            out.push(Platform {
                id: out.len(),
                x0: start,
                x1: end,
                surface_row: row,
            });
            x = end + 1;
        }
    }
    out
}

fn platform_at(platforms: &[Platform], surface_row: i32, col: i32) -> Option<usize> {
    platforms
        .iter()
        .find(|p| p.surface_row == surface_row && p.contains_col(col))
        .map(|p| p.id)
}

/// The action a canonical trajectory issues on tick `t`.
fn trajectory_action(dir: TakeoffDir, hold: JumpHold, hovering: bool, t: u32, ph: &Physics) -> AgentAction {
    let holding_jump = match hold {
        JumpHold::None => false,
        // "Held through the ascent" is expressed as `hovering`, which the stepper
        // clears once vertical velocity turns non-negative.
        JumpHold::Full => hovering,
        JumpHold::Short => t < ph.min_jump_hold,
    };
    match dir {
        // The action set has no "jump left": a leftward jump is a neutral jump
        // that carries its pre-jump momentum, plus air acceleration afterwards.
        TakeoffDir::Left => {
            if holding_jump {
                AgentAction::JumpInPlace
            } else {
                AgentAction::Retreat
            }
        }
        TakeoffDir::Still => {
            if holding_jump {
                AgentAction::JumpInPlace
            } else {
                AgentAction::Walk
            }
        }
        TakeoffDir::Right => {
            if holding_jump {
                AgentAction::RunJump
            } else {
                AgentAction::Run
            }
        }
    }
}

/// Simulate one canonical jump with the real physics stepper (§9.3).
///
/// * `takeoff_x` — tile column to launch from.
/// * `surface_row` — tile row the body stands on.
/// * `run_up` — whether the body may carry full run speed into the take-off.
pub fn simulate_jump(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: i32,
    surface_row: i32,
    dir: TakeoffDir,
    hold: JumpHold,
    run_up: bool,
) -> Trajectory {
    simulate_jump_at(
        level,
        platforms,
        takeoff_x as f32 + (1.0 - PHYSICS.player_w) * 0.5,
        surface_row,
        dir,
        hold,
        run_up,
    )
}

/// As [`simulate_jump`], but with the body's left edge placed at an exact x, so
/// callers can sweep the sub-tile positions [`TAKEOFF_OFFSETS`] covers.
pub fn simulate_jump_at(
    level: &Level,
    platforms: &[Platform],
    takeoff_x: f32,
    surface_row: i32,
    dir: TakeoffDir,
    hold: JumpHold,
    run_up: bool,
) -> Trajectory {
    let ph = &PHYSICS;
    let mut player = PlayerState::spawn(level, ph);
    player.w = ph.player_w;
    player.h = ph.player_h;
    player.x = takeoff_x;
    player.y = surface_row as f32 - ph.player_h;
    player.vx = if run_up { dir.sign() * ph.run_speed } else { 0.0 };
    player.vy = 0.0;
    // The body is authored standing on `surface_row`, which is by construction a
    // surface tile, so a jump is legal on tick zero.
    player.grounded = true;

    let mut hover = hold != JumpHold::None;
    // A "landing" only counts once the body has actually left the ground, so
    // walking off an edge is not misreported as landing on the take-off platform
    // during the ticks it takes to clear the edge.
    let mut airborne = false;
    for t in 0..MAX_TRAJECTORY_TICKS {
        let action = trajectory_action(dir, hold, hover, t, ph);
        let contact = player.tick(level, ph, action);
        if player.vy >= 0.0 {
            hover = false;
        }
        if !player.grounded {
            airborne = true;
        }
        if contact.touched_spike {
            return Trajectory {
                landed_on: None,
                touched_goal: false,
                hit_spike: true,
                fell_out: false,
                ticks: t + 1,
            };
        }
        if level.goals.iter().any(|g| player.aabb().overlaps(g)) {
            return Trajectory {
                landed_on: None,
                touched_goal: true,
                hit_spike: false,
                fell_out: false,
                ticks: t + 1,
            };
        }
        if player.y > level.height as f32 {
            return Trajectory {
                landed_on: None,
                touched_goal: false,
                hit_spike: false,
                fell_out: true,
                ticks: t + 1,
            };
        }
        if contact.landed && airborne {
            let row = (player.y + player.h).floor() as i32;
            let col = (player.x + player.w * 0.5).floor() as i32;
            let landed_on = platform_at(platforms, row, col);
            // Landing back on the take-off platform with no forward progress is
            // not an edge; it is reported as "landed here" and filtered by BFS.
            return Trajectory {
                landed_on,
                touched_goal: false,
                hit_spike: false,
                fell_out: false,
                ticks: t + 1,
            };
        }
    }
    Trajectory {
        landed_on: None,
        touched_goal: false,
        hit_spike: false,
        fell_out: false,
        ticks: MAX_TRAJECTORY_TICKS,
    }
}

/// Every canonical jump out of `platform` (§9.3).
fn edges_from(level: &Level, platforms: &[Platform], p: &Platform) -> Vec<(Trajectory, CulpritJump)> {
    let mut out = Vec::new();
    let cols = p.x0..=p.x1;
    for col in cols {
      for &offset in &TAKEOFF_OFFSETS {
        let x = col as f32 + offset;
        for dir in TakeoffDir::ALL {
            // A jump needs runway *behind* the take-off, not beyond it: the body
            // is already at full speed when it reaches the ledge it launches from.
            // One tile is enough (accel 0.01 for 10 ticks covers ~0.5 tiles).
            // Getting this backwards silently made every pit uncrossable, because
            // the platform's outermost column -- the only one you can clear a gap
            // from -- is exactly the column with no platform to its outside.
            let has_runway = match dir {
                TakeoffDir::Right => col > p.x0,
                TakeoffDir::Left => col < p.x1,
                TakeoffDir::Still => true,
            };
            let holds: &[JumpHold] = if dir == TakeoffDir::Still {
                &[JumpHold::Short, JumpHold::Full]
            } else {
                &JumpHold::JUMPING
            };
            for hold in holds {
                let traj = simulate_jump_at(level, platforms, x, p.surface_row, dir, *hold, has_runway);
                out.push((traj, CulpritJump {
                    from: p.id,
                    takeoff_x: col,
                    takeoff_offset: offset,
                    dir,
                    hold: *hold,
                    to: traj.landed_on.filter(|q| *q != p.id),
                }));
            }
            // Walking off the edge, only from the ends.
            let at_edge = col == p.x0 || col == p.x1;
            if at_edge && dir != TakeoffDir::Still {
                let traj = simulate_jump_at(level, platforms, x, p.surface_row, dir, JumpHold::None, has_runway);
                out.push((traj, CulpritJump {
                    from: p.id,
                    takeoff_x: col,
                    takeoff_offset: offset,
                    dir,
                    hold: JumpHold::None,
                    to: traj.landed_on.filter(|q| *q != p.id),
                }));
            }
        }
      }
    }
    out
}

/// Run the oracle: platform graph + BFS from the spawn platform to the goal (§9.4).
pub fn report(level: &Level) -> ReachReport {
    let platforms = extract_platforms(level);

    let spawn_col = (level.spawn.x + level.spawn.w * 0.5).floor() as i32;
    let spawn_surface_row = (level.spawn.y + level.spawn.h).floor() as i32;
    let start = platform_at(&platforms, spawn_surface_row, spawn_col);

    let goal_rect: Rect = level.goals[0];
    let goal_col = (goal_rect.x + goal_rect.w * 0.5).floor() as i32;
    let goal_platform = platforms
        .iter()
        .find(|p| p.contains_col(goal_col) && (p.surface_row as f32) >= goal_rect.y)
        .map(|p| p.id);

    let mut reached = vec![false; platforms.len()];
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); platforms.len()];
    let mut touched_goal_from: Vec<bool> = vec![false; platforms.len()];
    let mut all_edges: Vec<Vec<CulpritJump>> = vec![Vec::new(); platforms.len()];

    for p in &platforms {
        for (traj, culprit) in edges_from(level, &platforms, p) {
            all_edges[p.id].push(culprit);
            if traj.touched_goal {
                touched_goal_from[p.id] = true;
            }
            if let Some(q) = traj.landed_on {
                if q != p.id && !adjacency[p.id].contains(&q) {
                    adjacency[p.id].push(q);
                }
            }
        }
    }

    let mut goal_reached = false;
    if let Some(start) = start {
        // BFS. Deterministic: neighbours are visited in insertion order.
        let mut queue = std::collections::VecDeque::new();
        reached[start] = true;
        queue.push_back(start);
        while let Some(id) = queue.pop_front() {
            if touched_goal_from[id] {
                goal_reached = true;
            }
            if Some(id) == goal_platform && platforms[id].contains_col(goal_col) {
                goal_reached = true;
            }
            for &n in &adjacency[id] {
                if !reached[n] {
                    reached[n] = true;
                    queue.push_back(n);
                }
            }
        }
    }

    let (culprit, diagnosis, frontier_x) = if goal_reached {
        (None, None, None)
    } else {
        diagnose(&platforms, &reached, &all_edges, goal_col)
    };

    ReachReport {
        platforms,
        start,
        goal_platform,
        goal_reached,
        reached,
        culprit,
        diagnosis,
        frontier_x,
    }
}

/// Name the first jump that cannot be made: from the reachable platform nearest
/// the goal, describe why nothing further is possible (§9.4).
fn diagnose(
    platforms: &[Platform],
    reached: &[bool],
    edges: &[Vec<CulpritJump>],
    goal_col: i32,
) -> (Option<CulpritJump>, Option<String>, Option<i32>) {
    if platforms.is_empty() {
        return (None, Some("level has no solid ground at all".into()), None);
    }

    // The reachable platform whose surface gets closest to the goal.
    let mut best: Option<&Platform> = None;
    for p in platforms.iter().filter(|p| reached[p.id]) {
        let better = match best {
            None => true,
            Some(b) => (goal_col - p.x1).abs() < (goal_col - b.x1).abs(),
        };
        if better {
            best = Some(p);
        }
    }
    let Some(from) = best else {
        return (
            None,
            Some("the spawn tile is not on any solid platform".into()),
            None,
        );
    };

    // Prefer a failing edge that aimed forward; otherwise report the nearest
    // forward target on its own.
    let forward: Vec<&CulpritJump> = edges[from.id]
        .iter()
        .filter(|c| match c.dir {
            TakeoffDir::Left => goal_col < from.x0,
            TakeoffDir::Right => goal_col > from.x1,
            TakeoffDir::Still => false,
        })
        .collect();

    let target = platforms
        .iter()
        .filter(|p| p.surface_row <= from.surface_row && p.x0 > from.x1)
        .min_by_key(|p| p.x0 - from.x1);

    let message = match target {
        Some(t) => {
            let gap = (t.x0 - from.x1 - 1) as f32;
            let rise = (from.surface_row - t.surface_row) as f32;
            let ph = &PHYSICS;
            if rise > ph.discrete_max_jump_height() {
                format!(
                    "wall too tall: the next surface rises {rise:.0} tiles, but a full jump reaches only {:.2} tiles",
                    ph.discrete_max_jump_height()
                )
            } else if gap > ph.max_jump_distance() {
                format!(
                    "gap too wide: {gap:.0} empty tiles to the next surface, but a full-speed jump covers only {:.2} tiles",
                    ph.max_jump_distance()
                )
            } else {
                format!(
                    "no canonical jump from platform {} reaches the next surface: {gap:.0}-tile gap, {rise:.0}-tile rise, {} take-off points tried",
                    from.id,
                    edges[from.id].len()
                )
            }
        }
        None => "no further surface exists to the goal-ward side of the reachable set".into(),
    };

    let culprit = forward
        .first()
        .map(|c| CulpritJump { ..**c })
        .or_else(|| edges[from.id].last().map(|c| CulpritJump { ..*c }));
    (culprit, Some(message), Some(from.x1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixtures are ASCII art rather than inline JSON so a run of `#` never has
    /// to fight Rust's raw-string delimiters, and so the shape is scannable.
    fn level(name: &str, rows: &[&str], spawn: (i32, i32), entities: &[(&str, i32, i32)]) -> Level {
        let width = rows[0].len();
        assert!(
            rows.iter().all(|r| r.len() == width),
            "ragged fixture rows in {name}"
        );
        let tiles = rows
            .iter()
            .map(|r| format!("    \"{r}\""))
            .collect::<Vec<_>>()
            .join(",\n");
        let ents = entities
            .iter()
            .map(|(k, x, y)| format!("{{\"type\":\"{k}\",\"x\":{x},\"y\":{y}}}"))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            "{{\"format\":\"evo-platformer-level\",\"level_format_version\":1,\"name\":\"{name}\",
               \"width\":{width},\"height\":{},\"spawn\":{{\"x\":{},\"y\":{}}},
               \"tiles\":[\n{tiles}\n],\"entities\":[{ents}]}}",
            rows.len(),
            spawn.0,
            spawn.1
        );
        Level::from_json_str(&src).expect("fixture parses")
    }

    // Ground row 6, a two-tile pillar at x=6..8, shelf on top at row 4.
    const LADDER: &[&str] = &[
        "..........",
        "..........",
        "..........",
        "..........",
        "......###.",
        "......###.",
        "##########",
        "##########",
    ];

    // Ground row 6 at x 0..=5 and x 11..=19: a five-tile pit, wider than d = 4.
    const GAP: &[&str] = &[
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "....................",
        "######.....#########",
        "######.....#########",
    ];

    // Ground row 6 at x 0..=9, then a five-tile wall topped by a shelf at row 1.
    const WALL: &[&str] = &[
        "................",
        "..........######",
        "..........######",
        "..........######",
        "..........######",
        "..........######",
        "##########......",
        "##########......",
    ];

    #[test]
    fn platform_extraction_splits_only_at_real_breaks() {
        let lv = level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]);
        let shape: Vec<(i32, i32, i32)> = extract_platforms(&lv)
            .iter()
            .map(|p| (p.x0, p.x1, p.surface_row))
            .collect();
        assert_eq!(shape, vec![(6, 8, 4), (0, 5, 6), (9, 9, 6)]);
    }

    #[test]
    fn a_held_jump_reaches_a_two_tile_shelf_that_a_tap_jump_cannot() {
        let lv = level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]);
        let platforms = extract_platforms(&lv);
        // Take off far enough back that the body has cleared the pillar's rows by
        // the time it arrives: the shelf is 2 tiles up and the body is 0.9 tall,
        // so a full jump's 2.5-tile apex leaves well under a tile of clearance.
        let full = simulate_jump(&lv, &platforms, 3, 6, TakeoffDir::Right, JumpHold::Full, true);
        let short = simulate_jump(&lv, &platforms, 3, 6, TakeoffDir::Right, JumpHold::Short, true);
        assert_eq!(
            full.landed_on,
            Some(0),
            "a held jump should land on the shelf: {full:?}"
        );
        // The tap jump runs into the pillar's flank and drops back to the ground.
        // Note the tap jump is not *shorter* in ticks: a blocked trajectory is
        // truncated horizontally, not vertically, so airtime is not monotonic in
        // jump height. Only the landing spot is meaningful here.
        assert_eq!(
            short.landed_on,
            Some(1),
            "a tap jump should be blocked and fall back to the ground: {short:?}"
        );
        assert!(full.ticks <= MAX_TRAJECTORY_TICKS);
    }

    #[test]
    fn a_walk_off_the_edge_is_not_reported_as_a_landing_on_the_same_platform() {
        let lv = level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]);
        let platforms = extract_platforms(&lv);
        let walk = simulate_jump(&lv, &platforms, 5, 6, TakeoffDir::Right, JumpHold::None, true);
        assert_ne!(
            walk.landed_on,
            Some(1),
            "walking off must not count as landing back on the take-off platform: {walk:?}"
        );
    }

    #[test]
    fn the_ladder_fixture_is_beatable() {
        let report = report(&level("ladder", LADDER, (1, 5), &[("goal", 7, 3)]));
        assert!(report.goal_reached, "{:?}", report.diagnosis);
        assert_eq!(report.start, Some(1), "spawn should sit on the ground platform");
        assert_eq!(report.goal_platform, Some(0));
        assert!(report.culprit.is_none());
    }

    #[test]
    fn a_five_tile_gap_is_reported_as_too_wide() {
        let report = report(&level("gap", GAP, (1, 5), &[("goal", 17, 5)]));
        assert!(!report.goal_reached);
        let d = report.diagnosis.expect("a diagnosis");
        assert!(d.contains("gap too wide"), "unexpected diagnosis: {d}");
        assert!(report.culprit.is_some(), "the editor needs a jump to highlight");
        assert_eq!(report.frontier_x, Some(5));
    }

    #[test]
    fn a_five_tile_wall_is_reported_as_too_tall() {
        let report = report(&level("wall", WALL, (1, 5), &[("goal", 13, 1)]));
        assert!(!report.goal_reached);
        let d = report.diagnosis.expect("a diagnosis");
        assert!(d.contains("wall too tall"), "unexpected diagnosis: {d}");
        assert_eq!(report.frontier_x, Some(9));
    }

    #[test]
    fn an_empty_platform_graph_still_spawns() {
        // A flat world with the goal directly overhead: still reachable is false,
        // and the report must not panic or invent a culprit.
        let flat = [".....", "..G..", "#####"];
        let report = report(&level("tiny", &flat, (1, 1), &[]));
        assert!(report.goal_reached, "{:?}", report.diagnosis);
    }
}
