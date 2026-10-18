//! **The level tells the trainer what brain it needs.**
//!
//! Two of the search's defaults are wrong on a level whose answer is a *schedule*. The
//! observation has carried a clock and event ETAs since sensors v13, but the shipped net is
//! memoryless (`feedback 0`): a feedforward policy reads the clock as a number and cannot use it
//! to count, wait, or commit. And a level with patrols draws a different phase every generation
//! (finding 14), so ranking genomes on the mean of their trials lets a lucky phase outrank a
//! robust policy; `rank_pessimism` is the knob that reads the worst of them instead.
//!
//! The operator should not have to remember either fact. A level's loaders already classify every
//! mechanic it contains, so a run can ask the level: turrets, moving decks, lava vents, cracked
//! ground and flyers all have a period, and a policy that has to act inside one needs state; a
//! patrol makes an episode stochastic, and the ranking should be pessimistic about it. This module
//! is that question, answered conservatively:
//!
//! * it only *raises* memory — a run already given a wider feedback vector keeps it;
//! * it never overrides an explicit flag (the caller passes those in as `locked`);
//! * a level with none of the timed mechanics stays at the shipped reflex defaults, so every
//!   earlier measurement's configuration is still reachable and the pack's easy levels do not
//!   pay for capacity they do not use;
//! * the reason is returned as text, so the run header can say *why* it changed shape.

use sim::Level;

use crate::ga::GaConfig;

/// The memory width a timed level gets when the operator has not chosen one.
///
/// Sixteen is the shipped hidden width: the whole first hidden layer fed back, the largest
/// memory the default topology can carry without widening it. The review measured width 8 as
/// free (19 ms/gen); 16 is the smallest width that also gives a counter enough bits to hold a
/// turret cycle, and it is the ceiling the plan names for the default net.
pub const TIMED_FEEDBACK: usize = 16;

/// How pessimistic a stochastic level's ranking is by default.
///
/// Half the gap between a genome's mean and its worst trial. It is not the extreme: a run that
/// ranks purely on the worst phase is choosing for one patrol, which is the same mistake as
/// ranking on the mean. The measured arm that motivates the knob is in `AI_REVIEW.md` §2.2.
pub const PATROL_PESSIMISM: f32 = 0.5;

/// What a level's own mechanics ask the search to be.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AutoShape {
    /// The feedback width the level needs, or 0 for the shipped memoryless net.
    pub feedback: usize,
    /// Whether the level is stochastic enough to rank pessimistically.
    pub pessimism: bool,
    /// The mechanics that caused the answer, in the order they were found. Empty for a level
    /// that needs nothing.
    pub reasons: Vec<&'static str>,
}

impl AutoShape {
    /// The one-line explanation the run header prints.
    pub fn line(&self) -> String {
        if self.feedback == 0 && !self.pessimism {
            return "none (no timed mechanic, no patrol)".to_string();
        }
        let mut what = Vec::new();
        if self.feedback > 0 {
            what.push(format!("feedback {}", self.feedback));
        }
        if self.pessimism {
            what.push(format!("rank pessimism {PATROL_PESSIMISM:.2}"));
        }
        format!(
            "{} — {}",
            what.join(", "),
            if self.reasons.is_empty() {
                "the level asks for it".to_string()
            } else {
                self.reasons.join(", ")
            }
        )
    }
}

/// Read `level`'s mechanics and answer what shape it asks for.
pub fn scan(level: &Level) -> AutoShape {
    let mut out = AutoShape::default();
    let mut note = |what: &'static str| {
        if !out.reasons.contains(&what) {
            out.reasons.push(what);
        }
    };

    // **Timed mechanics.** Each of these has a period the policy has to act inside: a turret's
    // cooldown, a deck's travel, a vent's throw, a fuse, a flyer's beat. A reflex can react to
    // where the thing is now; only state can plan around when it will be somewhere else.
    if !level.turrets.is_empty() {
        note("turrets");
        out.feedback = TIMED_FEEDBACK;
    }
    if !level.platforms.is_empty() {
        note("moving platforms");
        out.feedback = TIMED_FEEDBACK;
    }
    if !level.lava_vents().is_empty() {
        note("lava vents");
        out.feedback = TIMED_FEEDBACK;
    }
    if !level.flyers.is_empty() {
        note("flyers");
        out.feedback = TIMED_FEEDBACK;
    }
    if has_cracked_ground(level) {
        note("cracked ground");
        out.feedback = TIMED_FEEDBACK;
    }

    // **Stochastic episodes.** Walkers, chasers and flyers all draw a patrol phase per
    // generation; a level with any of them is the level the pessimistic ranking is for.
    if !level.walkers.is_empty() || !level.chasers.is_empty() || !level.flyers.is_empty() {
        note("patrols");
        out.pessimism = true;
    }

    out
}

/// The same answer over a suite: the union of every level's needs.
///
/// A suite is judged on all of its levels at once, so the shape has to serve the hardest one.
/// The reasons stay the union too, which is what makes a suite header readable: it names every
/// mechanic that changed the shape anywhere in the run.
pub fn scan_all(levels: &[Level]) -> AutoShape {
    let mut out = AutoShape::default();
    for level in levels {
        let one = scan(level);
        out.feedback = out.feedback.max(one.feedback);
        out.pessimism |= one.pessimism;
        for reason in one.reasons {
            if !out.reasons.contains(&reason) {
                out.reasons.push(reason);
            }
        }
    }
    out
}

/// Apply the scan to a live config, respecting explicit operator flags.
///
/// Returns the shape that was applied, or `None` when both answers were already chosen by the
/// operator (nothing changed and there is nothing to report). `feedback_locked` and
/// `pessimism_locked` are the caller's statement that the corresponding flag was passed.
///
/// Memory is only ever **raised**; a run already carrying a wider feedback vector keeps it. The
/// pessimistic rank is set only when the level asks for it and the operator has not answered;
/// a level with no patrol leaves the operator's value (including a non-zero one) alone.
pub fn apply(
    cfg: &mut GaConfig,
    levels: &[Level],
    feedback_locked: bool,
    pessimism_locked: bool,
) -> Option<AutoShape> {
    let shape = scan_all(levels);
    let mut applied = AutoShape::default();
    if !feedback_locked && shape.feedback > cfg.shape.feedback {
        cfg.shape.feedback = shape.feedback;
        applied.feedback = shape.feedback;
    }
    if !pessimism_locked && shape.pessimism {
        cfg.rank_pessimism = cfg.rank_pessimism.max(PATROL_PESSIMISM);
        applied.pessimism = true;
    }
    applied.reasons = shape.reasons;
    if applied.feedback == 0 && !applied.pessimism {
        return None;
    }
    Some(applied)
}

/// Does the level contain a brittle tile anywhere?
///
/// A grid walk rather than a precomputed flag: the scan runs once at run start, a level is at
/// most a few thousand tiles, and the editor mutates the grid between runs — a cached answer
/// would be one more thing that can go stale when the level moves underneath it.
fn has_cracked_ground(level: &Level) -> bool {
    (0..level.height)
        .any(|y| (0..level.width).any(|x| level.tile(x, y).is_some_and(|t| t.is_cracked())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(slug: &str) -> Level {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../levels")
            .join(format!("{slug}.json"));
        Level::from_json_file(&path).unwrap_or_else(|e| panic!("shipped level {slug}: {e}"))
    }

    #[test]
    fn a_timed_level_asks_for_memory_and_a_patrol_level_asks_for_pessimism() {
        let elements = scan(&load("l11-the-elements"));
        assert_eq!(elements.feedback, TIMED_FEEDBACK);
        assert!(elements.pessimism);
        assert!(elements.reasons.contains(&"turrets"));
        assert!(elements.reasons.contains(&"moving platforms"));
        assert!(elements.reasons.contains(&"patrols"));

        let gauntlet = scan(&load("l3-gauntlet"));
        assert_eq!(gauntlet.feedback, 0, "a walker corridor needs no memory");
        assert!(gauntlet.pessimism, "but it does draw patrol phases");

        let flat = scan(&load("l1-flat-run"));
        assert_eq!(
            flat,
            AutoShape::default(),
            "a flat runway is still the shipped reflex"
        );
        assert!(flat.line().contains("none"));
    }

    #[test]
    fn an_explicit_answer_is_not_overridden_and_memory_is_only_raised() {
        let levels = [load("l11-the-elements")];
        let mut cfg = GaConfig::default();
        assert_eq!(cfg.shape.feedback, 0);

        let applied = apply(&mut cfg, &levels, false, false).expect("the level changes the shape");
        assert_eq!(cfg.shape.feedback, TIMED_FEEDBACK);
        assert_eq!(cfg.rank_pessimism, PATROL_PESSIMISM);
        assert!(applied.line().contains("feedback 16"));

        // A wider feedback vector the operator chose survives, and with the pessimism flag
        // locked there is nothing left to change: the scan reports no edit.
        let mut wider = GaConfig::default();
        wider.shape.feedback = 32;
        assert!(apply(&mut wider, &levels, false, true).is_none());
        assert_eq!(wider.shape.feedback, 32);
        assert_eq!(wider.rank_pessimism, 0.0);

        // Both answered explicitly: nothing to do, nothing to report.
        let mut chosen = GaConfig::default();
        assert!(apply(&mut chosen, &levels, true, true).is_none());
        assert_eq!(chosen.shape.feedback, 0);
        assert_eq!(chosen.rank_pessimism, 0.0);
    }
}
