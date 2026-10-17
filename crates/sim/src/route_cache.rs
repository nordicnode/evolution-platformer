//! The route cache: what a level's terrain has already been asked, kept beside its file.
//!
//! A level's terrain is asked two expensive questions, and both are pure functions of the
//! terrain, the spawn and the flag:
//!
//! * the **route graph** ([`crate::reach::goal_routes`], the waypoint compass the sensors and
//!   the trainer's progress term read), and
//! * the **verdict** ([`crate::reach::report`], the answer the editor's overlay and
//!   `train-cli reach` print).
//!
//! The sweep behind each of them is every canonical move from every column of every stand —
//! milliseconds per level, paid again by every process that opens the level. So each is kept
//! beside the level's file, in a sidecar named after it (`l1-flat-run.json` →
//! `l1-flat-run.json.routes`), and adopted at load when it belongs to the level in hand.
//!
//! **The key is the content, not the file.** A sidecar is used only when the level's own
//! [`crate::Level::content_hash_string`] matches the hash recorded in it *and* both behaviour
//! versions match: [`crate::SIM_VERSION`] (physics, tile semantics, episode termination) and
//! [`crate::reach::MOVE_SET_VERSION`] (the take-off grid and the canonical scripts). That is
//! what makes the file self-invalidating rather than something to be remembered and cleaned:
//! editing a level and saving it moves the hash, so the next open recomputes and rewrites,
//! and a build that changes what the oracle answers refuses every cache it did not write.
//!
//! **Nothing here is allowed to fail a load.** A sidecar that is absent, unreadable,
//! malformed, truncated by a process that is still writing it, or written by another build is
//! a *miss*: the same code that wrote it recomputes in milliseconds. In particular this is a
//! derived artifact, so it is written in place and its errors are dropped on purpose — a
//! read-only levels directory costs a recomputation and must not cost a command.
//!
//! The one thing that is *not* derivable from the file is staleness of a document in memory,
//! which is handled by the level itself: a slot exists exactly while the document is the one
//! its file describes, and every writer of the grid drops it ([`crate::Level::forget_routes`],
//! [`crate::Level::burn_tile`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::level::Level;
use crate::reach::{self, CachedVerdict, GoalRoutes};
use crate::SIM_VERSION;

/// What the sidecar says it is. A file whose tag is not this one is another tool's JSON.
const FORMAT: &str = "evo-route-cache";

/// The shape of the sidecar. Bump when a field changes meaning or goes away: an older file is
/// then a miss rather than a misreading.
///
/// v2: a route graph's edges carry the take-off that makes them
/// ([`crate::reach::RouteEdge`]), so a document written by v1 has no plan for any of its edges and
/// is not the same graph. `evo::route_play` walks routes and would fail every hop of a stale file
/// with nothing to press, which is the one failure mode a cache must never invent: a miss costs a
/// recomputation, and a misread costs a run.
const VERSION: u32 = 2;

/// The suffix the sidecar takes, appended to the level file's **whole** name.
///
/// It must not leave the level's name looking like a level: every tool in this workspace finds
/// levels by the `.json` extension (`train-cli`'s `list_levels`, the app's shelf), so a cache
/// called `l1-flat-run.routes.json` is offered up as a level and fails to parse as one — which
/// is not a hypothetical, it is what the first version of this module did and what the shelf's
/// own test caught.
const SUFFIX: &str = ".routes";

/// Where a level's cache lives: beside the level file, named after all of it.
///
/// Beside rather than in one shared directory, because a level file is what a person moves,
/// copies and mails around, and a cache keyed on content has no meaning apart from the file it
/// was derived from — the two travel together or the cache is simply recomputed.
pub(crate) fn sidecar_path(level_path: &Path) -> PathBuf {
    let mut name = level_path.file_name().unwrap_or_default().to_os_string();
    name.push(SUFFIX);
    level_path.with_file_name(name)
}

/// The cache a level loaded from a file carries: where its sidecar is, and the verdict that
/// file gave — or that a later ask produced.
///
/// It is a slot on the level rather than an argument to a write because the two halves are
/// produced by different calls: `Level::routes` builds the graph, `reach::report_with` answers
/// the verdict, and **whichever comes second must not drop the first one's half** when it
/// rewrites the file. Every writer writes everything the level knows.
#[derive(Clone, Debug)]
pub(crate) struct Slot {
    path: PathBuf,
    /// The verdict, once one is known. The same `Arc<OnceLock<…>>` shape
    /// [`Level`]'s route table uses, for the same reason: a verdict is as pure a function of
    /// the terrain as the graph is, so every clone of one terrain shares the one answer, and
    /// the first writer of the slot wins.
    verdict: Arc<OnceLock<Arc<CachedVerdict>>>,
}

impl Slot {
    pub(crate) fn new(path: PathBuf) -> Slot {
        Slot {
            path,
            verdict: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// The verdict this level's file has, if one has been adopted or computed.
    pub(crate) fn verdict(&self) -> Option<&Arc<CachedVerdict>> {
        self.verdict.get()
    }

    /// Keep `verdict` as this level's answer. The first one wins.
    pub(crate) fn remember(&self, verdict: CachedVerdict) {
        let _ = self.verdict.set(Arc::new(verdict));
    }
}

/// The sidecar's own shape, in write order.
///
/// Serializing this rather than hand-building JSON is what keeps the writer and the reader
/// from ever disagreeing about keys: they are the same struct. `RawLevel` follows the same
/// rule for the level file itself.
#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    format: String,
    version: u32,
    /// The sim's behaviour version — physics, tile semantics, episode termination.
    sim_version: String,
    /// The oracle's move set: the take-off grid and the canonical scripts.
    move_set: u32,
    /// The level's name, for a person reading the file. Not a check of its own: the hash below
    /// covers every field the answers were computed from, this one included.
    level: String,
    /// **The key.** [`Level::content_hash_string`] of the terrain these answers are for: the
    /// grid, the spawn and the flag — everything the graph and the verdict are functions of.
    level_hash: String,
    /// The route graph, when the file has one.
    ///
    /// Absent for a level whose verdict was asked for before its route table was ever needed
    /// (`train-cli reach` does exactly that), and filled in by that level's next writer.
    routes: Option<GoalRoutes>,
    /// The oracle's verdict, when the file has one, with the abilities it was computed for.
    verdict: Option<CachedVerdict>,
}

/// The answers beside `level_path` that belong to **this** level, if any do.
///
/// `None` is every way a file can fail to be this level's answer — absent, unreadable,
/// malformed, another format or version, another move set, another level — and all of them are
/// a miss rather than an error, because a recomputation is milliseconds and a cache is not
/// worth failing a load over.
pub(crate) fn read(
    level: &Level,
    path: &Path,
) -> Option<(Option<GoalRoutes>, Option<CachedVerdict>)> {
    let src = std::fs::read_to_string(path).ok()?;
    let payload: Payload = serde_json::from_str(&src).ok()?;
    if payload.format != FORMAT || payload.version != VERSION {
        return None;
    }
    if payload.sim_version != SIM_VERSION || payload.move_set != reach::MOVE_SET_VERSION {
        return None;
    }
    // The hash is what makes staleness impossible to forget: a level that has been edited and
    // saved is a different level, and the file's answers are about the one it used to be.
    if payload.level_hash != level.content_hash_string() {
        return None;
    }
    Some((payload.routes, payload.verdict))
}

/// Write `level`'s cache, best effort, for the halves the caller has.
///
/// **Errors are dropped deliberately.** A cache that cannot be written — a read-only levels
/// directory, a full disk, a file another process is mid-write on — must not fail the command
/// that asked for the answer, and must not be reported as anything: the next ask recomputes,
/// which is what would have happened anyway. Written in place, like every other JSON artifact
/// in this workspace; a reader that catches a half-written file treats it as a miss.
pub(crate) fn write(
    level: &Level,
    slot: &Slot,
    routes: Option<&GoalRoutes>,
    verdict: Option<&CachedVerdict>,
) {
    let payload = Payload {
        format: FORMAT.to_string(),
        version: VERSION,
        sim_version: SIM_VERSION.to_string(),
        move_set: reach::MOVE_SET_VERSION,
        level: level.name.clone(),
        level_hash: level.content_hash_string(),
        routes: routes.cloned(),
        verdict: verdict.cloned(),
    };
    if let Ok(json) = serde_json::to_string_pretty(&payload) {
        let _ = std::fs::write(slot.path(), json + "\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sidecar_sits_beside_the_level_and_does_not_look_like_one() {
        let sidecar = |name: &str| sidecar_path(Path::new(name)).display().to_string();
        assert_eq!(
            sidecar("levels/l1-flat-run.json"),
            "levels/l1-flat-run.json.routes"
        );
        // The whole name, not the stem: a level whose name carries a dot keeps it, so two
        // levels called `hard.json` and `hard.practice.json` cannot collide.
        assert_eq!(
            sidecar("levels/hard.practice.json"),
            "levels/hard.practice.json.routes"
        );
        assert_eq!(sidecar("plain"), "plain.routes");
        // And never a level file: everything that lists levels in this workspace does it by the
        // `.json` extension.
        for name in ["levels/l1-flat-run.json", "levels/hard.practice.json"] {
            assert_ne!(
                Path::new(&sidecar(name)).extension(),
                Some(std::ffi::OsStr::new("json")),
                "a cache must not be offered to a level lister as a level"
            );
        }
    }
}
