# Audit — 2026-10-15

> **Status: addressed the same day.** Every item below has been implemented or explicitly
deferred, and the workspace is green (`cargo test --workspace`, 553 tests, 0 failed) with all
five views rendered and looked at (`--screenshot` for edit / play / evolve / replay / splash).
>
> | # | Item | State | Where it landed |
> |---|---|---|---|
> | B1 | Unsaved edits discarded silently | **done** | `EvoApp::refuses_to_discard` + `discard_guard`; guards New and every shelf click |
> | B2 | Every new level called `untitled`, overwriting each other | **done** | `editor::free_level_name`, `EvoApp::name_the_document`, `Editor::rename` + the name field in the details window |
> | B3 | A scripted `--level` typo is a silent success | **done** | `EvoApp::new` fails the process when `--screenshot` was asked for |
> | B4 | A worker panic looks like a finished run | **done** | `lab::WorkerGuard`, `Lab::panicked`, the dock's banner |
> | B5 | The window title is a constant | **done** | `window_title_for` + `EvoApp::sync_window_title` |
> | F1 | Two dozen keys, no way to look them up | **done** | `crates/app/src/help.rs` + `?`/`F1` + the permanent `? keys` hint in the status bar |
> | F2 | The trainer's controls are command-line only | **done** | *Run settings* in the run's details window (population, seed, cap, curriculum step) |
> | F3 | The editor cannot mirror a level | **done** | `Editor::mirror_horizontal`, a new `ui_mirror` glyph (`tools/art/ui.py`), the dock's 7th tool |
> | F4 | No eyedropper, no straight line, no region tools | **two of three** | `Editor::pick_at` + ctrl-click, `Editor::line` + shift-drag. **Region select/copy/paste is deferred** — see below |
> | F5 | No window icon, no pointer affordances | **done** | `atlas::window_icon` (the runner, 4×), `CursorIcon` on the plates and the canvas |
> | V1 | Mode changes are instant cuts | **done** | `MODE_FADE` shutter over everything but a scripted frame |
> | V2 | The brush's target is invisible | **done** | `EvoApp::draw_brush_ghost` on hover, the same picture the palette drag draws |
> | V3 | The shelf never says a level has been beaten | **done** | `shelf::Entry::champion` + the line on the card |
> | V4 | No audio anywhere | **deferred** | It needs a new dependency (`rodio`/`cpal`) and generated sound assets — a change of its own kind, and the one item here that cannot be verified headlessly. Recorded below rather than half-built |
> | G1 | The version-cost asymmetry is undocumented | **done** | the *Status* section of `README.md`, next to the version trio |
> | G2 | Sharp edges that are already correct | **nothing to do** | recorded so nobody re-opens them |
>
> Two caveats worth reading before trusting the table. The **mode shutter and the hover ghost
have no test**: both are pure view code inside `EvoApp`, which cannot be constructed without a
window, so what verifies them is the rendered frame above and the reader's eye rather than the
suite. And **region select** is the one feature left undone: it is the largest item in this
document, the audit already said to do it last, and a half-built selection tool that silently
loses a selection is worse than the copy-by-hand it would replace.

A pass over the whole workspace hunting for (1) errors, bugs and data loss, (2) friction,
(3) features the core already supports but the window does not expose, and (4) visual polish.

**Method.** Read the window crate (`crates/app`) end to end, the deterministic core
(`crates/sim`), the trainer (`crates/evo`, `crates/train-cli`), the art pipeline
(`tools/art`), the CI workflow and the three long-form docs (`REVIEW.md`, `FINDINGS.md`,
`README.md`). Claims below carry a `file:line`; anything marked **verified by reading** was
not reproduced by running the app, anything marked **verified by running** was.

**Checks run.** `cargo test --workspace` — **green**: 0 failed, 0 ignored, exit 0 (the full
suite, including the determinism, reachability and playability tests CI runs). Every
finding below therefore survives a green suite, which is itself the point: B1–B3 live in the
window's *document lifecycle*, and nothing in the test set drives the dock's New button or a
shelf click, so no test can currently fail when a person's level is thrown away.

**What this audit found zero of.** No `todo!()` or `unimplemented!()` anywhere under
`crates/*/src` (verified by search), no `unwrap()` on a user-supplied path outside tests,
no wall-clock or thread dependence anywhere in `crates/sim`, no floating-point comparison
in the sim's state hash, no ignored test. The workspace is in much better shape than the
brief assumes, and the sections below are weighted accordingly: the real findings are
concentrated in the window's document lifecycle (B1–B3), not in the simulation. Where I
expected a classic defect and found the code already correct, I recorded it in §3 (G2) rather
than inventing a finding.

---

## 1. Bugs

### B1 — Unsaved level edits are discarded without a word *(data loss, highest)*

`Editor.dirty` is set by every mutation (`crates/app/src/editor.rs:421,431,581,609,625`) and
cleared by a successful save (`editor.rs:713`), but it is only ever **read to draw**:

* `crates/app/src/app.rs:1428` — the dock plaque's subtitle.
* `crates/app/src/app.rs:1483` — the gold Save button.

Both ways of replacing the document ignore it:

* `crates/app/src/app.rs:1529-1533` — the **New** button: `self.editor = Editor::fresh(...)`.
* `crates/app/src/app.rs:1342-1354` — `open_level`, which the shelf, the playtest and the
  editor all route through: `self.editor = editor;` with no check.

So a person who drags forty tiles and then clicks a level in the shelf loses the lot, and
the *only* signal that anything was pending was a dot they were not looking at. Painter
programs, editors and this project's own `train-cli` are all careful about the analogous
case (the trainer refuses to overwrite an existing artifact); this is the one place the
window is not.

**Smallest fix.** One guard in each path, reusing the state that already exists:

```rust
if self.editor.dirty && !self.discard_confirmed {
    self.editor.message = "unsaved changes — Save first, or click again to discard".into();
    self.discard_confirmed = true;
    return;
}
```

`discard_confirmed` cleared on any edit, on save, and on mode change. The second click is
the confirmation, which costs no new widget and no new window. Worth a test that the level
is unchanged after the first click (`crates/app/src/editor.rs` already has the harness for
document-level tests, e.g. `a refused save leaves the document edited` at `editor.rs:1133`).

### B2 — Every new level is called `untitled`, and they overwrite each other *(data loss)*

`Editor::fresh("untitled", 60, 15)` is hardcoded at both creation sites
(`crates/app/src/app.rs:305` for a failed open, `:1530` for **New**), the name is stored at
`crates/app/src/editor.rs:306`, and an untitled save derives its path from the name:

```rust
self.levels_dir.join(format!("{}.json", lab::slug(&self.editor.level.name)))   // app.rs:1355-1361
```

Three consequences:

1. **New → Save → New → Save silently overwrites the first level.** `untitled.json` is the
   target both times, and `Editor::save` (`editor.rs:700+`) does not refuse an existing file
   the way `train-cli` refuses an existing champion.
2. There is **no way to type a name**: no `TextEdit` anywhere in `crates/app/src`
   (verified by search), and the dock plaque (`crates/app/src/ui.rs:723`) renders the title
   as a label. So *every* hand-made level in the pack is named `untitled`, which is also what
   the shelf cards (`crates/app/src/shelf.rs:65`) and the window plaque show.
3. The name feeds `lab::slug`, so `.`, spaces and capitals are all mangled silently.

**Smallest fix.** Two independent halves: (a) on save with the name still `untitled`, pick
the first free `untitled-N`; (b) make the plaque's title an editable field (a single-line
`TextEdit` behind a click on the name, or a rename row in the Details panel where
`Resize the grid` already lives at `app.rs:2139`). Half (b) also fixes the shelf's
readability, which is a *pack* feature as much as an editor one.

### B3 — A scripted `--level` typo is a silent success *(CI/screenshot hazard)*

`crates/app/src/app.rs:302-309` deliberately degrades a failed open into a fresh untitled
level with the error text in `editor.message` — correct for a person at a window, wrong for a
script. `main` only fails on an `Err` from `app::run` (`crates/app/src/main.rs:211-224`),
and `run` returns `Ok(())` here, so:

```
cargo run -p app -- --level levels/typo.json --screenshot /tmp/x.png   # exit 0, PNG of an empty level
```

The screenshot is of 60 × 15 tiles of nothing, and nothing in the log says the level was
never opened. `--screenshot` exists precisely to verify the renderer without a human, and
this is the one path where it can lie.

**Smallest fix.** In `app::run`, when `screenshot.is_some()` and `Editor::open` fails,
return the error instead of falling back. One condition on the existing branch.

### B4 — A worker panic looks like a finished run *(minor, silent)*

The lab's worker loop (`crates/app/src/lab.rs:955-1000`) is careful about the lock: the
scoring — the only panic-prone part — runs in `pool.install(|| generator.next_packet())`
*outside* the mutex, so a bad genome cannot poison the hub. But the thread's handle is
discarded at join (`let _ = handle.join();`, `lab.rs:1281`), and the panic is not caught
anywhere (`catch_unwind` appears nowhere in the crate). A worker that panics therefore
disappears: `Lanes::is_finished` (`lab.rs:349`) starts returning `true`, so the window
presents a dead run as a completed one and sits on the last generation with no message.
I did not construct a failing worker to reproduce this — it is a reading of the control
flow, flagged because the symptom (a run frozen at "finished") is indistinguishable from
success. **Smallest fix:** have the workers publish a `panicked` flag on the hub from a
`Drop` guard before the join, and show it in the run's banner.

### B5 — The OS window title is a constant

`crates/app/src/app.rs:170` — `.with_title("evolution platformer")`. The document's name
reaches the in-window plaque and nowhere else, so two windows on two levels are
indistinguishable in a taskbar, a window list or a screenshot bundle. `egui` makes this a
one-liner (`ViewportCommand::Title`) issued when the document or its dirty flag changes:
`evolution platformer — l1-flat-run *`.

---

## 2. Friction

### F1 — Twenty-eight key bindings and no way to look them up *(highest-value UI fix)*

`handle_shortcuts` (`crates/app/src/app.rs:858`, called at `:2715`) binds about two dozen
keys in the editor and play modes alone, and the replay, lab, panel and shelf add more
(`crates/app/src/replay.rs`, `lab.rs`, `panel.rs`, `shelf.rs`). The only teaching anywhere
is per-widget hover text, plus the splash's two keys — `P play it yourself  E evolve an AI`
(`crates/app/src/splash.rs:58`).

There is **no help overlay**: no `F1`, no `?`, no keys section in the panel (verified by
search — `fn keys`/`fn help`/`KEYS` exist only as the splash's two-key line). For a tool
whose documentation is otherwise extraordinary, the keys are the one thing a player is
expected to already know.

**Fix.** A `?`/`F1` sheet drawn in the *current mode's* language (Edit / Play / Evolve /
Replay), reusing the plate-and-bevel furniture in `crates/app/src/ui.rs` and `theme.rs`.
Pure app-side: no version stamp, no champion invalidated.

### F2 — The trainer's controls are command-line only

Population, seed, generations, trials, curriculum, novelty and the champions directory are
`LabOptions` fields parsed only from argv (`crates/app/src/main.rs:73-100`, `:150-180`); the
run's *banner* shows the values in the window but nothing in the window sets them. The lab
panel can Restart (which deliberately bumps the seed, `app.rs:228-231`) but not "restart with
pop 400" or "resume this checkpoint". Trying a different population means killing the window
and relaunching, which also abandons the run on screen. A small **Run settings** dialog —
pre-filled from the current `LabOptions`, applied on the next run — would close the biggest
gap between the CLI and the window.

### F3 — The editor cannot mirror a level, though the core can

`Level::mirrored_horizontal` (`crates/sim/src/level.rs:688`) is a real, *tested* capability:
`crates/sim/tests/levels.rs:400` and `crates/evo/tests/difficulty.rs:182` both build mirrored
copies of shipped levels to prove spawn/goal symmetry. The editor never calls it — `Brush`
(`crates/app/src/editor.rs:77-102`) has eleven brushes and no mirror, and no dock button
does either. Mirroring is the single most useful level-design verb (it is how you get a
return trip out of one stroke) and it is sitting in the library already proven correct.
**Fix:** one dock button or one palette cell that replaces `editor.level` with its mirror,
snapshotted into the undo history like every other document change.

### F4 — No eyedropper, no region select, no copy/paste

The palette gives 11 brushes and a stroke history (`crates/app/src/editor.rs:421` ff.), and
middle/alt-drag pans (`app.rs:2187-2189`), but:

* no **alt-click to pick** the tile or entity under the cursor (the nearest thing is Undo);
* no **rectangle select / move / copy** of a region — a 60 × 15 level can only be retyped;
* no shift-constrained straight stroke, so a long floor edge is a steady hand and luck.

The first is a few lines (the brush enum is `Copy`, `hover_tile` is already tracked at
`app.rs:2176`); the second is the one substantive editor feature still missing.

### F5 — No window icon, no pointer affordances

No `set_window_icon`/`IconData` and no `CursorIcon` anywhere in `crates/app/src` (verified by
search), so the app wears the platform default icon and the pointer never changes shape:
crosshair over the canvas, pointing hand over the dock, resize arrow over a level edge. The
art pipeline already generates 16 px sprites (`tools/art/`), so an icon is a build step
away.

---

## 3. Gameplay and feel

### G1 — The expensive class of change, and the cheap class

Any edit to a sim constant, sensor encoding or fitness term invalidates every stored
artifact: the version trio (`sim::SIM_VERSION`, `sensors::VERSION`,
`fitness::FITNESS_VERSION`) is stamped into every champion and checked on load,and `.github/workflows/ci.yml` exists to enforce the determinism promise that makes them
comparable. There are 29 files in `champions/` — trained champions, `.population.json`
checkpoints and `.practice.json` scratch runs, all carrying the stamp.

So: **feel changes cost a retraining, UI and visual changes cost nothing.** Everything in
§2 and §4 is free to adopt and cannot break a champion; nothing in this section is. That
asymmetry should decide the order of work, and it is worth stating in `README.md`, because
it is not obvious from the flag list.

### G2 — Sharp edges that are already handled (do not "fix" these)

I went looking for these specifically and found them correct, which is worth recording so
nobody re-opens them:

* **The anti-camper timer does not kill climbers.** `NO_PROGRESS_FRAMES = 600`
  (`crates/sim/src/lib.rs:55`) resets on `closest_approach` *or* `route_advanced`
  (`lib.rs:466-477`), not on raw rightmost x — so upward progress toward the goal counts as
  progress. The v2 failure that `fitness.rs:90-91` documents (`Stuck killed climbers`) is
  fixed in the sim's own accounting, not papered over in the score.
* **Walker jitter cannot shove a body into a pit.** An unsupported random offset is
  *rejected*, not clamped (`crates/sim/src/physics.rs:881-889`), which preserves the
  determinism of `trials` while keeping the only gameplay randomness legal.
* **Pixel crispness is already exact.** `Camera::fit` floors the zoom and `Camera::scale`
  rounds it, with the tile origin snapped once so neighbouring tiles share an edge
  (`crates/app/src/view.rs:274-318`), and the atlas is uploaded `NEAREST`
  (`crates/app/src/atlas.rs:216`). There is no shimmer to fix and no integer-zoom work to do.
* **Spikes are tile atoms by design.** `Tile::Spike` is a tile (`crates/sim/src/level.rs:52`)
  and the sprite fills the cell (`crates/app/src/view.rs:2279-2281`) — art and hitbox agree.
  A sub-tile hitbox would be a *feel* change with a champion-retraining price (G1), not a bug.

---

## 4. Visual polish

All app-side, all free (G1), ranked by payoff per line:

1. **Mode transitions.** Edit ↔ Play ↔ Evolve ↔ Replay are instant cuts. The splash already
   establishes the language — an eased camera pullback (`crates/app/src/splash.rs:200-215`)
   — so a ~120 ms fade or horizontal wipe through the same easing would make the four modes
   read as one place. The app already owns a full-screen painter each frame (`app.rs:2168`
   ff.), so it is a draw-order problem, not an architecture one.
2. **A hovered-tile ghost.** The editor knows the hovered tile (`app.rs:2176`), but nothing
   is drawn on it, so the brush's target is invisible until the click lands. A clipped
   silhouette of the held brush's sprite at 40 % alpha, snapped by `Camera::to_screen`, is
   the single change that most improves how the editor *feels*.
3. **The shelf as a pack, not a list.** Cards are already rendered from the level
   (`crates/app/src/shelf.rs:102`), and champions are on disk in `champions/`, yet a card
   never says "a champion has won this". A small badge on levels with a saved winner (and a
   dimmed card for levels retrained past their version, which the loader can already detect)
   would connect two halves of the game that currently do not speak.
4. **An audio pass.** There is no sound anywhere — no crate, no asset. A 6-sprite-equivalent
   of effort: a footstep tick, a jump, a spike death, a win sting on the flag raise, and a
   generation-chime in the lab. The win/death bursts already exist as particles
   (`crates/app/src/view.rs:2656-2685`), so the visual grammar is defined and the audio
   grammar is simply absent.
5. **Window icon** (see F5) — the one piece of the art pipeline a player sees before the
   window opens.

---

## 5. Suggested order

| # | Item | Cost | Why first |
|---|------|------|-----------|
| 1 | B1 unsaved-work guard | ~15 lines + 1 test | Only data-loss bug with a one-click trigger |
| 2 | B2 unique name on save | ~10 lines | Second half of the same data-loss story |
| 3 | B3 fail loudly on a bad scripted level | ~3 lines | Makes the screenshot path trustworthy |
| 4 | F1 `?` shortcut sheet | ~80 lines, app-side | Largest friction payoff, zero risk |
| 5 | F3 mirror button | ~20 lines | A tested capability the UI forgot |
| 6 | B5 window title, B4 worker-panic flag, F5 icon + cursors | ~30 lines | Cheap, visible, no risk |
| 7 | V2 hovered-tile ghost, V1 transitions | ~60 lines | The editor's feel |
| 8 | F2 run-settings dialog | ~150 lines | Closes the CLI/window gap |
| 9 | F4 region select | largest | Do after the guard exists to protect it |
| 10 | G1/G2 feel changes | a retraining each | Only once §2–§4 are exhausted |

Items 1–6 are independent and small enough to land as one sweeping session; each is
verifiable by `cargo test -p app` plus a `--screenshot` frame, and none of them can
invalidate a champion.
