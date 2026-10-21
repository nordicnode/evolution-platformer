//! The evolution view's panels: what the run is doing, and every control on it.
//!
//! Like [`crate::view`], this module **only draws**. It owns no simulation, advances
//! no generation, and holds no trainer: the window's `lab` runs the evolution on its
//! own thread and hands this module a finished [`Packet`] plus the run's history.
//!
//! Everything the panel *says* about the run is computed by `render::chart`, not
//! here — [`fitness_trend`], [`fitness_stall`], [`death_cluster`] and
//! [`format_fitness`] are the same functions the terminal watch mode puts on screen.
//! That is the whole reason this module is thin: the window and the terminal are two
//! views of one run, and a viewer must not be able to read "few good agents" in one
//! and "learning" in the other about the same numbers.
//!
//! The chart is drawn as real geometry rather than the terminal's character grid.
//! `render::chart::fitness_chart` composes a *text* plot, and the plan's own boundary
//! rule says pixels are the GUI's business, so the split is: `render` decides what the
//! numbers mean, this module decides where the lines go. [`chart_geometry`] is that
//! decision as a pure function of the history and a rectangle, which is what makes it
//! testable without a window.

use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2,
};

use render::chart::{
    death_cluster, fitness_stall, fitness_trend, format_fitness, HistoryPoint, Stall, Trend,
};

use crate::atlas::Atlas;
use crate::lab::{Lane, Packet, MUTATION_RATES, SPEEDS};
use crate::replay::{Population, Replay, StarBasis};
use crate::theme;
use crate::ui;
use crate::view::cohort_swatch;
use sim::Outcome;

// The panel's colours are the window's palette, named once in `crate::theme` rather than
// re-typed here. The old set was this module's own near-misses of the same eleven colours —
// a `TEXT` four shades off the window's, a `GOOD` that was the same green spelled
// separately — and a game whose furniture is drawn in three palettes reads as three
// programs. The names stay local because the drawing code reads better with them
// (`AXIS` says more at a `line_segment` than `theme::EDGE` does) and because a name that is
// only ever used two lines from where it is defined is a name that does not need a path.

/// The chart's own background. **The chart is a screen, not a panel.** Everything else in the
/// window is cream chrome with dark ink; this strip is the run's instrument, and instruments
/// are dark — it is drawn as a display set into the chrome, framed by it.
const PANEL_BG: Color32 = theme::SHELL;
/// The chart's plot area, cut into that display so the lines read as data.
const PLOT_BG: Color32 = theme::SCREEN_WELL;
/// The chart's frame and its baseline: the edge of the display, at the control-boundary weight.
const AXIS: Color32 = theme::SCREEN_EDGE;
/// The chart's interior rules: a scale, one step quieter than its frame.
const GRID: Color32 = theme::SCREEN_GRID;
/// The best-of-generation line: the growth colour, because the max series *is* the run's
/// growth and the chart's headline series.
const MAX_LINE: Color32 = theme::GOOD;
/// The average line: the cool end of the palette, so the two series can never be confused
/// for one another even in a monochrome screenshot.
const AVG_LINE: Color32 = theme::COOL;
/// A generation that reached the flag: the accent, the window's "look at this" colour.
const WIN_TICK: Color32 = theme::ACCENT;
/// The cursor of a scrubbed, i.e. not-live, view: the warning colour, because that is what
/// looking at the past while a run continues is.
const SCRUB_LINE: Color32 = theme::WARN;
/// The panel's own ink, for type on the cream chrome around it.
const TEXT: Color32 = theme::TEXT;
const TEXT_DIM: Color32 = theme::TEXT_DIM;
/// Type *on the chart*, which is a display: the panel's inks are for cream and would be
/// invisible on it. Two ink scales in one module, because the module draws on both materials.
const SCREEN_TEXT: Color32 = theme::SCREEN_INK;
const SCREEN_TEXT_DIM: Color32 = theme::SCREEN_INK_DIM;
/// A verdict, on chrome: the bright greens and ambers are lamps and chart lines, and type needs
/// the darker half of each pair.
const GOOD: Color32 = theme::GOOD_INK;
const WARN: Color32 = theme::WARN_INK;

/// Room left for the axis labels, inside the chart's rectangle.
const LABEL_W: f32 = 52.0;
const LABEL_H: f32 = 16.0;

/// One generation's champion, as the scrubber needs it.
///
/// A copy of the recorded facts rather than a [`render::trace::ChampionTrace`]: the
/// window's canvas draws the route from the packet it already owns, and this copy is
/// small enough to live in the summary, which keeps the slider and the chart from
/// having to reach into the packet for a single number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordedGeneration {
    pub generation: u32,
    pub fitness: f32,
    /// The route ended at the flag rather than in a death.
    pub won: bool,
}

/// The run's best agent so far, as the panel prints it.
#[derive(Clone, Debug, PartialEq)]
pub struct BestLine {
    pub generation: u32,
    pub fitness: f32,
    pub genome_id: u32,
    /// The outcome's own word ("won", "died", ...), for display.
    pub outcome: String,
    pub frames: u32,
    pub best_x: f32,
    /// Tiles still to go when the episode ended. Zero if it touched the flag.
    ///
    /// Carried because fitness alone cannot be read: it is a weighted progress score,
    /// so 300 means "three quarters of the way" on one level and "barely started" on a
    /// longer one. The remaining distance is the same number in every level's units.
    pub distance_to_goal: f32,
}

impl BestLine {
    /// Build the champion line from one finished episode's own numbers.
    ///
    /// `outcome` is the label to show, `won` is the fact that decides the distance (a
    /// winning episode is at the flag by definition, whatever `best_x` says — the flag
    /// is touched by a box, so its left edge is a tile short of the goal column), and
    /// `goal_x` is the level's flag column in tiles.
    #[allow(clippy::too_many_arguments)]
    pub fn from_episode(
        generation: u32,
        genome_id: u32,
        fitness: f32,
        outcome: &str,
        won: bool,
        frames: u32,
        best_x: f32,
        goal_x: f32,
    ) -> BestLine {
        let distance = if won { 0.0 } else { (goal_x - best_x).max(0.0) };
        BestLine {
            generation,
            fitness,
            genome_id,
            outcome: outcome.to_string(),
            frames,
            best_x,
            distance_to_goal: distance,
        }
    }
}

/// The run, summarised for the panel.
///
/// Built in steps, because two of the facts it reports are not in the packet at all:
/// the level's width (the death-cluster rule needs it to know which marks are even
/// inside the level) and the champion's episode (its outcome, frame count and final
/// position live in `evo`'s best-ever record, which the trainer owns). [`RunSummary::new`]
/// produces an honest summary with both missing; [`RunSummary::with_level`] and
/// [`RunSummary::with_champion`] fill them in from whoever has them.
#[derive(Clone, Debug)]
pub struct RunSummary {
    pub generation: u32,
    /// Generations this run has been through, counted from its own history.
    pub generations_run: u32,
    /// Generations the run was asked for, when it has a cap.
    pub target: Option<u32>,
    pub pop_size: usize,
    pub wins: usize,
    /// Genomes that won **every** episode they were judged on this generation — the conservative
    /// reading of [`RunSummary::wins`], and the one that is not a coin flip on a level where a
    /// patrol phase is drawn per episode.
    pub solved: usize,
    pub max_fitness: f32,
    pub avg_fitness: f32,
    pub min_fitness: f32,
    /// The mutation step the children of this generation were bred with.
    ///
    /// Carried through from the packet rather than derived here: it is a fact about the
    /// search, not a reading of the history. Constant unless the run was started with
    /// `adaptive_sigma`, and even then it is the run's own number rather than the panel's.
    pub mutation_sigma: f32,
    /// Generations since the best fitness last improved, straight from the trainer.
    pub stagnation: u32,
    /// Genomes reheated from the run's best-ever copy and bred in after that stagnation.
    /// Non-zero only after a stall.
    pub immigrants: usize,
    /// Children bred from the quality-diversity archive, which arrive every generation rather
    /// than only after a stall.
    pub archive_children: usize,
    /// Children bred from a mistake the generation's best genome made (Item 14). Read as the
    /// run's own account of *why* the next generation looks the way it does: these children are
    /// not perturbations, they are the same policy with one decision corrected.
    pub repairs: usize,
    /// Behavioural niches the archive holds, of `evo::ARCHIVE_CELLS`. The run's diversity
    /// reading: it falls when the population collapses onto one way of moving.
    pub archive_bits: usize,
    /// Finish bands holding a win, of `evo::ARCHIVE_TIME_BINS`. The run's speed coverage, and
    /// a different reading from the one above: it climbs only when the population learns to
    /// solve the level *quicker*, never when it finds a new way of moving.
    pub archive_time_bits: usize,
    /// The quickest win the run has recorded, and whose it was (see [`evo::FastWin`]).
    ///
    /// Carried rather than derived: it is the run's own record rather than a reading of the
    /// history, and it stands across generations that win nothing, which is what makes it a
    /// time to beat rather than a number that flickers.
    pub fastest: Option<evo::FastWin>,
    pub best: Option<BestLine>,
    /// What `render`'s own rule makes of the history.
    pub trend: Trend,
    pub stall: Option<Stall>,
    pub genius: bool,
    /// **The newest find (WP16)**, when this generation produced one that the level's codex had
    /// not held before. `None` for a rediscovery, which is not news, and for a generation that did
    /// not move the record.
    pub discovery: Option<evo::Discovery>,
    /// How many finds the level's codex holds, after this generation.
    pub codex_len: usize,
    /// The note from the run's one curriculum step, if it fell back to an easier level
    /// when it stalled (§12 M5). A fact about the run rather than a reading of its
    /// history, so it is carried through from the packet and not derived here.
    pub curriculum: Option<String>,
    /// **Phase 0: what this generation learned.** Solved rate, the winning clock, the leg and
    /// frame the best genome ended on, and the per-leg census of the population. Carried from
    /// the trainer's report rather than recomputed: the panel is a view of the run, and a second
    /// implementation of the leg census would be a second answer to "where did they die".
    pub telemetry: evo::Telemetry,
    /// Whether a champion file exists for this level, and so whether *Continue champion*
    /// has anything to continue.
    ///
    /// Set by the window when the menu is drawn rather than when the generation was scored:
    /// pressing Save and then Continue in the same generation has to work, and a reading
    /// taken one generation earlier would leave the second control disabled for no reason.
    pub champion_saved: bool,
    pub finished: bool,
    /// Tile the population is dying in, when a stall makes that the interesting
    /// question. `None` before a stall, and `None` until a level width is supplied.
    pub deaths_at: Option<i32>,
    /// How many died there. The tile alone reads as a coordinate; the count is what
    /// tells a level designer whether it is a wall or a trickle.
    pub deaths_count: usize,
    pub level_name: String,
    /// The recorded improvements the scrubber walks, oldest first.
    pub recorded: Vec<RecordedGeneration>,
    /// The lanes of the newest generation, for the panel's list.
    pub lanes: Vec<Lane>,
    /// The rolling window of deaths the cluster rule reads. Private because it is only
    /// ever needed once, to name a tile the moment a stall fires, and because it is the
    /// one field here that is a copy of the run rather than a reading of it.
    death_marks: Vec<(f32, f32, u32)>,
}

impl RunSummary {
    /// Summarise a finished generation.
    ///
    /// `history` is the whole run's history in generation order; the trend and the
    /// stall are read from it and from nothing else.
    pub fn new(packet: &Packet, history: &[HistoryPoint]) -> RunSummary {
        let mut recorded: Vec<RecordedGeneration> = packet
            .traces
            .iter()
            .map(|trace| RecordedGeneration {
                generation: trace.generation,
                fitness: trace.max_fitness,
                won: trace.won,
            })
            .collect();
        recorded.sort_by_key(|r| r.generation);

        RunSummary {
            generation: packet.generation,
            generations_run: history.len() as u32,
            target: packet.target,
            pop_size: packet.pop_size,
            wins: packet.wins,
            solved: packet.solved,
            max_fitness: packet.max_fitness,
            avg_fitness: packet.avg_fitness,
            min_fitness: packet.min_fitness,
            mutation_sigma: packet.mutation_sigma,
            stagnation: packet.stagnation,
            immigrants: packet.immigrants,
            archive_children: packet.archive_children,
            repairs: packet.repairs,
            archive_bits: packet.archive_bits,
            archive_time_bits: packet.archive_time_bits,
            fastest: packet.fastest,
            best: None,
            trend: fitness_trend(history),
            stall: fitness_stall(history),
            genius: packet.genius,
            discovery: packet.discovery.clone(),
            codex_len: packet.codex_len,
            curriculum: packet.curriculum.clone(),
            telemetry: packet.telemetry.clone(),
            champion_saved: false,
            finished: packet.finished,
            deaths_at: None,
            deaths_count: 0,
            level_name: packet.level_name.clone(),
            recorded,
            lanes: packet.lanes.clone(),
            death_marks: packet.deaths.clone(),
        }
    }

    /// Add the one thing the packet cannot carry: how wide the level is.
    ///
    /// Only then can the busiest death tile be named, because the rule (and the death
    /// strip beside it) deliberately ignores marks outside the level — a corpse in a
    /// pit below the map is not a level-design problem.
    pub fn with_level(mut self, level_width_tiles: i32) -> RunSummary {
        // The cluster is only asked for once the run has stalled: before that, "where
        // is it stuck" is not a question, and a red tile would answer a question
        // nobody asked (the terminal watch mode makes the same call).
        let cluster = if self.stall.is_some() {
            death_cluster(&self.death_marks, level_width_tiles)
        } else {
            None
        };
        if let Some((tile, count)) = cluster {
            self.deaths_at = Some(tile);
            self.deaths_count = count;
        }
        self
    }

    /// Attach the champion's episode. The window builds this from `evo`'s best-ever
    /// record, which is where the outcome, the frame count and the final position are.
    pub fn with_champion(mut self, best: BestLine) -> RunSummary {
        self.best = Some(best);
        self
    }

    /// Whether the run has **solved** the level — a genome that won every episode it was judged
    /// on, not one that touched the flag on the phase it happened to draw.
    ///
    /// The colour this drives is the run's answer rather than its standing fact, so it reads the
    /// conservative census ([`RunSummary::solved`]); a walker level whose `wins` are all
    /// single-phase draws stays dim, which is the honest reading. The champion's own episode is
    /// still accepted, because a record whose episode is a win is by construction solved.
    pub fn solved(&self) -> bool {
        self.solved > 0 || self.best.as_ref().is_some_and(|b| b.outcome == "won")
    }
}

/// What the window is doing right now, owned by the window and edited by the widgets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    pub paused: bool,
    /// Physics ticks per 1/60 s of wall clock (see `lab::SPEEDS`).
    pub speed: u32,
    /// Per-gene mutation probability, always one of `lab::MUTATION_RATES`.
    pub mutation_rate: f32,
    pub show_deaths: bool,
    /// Follow the leading agent's camera rather than holding the whole level still.
    ///
    /// Off by default, and that is the one place the two front-ends differ on purpose:
    /// the terminal follows because its frame is eighty columns wide and cannot show a
    /// whole level, while the window can — so following here is the viewer asking to
    /// zoom in on the leader, not the only way to see anything. The terminal's `f` key
    /// and this checkbox are the same control either way.
    pub follow: bool,
    /// The recorded generation being shown, or `None` for the live view.
    pub scrub: Option<u32>,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            paused: false,
            speed: crate::lab::DEFAULT_SPEED,
            mutation_rate: MUTATION_RATES[1],
            show_deaths: true,
            follow: false,
            scrub: None,
        }
    }
}

/// One thing the viewer asked the run to do.
///
/// A button and a key are two ways of saying the same thing, so they produce the same
/// value: the window translates both into these and handles them in one place. Anything
/// else would be two implementations of "pause", "save" and "scrub the record", which is
/// how a key and a button start disagreeing about what the same word means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Pause if running, resume if paused.
    TogglePause,
    /// Run exactly one generation and stop.
    StepOne,
    /// A fresh population on the level as it stands.
    Restart,
    /// Write the best-ever genome out as a champion file.
    Save,
    /// Start a new population from the champion saved for this level, when there is one.
    ContinueChampion,
    /// Leave the scrubbed view and follow the live run again.
    Live,
    /// Follow the leading agent's camera, or go back to the whole level.
    ToggleFollow,
    /// Show or hide the replay's ghosts.
    ToggleGhosts,
    /// Walk the record: -1 older, +1 newer.
    Scrub(i32),
    /// One rung of the speed ladder: +1 faster, -1 slower.
    Speed(i32),
    /// Dial the mutation rate, in rungs of the ladder.
    MutationRate(f32),
}

/// What the viewer pressed this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Actions {
    pub pause: bool,
    /// Run one generation and then stop.
    pub step: bool,
    pub restart: bool,
    pub save: bool,
    /// Start a new population from the champion saved for this level.
    pub continue_champion: bool,
    /// Leave the scrubbed view and follow the live run again.
    pub resume: bool,
    /// The follow checkbox was clicked. The window owns the state, as it does for pause:
    /// the widget reports the intent and the state is flipped in one place.
    pub follow: bool,
    /// The ghosts row was pressed, in the replay's own panel.
    pub ghosts: bool,
    /// -1 / +1 from the record's own step buttons.
    pub scrub_delta: i32,
    /// One rung of the speed ladder: +1 faster, -1 slower. Reported rather than applied, so a
    /// button on the panel and the `+`/`-` keys reach the run through one command.
    pub speed: i32,
    /// Open or close the Details window.
    ///
    /// The one request a panel makes that is not about the run: both panels carry the row, because
    /// the board behind it is about a run and the window is where a run's numbers are read. What the
    /// window holds depends on the mode, so the request is a toggle and the window decides.
    pub details: bool,
}

/// One step older in the record, stopping at the oldest.
///
/// The walk is by **generation**, never by index into the record: the log drops its
/// oldest entry when it fills, which shifts every index by one and would silently
/// re-point the cursor at a route the viewer never chose. Entering from the live view
/// starts at the newest recorded route, because that is the one that explains where the
/// run has got to; the older ones are the history of how it got there.
///
/// Shared by the panel's `<` button and the window's left-arrow key, so the two cannot
/// walk the record differently.
pub fn scrub_back(generations: &[u32], current: Option<u32>) -> Option<u32> {
    match current {
        None => generations.last().copied(),
        Some(current) => generations
            .iter()
            .rev()
            .find(|g| **g < current)
            .copied()
            .or(Some(current)),
    }
}

/// One step newer; one step past the newest returns to the live view.
pub fn scrub_forward(generations: &[u32], current: Option<u32>) -> Option<u32> {
    let current = current?;
    generations.iter().find(|g| **g > current).copied()
}

/// Which evolution keys are down this frame.
///
/// Plain data rather than an egui input state, so what a key *means* can be tested
/// without a window — the terminal watch mode has the same test for the same reason
/// (`every_documented_key_maps_to_an_action`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// `space`
    pub pause: bool,
    /// `n`
    pub step: bool,
    /// `+` / `=`
    pub faster: bool,
    /// `-`
    pub slower: bool,
    /// `M` (shift-`m`)
    pub mutate_up: bool,
    /// `m`
    pub mutate_down: bool,
    /// `h`
    pub live: bool,
    /// `f`
    pub follow: bool,
    /// `g`
    pub ghosts: bool,
    /// `s`
    pub save: bool,
    /// `c`
    pub continue_champion: bool,
    /// `left`
    pub older: bool,
    /// `right`
    pub newer: bool,
}

/// The commands a set of pressed keys asks for, in the order they apply.
///
/// One place decides what a key *is*; the window only decides which keys are down. The
/// order is written down here rather than left to the order of the `if`s in a frame
/// function, because two keys landing on the same frame is a real case (a pause and a
/// step, or two rungs of a ladder) and "which wins" should not depend on where a line
/// happens to sit. The ladders resolve against `view`, so a keypress always carries a
/// value the other front-end could have reached too.
/// A section title, then a hairline under it.
///
/// The panel used to find its structure in bare `ui.label` lines, which the eye reads as
/// one more sentence: `Agents in this generation` sat in the same weight and colour as the
/// readings under it, so three sections looked like one column of prose. A title in the
/// accent colour, in the monospace the numbers use, with a rule under it, is the whole of
/// the fix — and it is one function rather than four call sites so the next section gets
/// the same treatment by existing.
pub fn section(ui: &mut egui::Ui, title: &str) {
    // A ribbon, not a label over a rule: the panel's sections are the *tabs* of a game's menu,
    // so each one is a bright blue plate with white caps on it, a brown outline and a rounded
    // corner. The strip takes the panel's whole width, which is what makes a column of them read
    // as one board of headings rather than as a document with subheads.
    ui.add_space(10.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(theme::RADIUS);
    painter.rect_filled(rect.translate(Vec2::new(0.0, 2.0)), radius, theme::OUTLINE);
    painter.rect_filled(rect, radius, theme::RIBBON);
    // The lit top edge every candy-coloured plate in this window wears: one pixel of white at
    // 35%, which is what makes blue read as a *plastic sign* rather than as a flat swatch.
    painter.rect_filled(
        Rect::from_min_size(
            Pos2::new(rect.left() + 8.0, rect.top() + 2.0),
            Vec2::new((rect.width() - 16.0).max(0.0), 1.0),
        ),
        radius,
        Color32::from_white_alpha(90),
    );
    let font = FontId::monospace(12.0);
    let galley = painter.layout_no_wrap(title.to_uppercase(), font, Color32::WHITE);
    painter.galley(
        Pos2::new(
            rect.left() + 10.0,
            rect.center().y - galley.size().y * 0.5 - 1.0,
        ),
        galley,
        Color32::WHITE,
    );
    ui.add_space(6.0);
}

/// Small print: a qualifier, a unit, a hint. Dim, so it never competes with a reading.
fn small_dim(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .small()
            .color(theme::TEXT_DIM),
    );
}

/// A reading: the number in the monospace the HUD uses, its unit dimmer beside it.
fn reading(ui: &mut egui::Ui, value: impl Into<String>, unit: &str) {
    ui.label(
        egui::RichText::new(value.into())
            .monospace()
            .color(theme::TEXT),
    );
    if !unit.is_empty() {
        ui.label(egui::RichText::new(unit).small().color(theme::TEXT_DIM));
    }
}

pub fn commands_for(keys: Keys, view: &ViewState) -> Vec<Command> {
    let mut out = Vec::new();
    if keys.pause {
        out.push(Command::TogglePause);
    }
    if keys.faster {
        out.push(Command::Speed(1));
    }
    if keys.slower {
        out.push(Command::Speed(-1));
    }
    if keys.mutate_up {
        out.push(Command::MutationRate(crate::lab::step_mutation_rate(
            view.mutation_rate,
            true,
        )));
    }
    if keys.mutate_down {
        out.push(Command::MutationRate(crate::lab::step_mutation_rate(
            view.mutation_rate,
            false,
        )));
    }
    if keys.step {
        out.push(Command::StepOne);
    }
    if keys.live {
        out.push(Command::Live);
    }
    if keys.follow {
        out.push(Command::ToggleFollow);
    }
    if keys.ghosts {
        out.push(Command::ToggleGhosts);
    }
    if keys.save {
        out.push(Command::Save);
    }
    if keys.continue_champion {
        out.push(Command::ContinueChampion);
    }
    if keys.older {
        out.push(Command::Scrub(-1));
    }
    if keys.newer {
        out.push(Command::Scrub(1));
    }
    out
}

/// The setup line about the search itself: the step, and — only when it has something
/// to say — how long the best has stood still and what that bought.
///
/// A pure function of the summary so the three shapes can be checked without a window, the
/// same way [`chart_geometry`] is split out of the chart's drawing. The clauses after the
/// step are conditional because a run that is still finding new bests has neither a stall to
/// report nor a restart to explain, and `still for 0   +0 reheated genomes` beside its score
/// would be noise dressed as information.
///
/// `sigma` is a constant on a run with a fixed step — the default — and there the line is
/// still worth its row: with reheated genomes only ever arriving after a stall, the line is
/// where a viewer sees the restarts happen.
///
/// It is the search's **own** knobs and nothing else. The two grid readings the board used to append
/// to this line — how many behavioural niches the archive holds and how many children the trickle
/// bred — are rows of their own above, and that is a promotion rather than a demotion: a clause on a
/// sentence is read as prose, while the same count in a column of readings is compared. They were
/// never the same kind of fact as the step and the stall anyway — those are about *how hard* the
/// search is pushing, and a niche count is about how far the population is *spread* — so one line
/// carrying both was one line answering two questions.
pub fn search_line(summary: &RunSummary) -> String {
    match (summary.stagnation, summary.immigrants) {
        (0, 0) => format!("search   sigma {:.3}", summary.mutation_sigma),
        (stagnation, 0) => format!(
            "search   sigma {:.3}   still for {stagnation}",
            summary.mutation_sigma
        ),
        (stagnation, immigrants) => format!(
            "search   sigma {:.3}   still for {stagnation}   +{immigrants} reheated genomes",
            summary.mutation_sigma
        ),
    }
}

/// A reading in a run's panel: the number, what it counts, and the colour it is worth.
///
/// Split rather than one pre-formatted string because the two halves are set differently — the
/// number in the big monospace the window reads numbers in, the unit small and dim beside it — and a
/// reading the drawing code has to take apart again to lay out was never a string in the first place.
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub value: String,
    pub unit: String,
    pub colour: Color32,
}

/// The type size a reading in a side panel is set in.
///
/// Bigger than [`theme::MONO_SIZE`], because a reading is what the panel is *for* and it is read from
/// a step back, and smaller than [`theme::HEADING_SIZE`], which is the weight the level's own name
/// carries. Sized here rather than in the theme because nothing else in the window is set in it.
const HERO_FONT: f32 = 16.0;

/// The plate the two pace buttons are: square enough to read as a stepper's ends, and shorter than a
/// menu row because it sits *above* the menu rather than in it.
const SPEED_STEP: Vec2 = Vec2::new(30.0, 26.0);

/// Which rung of [`SPEEDS`] `speed` is on, or the nearest one.
///
/// The ladder is the whole of what a viewer can reach — a slider, a button and a key all land on a
/// rung — so a speed off the ladder can only come from a build that changed the list under a run.
/// Snapping here rather than trusting the state is what keeps the two pace buttons' enabled-ness
/// (and the reading beside them) from describing a rung that does not exist.
/// The pace stepper: `− 8x +`, reporting one rung of the ladder or `0` for a press that has
/// nowhere to go.
///
/// **One control in both panels**, because both of them have a pace: the run's is the rate the
/// population is animated at and the replay's is the rate the episode is played at, and a viewer who
/// has learned the stepper beside a run should find the same one beside an episode. Which clock a
/// rung moves is the window's business (`Command::Speed` decides, and both clocks are one command
/// with one meaning per subject), so nothing here has to know which of the two it is drawing.
///
/// Disabled at the ends rather than stepped past them, so the two plates say the whole of the
/// ladder's range without a viewer having to press one to find out — and the lit state and the
/// request come from [`pace_step`], so a plate cannot be lit while asking for nothing, or quiet
/// while it would move the pace.
fn pace_row(ui: &mut egui::Ui, atlas: Option<&Atlas>, current: u32, unit: &str) -> i32 {
    let mut step = 0;
    let rung = speed_rung(current);
    ui.horizontal(|ui| {
        let down = pace_step(rung, false);
        if ui::icon_button(ui, atlas, None, "\u{2212}", SPEED_STEP, false, down != 0)
            .on_hover_text("one rung slower — the minus key")
            .clicked()
        {
            step = down;
        }
        ui.label(
            egui::RichText::new(format!("{}x", SPEEDS[rung]))
                .monospace()
                .color(theme::TEXT),
        );
        let up = pace_step(rung, true);
        if ui::icon_button(ui, atlas, None, "+", SPEED_STEP, false, up != 0)
            .on_hover_text("one rung faster — the plus key")
            .clicked()
        {
            step = up;
        }
    });
    // The description goes *under* the stepper rather than beside it. Beside it, the replay's
    // "the pace it plays at, in real time" made the row wider than the panel, and a panel that is
    // sized to its content grows to fit the row; the canvas is then laid out against the width the
    // panel reserved a frame earlier and the two disagree. Under it, the phrase wraps in the column
    // like every other sentence in the panel, and the two buttons stay a stepper instead of being
    // pushed apart by the words between them.
    small_dim(ui, unit);
    step
}

/// What one of the two pace buttons asks for, from the rung the run is on.
///
/// `0` at the end of the ladder rather than a step the run's own rule would refuse, which is what
/// makes the button's enabled state and its request the same fact. Pure, so the ends are a test
/// rather than something a viewer discovers by pressing a button that does nothing.
pub fn pace_step(rung: usize, up: bool) -> i32 {
    if up {
        if rung + 1 < SPEEDS.len() {
            1
        } else {
            0
        }
    } else if rung > 0 {
        -1
    } else {
        0
    }
}

fn speed_rung(speed: u32) -> usize {
    SPEEDS.iter().position(|s| *s == speed).unwrap_or_else(|| {
        SPEEDS
            .iter()
            .enumerate()
            .min_by_key(|(_, s)| s.abs_diff(speed))
            .map(|(i, _)| i)
            .unwrap_or(0)
    })
}

/// The four things the run's panel says, and no more.
///
/// A pure function of the summary, so what the panel *claims* can be checked without a window — the
/// same split [`chart_geometry`] and [`search_line`] use. Four, because a panel beside a level is
/// glanced at rather than read: where the run has got to, whether anything has reached the flag,
/// **how quickly the flag was reached**, and how good the best agent is. Everything else the window
/// knows about the search is one press away in the Details window ([`lab_details`]), and every line
/// kept here is one more thing between the eye and the answer to "is it getting anywhere?".
///
/// The clock is here rather than in the board because it is the one reading a viewer *acts* on: it is
/// what they take to the playtest to race, and a time a person has to open a window to find is a time
/// nobody races. It is a fourth reading rather than a replacement for any of the three, because the
/// three answer "is it learning" and this one answers "how fast is it" — a run can be at generation
/// 4 or generation 400 with the same best time.
///
/// The flag reading is the one that changes colour. A run nobody has solved yet is a standing fact
/// about the level; a run that has been solved is the run's *answer*, and the same sentence in the
/// growth colour is what says which of the two a viewer is looking at. The clock follows the same
/// rule for the same reason: no win, no time — and the reading says so in dim ink rather than
/// inventing a number.
pub fn menu_readings(summary: &RunSummary) -> [Reading; 4] {
    let generation = match summary.target {
        Some(target) => format!("{} / {}", summary.generation + 1, target),
        None => (summary.generation + 1).to_string(),
    };
    [
        Reading {
            value: generation,
            unit: if summary.target.is_some() {
                "generations".to_string()
            } else {
                "generation".to_string()
            },
            colour: TEXT,
        },
        Reading {
            // A count of zero is the reading, spelled as the number rather than as "none": it is the
            // same shape as the solved case, so the line is comparable from generation to generation
            // and a run's first win is the same three characters changing colour.
            value: format!("{} / {}", summary.wins, summary.pop_size),
            // The strict reading is named only when it differs, which is exactly when the level
            // draws its world per episode: "3 / 200 reached the flag" and "1 of them on every
            // trial" are two different claims, and the second is the one that survives the next
            // patrol phase. On a deterministic level the clause never appears.
            unit: if summary.solved < summary.wins {
                format!("reached the flag — {} on every trial", summary.solved)
            } else {
                "reached the flag".to_string()
            },
            colour: if summary.solved() { GOOD } else { TEXT_DIM },
        },
        Reading {
            // The clock, from the run's own record and through the sim's own formatter, so the
            // number beside the panel is the number the episode was scored on and the number the
            // playtest races. An em dash rather than a zero, because no win is not a time of 0.00s.
            value: match summary.fastest {
                Some(fast) => sim::format_time(fast.frames),
                None => "\u{2014}".to_string(),
            },
            unit: if summary.fastest.is_some() {
                "the quickest win".to_string()
            } else {
                "nothing has reached the flag yet".to_string()
            },
            colour: if summary.fastest.is_some() {
                GOOD
            } else {
                TEXT_DIM
            },
        },
        Reading {
            value: format_fitness(summary.max_fitness),
            unit: "best fitness".to_string(),
            colour: TEXT,
        },
    ]
}

/// One reading, drawn: the number big, its unit small and dim after it.
fn reading_line(ui: &mut egui::Ui, reading: &Reading) {
    // Wrapped, not a plain row: a reading with a long qualifier — "4 / 200 reached the flag — 1 on
    // every trial" is the one that does it — was wider than the panel, and a row that cannot wrap
    // makes the *panel* wider instead. egui sizes a side panel to its content, so the panel grew
    // past the window, the canvas was laid out against the width it had reserved a frame earlier,
    // and the two disagreed: the level painted over the panel's left edge while the panel's text
    // ran off the screen. Letting the qualifier fall under the number keeps both inside the width
    // the panel asked for.
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(reading.value.clone())
                .font(FontId::new(HERO_FONT, egui::FontFamily::Monospace))
                .color(reading.colour),
        );
        ui.label(
            egui::RichText::new(reading.unit.clone())
                .small()
                .color(theme::TEXT_DIM),
        );
    });
}

/// The run's panel: four readings, the trend in a word, and the four things a viewer does to a run.
///
/// It used to be the whole instrument board — the step the search is taking and how long the best has
/// stood still, the diversity the archive has covered, the x the population keeps dying at, a row for
/// every agent in the generation, the recorded champions, two ladders and three checkboxes — which
/// came to thirty-odd lines of type beside a level once a run had any age on it. Every one of those
/// numbers is real and every one of them was in the way: read at a glance, a panel that says
/// everything says nothing, and the person it was written for is watching a population learn to jump,
/// not auditing a search.
///
/// So it is a menu. What stays is the four readings ([`menu_readings`]) and the trend word, which
/// together answer the question a viewer asks of a run in progress, and the controls a person presses
/// while watching it — pause, one generation, keep the champion, start again — drawn as
/// [`ui::menu_row`]s with their pictures down the left edge, which is the shape a game pauses into.
/// Everything else moved one press away into the Details window ([`lab_details`]), where a person
/// goes *looking* for a number instead of being recited at: nothing was deleted, it was put where it
/// is asked for.
///
/// Two things that are conditions rather than readings are still said here, because a viewer has to
/// be *told* them rather than shown them: that this run is training on a level that has been edited
/// since, and that it has stopped. Both cost the panel nothing while they are false.
///
/// **Nothing here changes the view.** Leaving the run for the replay is the dock's switch, whose
/// replay cell is disabled until there is an agent to watch; a panel that offered the same move as
/// well would be one more control on the one surface that is trying to be quiet.
pub fn lab_panel(
    ui: &mut egui::Ui,
    atlas: Option<&Atlas>,
    summary: &RunSummary,
    view: &mut ViewState,
    level_changed: bool,
) -> Actions {
    let mut actions = Actions::default();

    // --- where the run is -------------------------------------------------
    section(ui, "The run");
    for reading in menu_readings(summary) {
        reading_line(ui, &reading);
    }
    // The trend is `render`'s reading of the same history the chart draws, so it is never a phrase
    // invented here — and it is the panel's only piece of prose, which is what makes it read as a
    // verdict rather than as one more number.
    let trend_colour = match summary.trend {
        Trend::Solved => GOOD,
        Trend::Stalled | Trend::Degenerating => WARN,
        _ => TEXT_DIM,
    };
    ui.colored_label(trend_colour, summary.trend.label());

    if level_changed {
        ui.add_space(4.0);
        ui.colored_label(
            WARN,
            "the level has been edited since this run started — Restart trains on what you see",
        );
    }
    if summary.finished {
        ui.add_space(4.0);
        ui.colored_label(
            GOOD,
            format!(
                "the run is over after {} generations — Restart trains again",
                summary.generations_run
            ),
        );
    }

    // --- the pace the run plays at ------------------------------------------
    ui.add_space(6.0);
    // Here, on the panel, and not only in the board: the one setting a viewer changes *while*
    // watching is how fast the run plays — a population that wins in its first generation is over
    // before the eye has moved, and a stalled one is unwatchable at 1x. A ladder that lived only in
    // the Details window was a setting a person had to know was there; the keys reached it, but a
    // key is not a control, and the number was two hundred points from the menu it belonged to.
    actions.speed = pace_row(ui, atlas, view.speed, "the pace it plays at");

    // --- what to do about it ----------------------------------------------
    section(ui, "The controls");
    // The rows all take the panel's own width: a menu is a *column*, and a column of plates that each
    // sized themselves would read as separate buttons that happen to be near each other.
    let width = ui.available_width();
    let pause = ui::menu_row(
        ui,
        atlas,
        Some(if view.paused {
            ui::GLYPH_PLAY
        } else {
            ui::GLYPH_PAUSE
        }),
        if view.paused { "Resume" } else { "Pause" },
        width,
        view.paused,
        true,
    )
    .on_hover_text(if view.paused {
        "let the run go on — space"
    } else {
        "hold the run where it is — space"
    });
    if pause.clicked() {
        actions.pause = true;
    }
    if ui::menu_row(
        ui,
        atlas,
        Some(ui::GLYPH_STEP),
        "One generation",
        width,
        false,
        true,
    )
    .on_hover_text("run exactly one generation, then hold still — N")
    .clicked()
    {
        actions.step = true;
    }
    if ui::menu_row(
        ui,
        atlas,
        Some(ui::GLYPH_MEDAL),
        "Save champion",
        width,
        false,
        true,
    )
    .on_hover_text("write the best-ever genome to the champions directory — S")
    .clicked()
    {
        actions.save = true;
    }
    // **Continue from a champion.** Disabled rather than hidden when this level has no
    // champion yet: a control that appears only after the file exists is a control a viewer
    // never learns about, and the disabled row says what would make it usable.
    if ui::menu_row(
        ui,
        atlas,
        Some(ui::GLYPH_PLAY),
        "Continue champion",
        width,
        false,
        summary.champion_saved,
    )
    .on_hover_text(if summary.champion_saved {
        "start a new population from the champion saved for this level — C"
    } else {
        "no champion saved for this level yet — S saves the run's best ever"
    })
    .clicked()
    {
        actions.continue_champion = true;
    }
    if ui::menu_row(
        ui,
        atlas,
        Some(ui::GLYPH_RESTART),
        "Restart run",
        width,
        false,
        true,
    )
    .on_hover_text("a fresh population on the level as it is now")
    .clicked()
    {
        actions.restart = true;
    }
    if ui::menu_row(
        ui,
        atlas,
        Some(ui::GLYPH_INFO),
        "Details",
        width,
        false,
        true,
    )
    .on_hover_text("everything else about this run: the search, the field, the record")
    .clicked()
    {
        actions.details = true;
    }

    actions
}

/// The run's instrument board: every number about the search that is not one of the three readings
/// the panel keeps.
///
/// This is what the panel used to be, and it lives behind the Details window because a run holds two
/// kinds of number. The ones a viewer acts on without thinking are on the panel ([`lab_panel`]): how
/// far it has got, whether anything has won, how good the best is, and the four controls. The ones a
/// viewer goes *looking* for are here: the step the search is taking and how long the best has stood
/// still, how much of the behavioural grid the archive has covered, where the population keeps dying,
/// the champion's own record, a row for every agent in the generation, the recorded champions, and
/// the two ladders that tune it all. A person asking "why is it stuck" wants the lot; a person
/// watching wants none of it, and one panel cannot be both.
///
/// It reports the same [`Actions`] the panel does, minus the four the panel owns: the widgets say
/// what was pressed wherever they are drawn, and the window handles one vocabulary rather than one
/// per surface.
pub fn lab_details(ui: &mut egui::Ui, summary: &RunSummary, view: &mut ViewState) -> Actions {
    let mut actions = Actions::default();

    // --- the search -------------------------------------------------------
    section(ui, "The search");
    // Named, because this window floats over whatever is behind it: a board of numbers with nothing
    // saying whose they are is a board a viewer has to remember the context of.
    small_dim(ui, summary.level_name.clone());
    // What the search is doing to itself (see [`search_line`]).
    ui.colored_label(TEXT_DIM, search_line(summary));
    // The spread of the generation, and the reason it is here rather than on the panel: the best
    // fitness on its own cannot say whether the population moved *with* its best agent or whether one
    // lucky genome is carrying a field that has learned nothing. The average and the worst do, and
    // reading them side by side is how a viewer tells a run that is getting somewhere from a run that
    // rolled a six.
    ui.monospace(format!(
        "max {}   avg {}   min {}",
        format_fitness(summary.max_fitness),
        format_fitness(summary.avg_fitness),
        format_fitness(summary.min_fitness)
    ));
    if let Some(stall) = &summary.stall {
        ui.colored_label(WARN, stall.label());
        match summary.deaths_at {
            Some(tile) => small_dim(
                ui,
                format!(
                    "deaths cluster at x {tile} ({} of them) — the obstacle to fix",
                    summary.deaths_count
                ),
            ),
            None => small_dim(ui, "no death cluster inside the level to point at"),
        };
    }
    if summary.genius {
        ui.colored_label(WIN_TICK, "genius agent: max jumped by more than half");
    }
    // **A named find, on the panel as well as in the card (WP16).** The card is the moment; this
    // line is the record of it, and it stays for as long as the generation it belongs to does, so
    // a viewer who looks away during the card can still read what was found.
    if let Some(discovery) = &summary.discovery {
        ui.colored_label(WIN_TICK, format!("discovery: {}", discovery.name));
        small_dim(ui, discovery.line.clone());
        if summary.codex_len > 1 {
            small_dim(
                ui,
                format!("{} finds in this level's codex", summary.codex_len),
            );
        }
    }
    // The two grids, as rows rather than as another clause on the sentence above: they are readings
    // about how *spread out* the population is, which is a different question from how hard the
    // search is pushing, and a pair of numbers a viewer compares against each other belongs in a
    // column that lines up. The diversity denominator is the archive's *behavioural* cells and not
    // its total: the grid is 48 ways of moving × 5 finish bands, and a count out of the 240 would
    // report a run across a third of the behaviours as one eighth explored.
    if summary.archive_bits > 0 {
        ui::stat_row(
            ui,
            "ways of moving",
            format!(
                "{} / {} niches",
                summary.archive_bits,
                evo::ARCHIVE_BEHAVIOUR_CELLS
            ),
        );
        ui::stat_row(
            ui,
            "how quick it has won",
            format!(
                "{} / {} finish bands",
                summary.archive_time_bits,
                evo::ARCHIVE_TIME_BINS
            ),
        );
    }
    // And the trickle that keeps the search from closing on one niche, as a row of its own. It was
    // the last clause of [`search_line`] and it is the one number here that is about the *next*
    // population rather than about the archive: a fifth of the children bred this generation came
    // out of a niche rather than out of the ranking, and a run that did not say so would be leaving
    // that unexplained. Reported *because* it is quiet — it neither stops nor fires on a threshold.
    if summary.archive_children > 0 {
        ui::stat_row(
            ui,
            "children from the archive",
            summary.archive_children.to_string(),
        );
    }
    // The same row for Item 14, and for the same reason: a child that came out of a mistake is
    // not a perturbation of the population, it is a correction of one decision, and a viewer
    // looking at a generation that suddenly moved is owed the difference.
    if summary.repairs > 0 {
        ui::stat_row(ui, "children from a mistake", summary.repairs.to_string());
    }
    // The run's own account of the one detour it took, in the words `evo` gave it: the window has no
    // opinion about how the step went, it only has to say it happened.
    if let Some(note) = &summary.curriculum {
        ui.colored_label(TEXT, note.clone());
    }

    // --- what the generation learned --------------------------------------
    // Its own section between the search and the clock, because it is the bridge: the rows above
    // say what the search did to itself and the rows below time the one win it has. These say
    // *where the population actually got*, which is the reading a viewer needs when a score that
    // will not move is the only thing the chart can show. A maximum that is flat while the deaths
    // march one leg deeper is a run that is learning; the reverse is a run that is not, and no
    // single scalar distinguishes them.
    section(ui, "What it learned");
    let learning = &summary.telemetry;
    if learning.pop == 0 {
        small_dim(ui, "waiting for the first scored generation");
    } else {
        ui::stat_row(
            ui,
            "solved this generation",
            format!(
                "{} / {} ({:.0}%)",
                learning.solved,
                learning.pop,
                100.0 * learning.solved as f32 / learning.pop as f32
            ),
        );
        ui::stat_row(ui, "clock, best / median", learning.clock_line());
        if learning.legs > 0 {
            ui::stat_row(
                ui,
                "best genome crossed",
                format!("{} / {} legs", learning.best_end_leg, learning.legs),
            );
        }
        // The end, not only the score: `died in leg 3 at frame 412 (decision 103)` is a place a
        // viewer can point at, and the panel has the room for the whole clause.
        ui::stat_row(ui, "best genome", learning.best_end_line());
        ui::stat_row(
            ui,
            "first-action spread",
            format!(
                "{:.0}% of the table ({:.2} bits)",
                learning.spread() * 100.0,
                learning.first_action_entropy
            ),
        );
        // The census, one row per leg anybody ended on. Legs nobody ended on are the stretches the
        // population walks through, and printing twelve rows of zeroes to say that would bury the
        // two places it does not. `ended` is every non-win; `died` is the sim's own verdict, so a
        // timed-out or stuck episode is counted in the first and not the second.
        for (leg, ended, died, through) in learning.leg_rows() {
            if ended == 0 && died == 0 {
                continue;
            }
            let label = format!("leg {leg}");
            ui::stat_row(
                ui,
                &label,
                format!("{ended} ended ({died} died), {through} crossed"),
            );
        }
    }

    // --- the clock ---------------------------------------------------------
    // Its own section, because the clock is its own question: everything above is about the search
    // and everything below is about agents, and "how fast has this been solved" is neither — it is
    // the number a viewer takes to the playtest, and the one number on this board they act on.
    section(ui, "The clock");
    match summary.fastest {
        Some(fast) => {
            ui::stat_row(
                ui,
                "quickest win",
                format!("{} ({} frames)", sim::format_time(fast.frames), fast.frames),
            );
            // The budget beside the time, because a time on its own is not a statement about
            // anything: 8.42 s is quick on a level that allows 40 s and impossible on one that
            // allows 4. Both numbers come from the same episode and the same level
            // (`sim::frame_budget`), so they cannot be about different terrains — and each is a row
            // of its own rather than one bracketed aside, because that aside ran off the end of the
            // column at every level whose budget is not a round number of seconds.
            ui::stat_row(ui, "the level allows", sim::format_time(fast.budget_frames));
            ui::stat_row(
                ui,
                "of it unspent",
                sim::format_time(fast.budget_frames.saturating_sub(fast.frames)),
            );
            // The fraction of the speed bonus the win actually collected, printed from the record's
            // own method rather than recomputed here: a board that did its own division could
            // disagree with the search about how quick the win it is describing was. The *bonus* is
            // named as well, because a percentage of an unnamed number is not a reading.
            ui::stat_row(
                ui,
                "pace paid",
                format!(
                    "{:.0}% of the {} speed bonus",
                    fast.spare() * 100.0,
                    format_fitness(evo::SPEED_BONUS)
                ),
            );
            ui::stat_row(
                ui,
                "whose win",
                format!("gen {} id {}", fast.generation, fast.genome_id),
            );
        }
        None => small_dim(
            ui,
            "no win timed yet — the clock starts when an agent reaches the flag",
        ),
    }

    // --- the best agent ---------------------------------------------------
    section(ui, "The best agent so far");
    match &summary.best {
        Some(best) => {
            // A board rather than a sentence, and one that says the clock: the same episode the
            // section above times is this one, and a reader comparing the two should be able to
            // see that they are the same run without doing arithmetic on the frame count.
            ui::stat_row(ui, "generation", best.generation.to_string());
            ui::stat_row(ui, "genome", best.genome_id.to_string());
            ui::stat_row(ui, "fitness", format_fitness(best.fitness));
            ui::stat_row(
                ui,
                "episode",
                format!(
                    "{} in {} ({} frames)",
                    best.outcome,
                    sim::format_time(best.frames),
                    best.frames
                ),
            );
            ui::stat_row(
                ui,
                "reached",
                format!(
                    "x {:.1}, {:.1} tiles from the flag",
                    best.best_x, best.distance_to_goal
                ),
            );
        }
        // A run's first generation is always scored, so this is the moment before the first packet
        // rather than a state a run can sit in.
        None => small_dim(ui, "nothing scored yet"),
    }

    // --- the field --------------------------------------------------------
    section(ui, "The field");
    small_dim(
        ui,
        "the generation on screen: the champion solid, the rest ghosted, and the swatch is the \
         agent's own outline colour",
    );
    // In a scroller with a ceiling, because this is the one list on the board whose length is a
    // property of the run's configuration rather than of the window: a population of two hundred
    // ghosts is a panel taller than the screen, and a window that grew to show it would put its own
    // close button off the top of the display.
    egui::ScrollArea::vertical()
        // Eight rows and no half of one: a list cut through the middle of a line reads as a layout
        // that ran out of room rather than as a list that scrolls.
        .max_height(140.0)
        .show(ui, |ui| {
            for lane in &summary.lanes {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(2),
                        Color32::from_rgb(lane.colour[0], lane.colour[1], lane.colour[2]),
                    );
                    let colour = if lane.rank == 0 { TEXT } else { TEXT_DIM };
                    ui.colored_label(
                        colour,
                        format!(
                            "{}  id {}  {}{}",
                            lane.name,
                            lane.genome_id,
                            format_fitness(lane.fitness),
                            if (lane.mean_fitness - lane.fitness).abs() > 0.005 {
                                format!("  mean {}", format_fitness(lane.mean_fitness))
                            } else {
                                String::new()
                            }
                        ),
                    );
                });
            }
        });

    // --- the record -------------------------------------------------------
    section(ui, "Recorded champions");
    if summary.recorded.is_empty() {
        small_dim(
            ui,
            "nothing recorded yet — a route is kept when the best improves",
        );
    } else {
        // Walked by **generation**, never by index into the record: the log drops its oldest entry
        // when it fills, which shifts every index by one and would silently re-point the cursor at a
        // route the viewer never chose.
        let generations: Vec<u32> = summary.recorded.iter().map(|r| r.generation).collect();
        let current = view
            .scrub
            .and_then(|g| generations.iter().position(|x| *x == g));

        // Named rather than drawn as `<` and `>`: the two are the same control the arrow keys are,
        // and a pair of angle brackets is a notation a viewer has to work out rather than read.
        let half = ((ui.available_width() - 8.0) * 0.5).max(72.0);
        ui.horizontal(|ui| {
            if ui::menu_row(
                ui,
                None,
                Some(ui::GLYPH_UNDO),
                "Older",
                half,
                false,
                current.is_some(),
            )
            .clicked()
            {
                actions.scrub_delta = -1;
            }
            if ui::menu_row(
                ui,
                None,
                Some(ui::GLYPH_REDO),
                "Newer",
                half,
                false,
                current.is_some(),
            )
            .clicked()
            {
                actions.scrub_delta = 1;
            }
        });
        if ui::menu_row(
            ui,
            None,
            Some(ui::GLYPH_PLAY),
            "Back to live",
            ui.available_width(),
            view.scrub.is_none(),
            view.scrub.is_some(),
        )
        .clicked()
        {
            actions.resume = true;
        }

        let mut index = current.unwrap_or(generations.len() - 1);
        let changed = ui
            .add(egui::Slider::new(&mut index, 0..=generations.len() - 1).text("route"))
            .changed();
        if changed {
            // Dragging the slider *is* the viewer choosing a generation.
            view.scrub = Some(generations[index]);
        }

        match view.scrub.and_then(|g| {
            summary
                .recorded
                .iter()
                .find(|r| r.generation == g)
                .map(|r| (g, r))
        }) {
            Some((generation, record)) => {
                ui.colored_label(
                    SCRUB_LINE,
                    format!(
                        "showing recorded generation {} ({}, fitness {}) — not the live run",
                        generation,
                        if record.won {
                            "reached the flag"
                        } else {
                            "died"
                        },
                        format_fitness(record.fitness)
                    ),
                );
            }
            None => small_dim(ui, "live: the canvas is drawing this generation's agents"),
        }
    }

    // --- watching and tuning ----------------------------------------------
    section(ui, "Watching");
    // The pace is *read* here and *set* on the panel, which is the split this board exists for: a
    // setting changed while watching the level belongs under the eye, and a fact about the run is
    // looked up in the window. Reported rather than repeated as a second slider, so the two surfaces
    // cannot disagree about what the run is playing at.
    ui::stat_row(ui, "pace", format!("{}x", view.speed));
    // The ladder is a slider over an index rather than a raw value, so every value the mouse can
    // reach is a rung the terminal's keys can also reach (§10 Tier 2.9).
    let mut mut_index = MUTATION_RATES
        .iter()
        .position(|r| (*r - view.mutation_rate).abs() < 1e-6)
        .unwrap_or(1);
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut mut_index, 0..=MUTATION_RATES.len() - 1)
                .text("mutation")
                .show_value(false),
        );
        reading(ui, format!("{:.2}", MUTATION_RATES[mut_index]), "sigma");
    });
    view.mutation_rate = MUTATION_RATES[mut_index];

    ui.checkbox(&mut view.show_deaths, "show death marks");
    // The checkbox reports the intent and the window flips the state, exactly as the panel's rows do:
    // a widget that changed it here as well would double-toggle.
    let mut follow = view.follow;
    if ui
        .checkbox(&mut follow, "follow the leader")
        .on_hover_text("follow the leading agent, or show the whole level")
        .changed()
    {
        actions.follow = true;
    }

    ui.add_space(8.0);
    // The key list lives on the board rather than on the panel: it is the one thing here that is
    // *about* the controls instead of being one, and a viewer reading it is already looking for how
    // to drive the run.
    small_dim(
        ui,
        "space pause   N one generation   S save   +/- speed   M/m mutation   F follow   \
         ←/→ walk the record   V watch the best",
    );

    actions
}
/// What to say when the ghost limit left agents out of the field, if it did.
///
/// `population.size` is everything the generation held and `ghosted` is what is drawn, so
/// the two differ only when the roster was capped. The comparison allows for the slot the
/// champion occupies in the population when it is in it: at exactly the limit nothing is
/// left out, and a line saying otherwise would describe a cap nobody hit.
fn capped_roster_note(population: Population, ghosted: usize) -> Option<String> {
    (population.size > ghosted + 1).then(|| {
        format!(
            "only the best {ghosted} of the generation's {} agents are ghosted, so the \
             field stays readable",
            population.size
        )
    })
}

/// The right-hand panel while a replay is on screen: whose episode it is, how it is
/// going, and the playback controls.
///
/// It reads the replay rather than the run for the same reason the canvas does — the run
/// is parked behind this view and none of its numbers describe the episode on screen. The
/// one thing the two modes share is the camera, which is why `follow` still arrives from
/// the view state: following is a rule about where to point, not about what is running.
/// The three things the replay's panel says: whose episode it is, and how it is going.
///
/// The same shape and the same restraint as the run's ([`menu_readings`]): a replay is *watched*, and
/// what a viewer looking at a running agent needs is which agent it is and what it has scored so far.
/// Everything else about the replay — the record it came from, the field of ghosts behind it, the
/// legend for their colours — is on the canvas in front of them, at the size of the window, in the
/// palette those colours are drawn in.
pub fn replay_readings(replay: &Replay) -> [Reading; 3] {
    let star = replay.star();
    let finished = replay.star_finished();
    let outcome = replay.star_sim().outcome();
    [
        Reading {
            value: (star.generation + 1).to_string(),
            unit: "the generation it came from".to_string(),
            colour: TEXT,
        },
        Reading {
            value: format!("id {}", star.genome_id),
            // Which of the run's two records this agent is, in the same words the strip and the
            // details board use ([`StarBasis::label`]): what is on screen is the run's *quickest
            // win*, which is not the champion — the champion is the top-scoring genome, and on a
            // run with two winners that is frequently the slower of the two. A viewer who has read
            // the label can make sense of the field behind it; one who has not would reasonably
            // ask why the best agent of the run is being overtaken.
            unit: star.basis.label().to_string(),
            colour: TEXT,
        },
        Reading {
            // The colour is the episode's own verdict: the growth colour once it is through the
            // flag, the warning colour once it has stopped short of it, and plain ink while it is
            // still running — which is the one case where the reading is not an answer yet.
            value: if finished {
                outcome.as_str().to_string()
            } else {
                "running".to_string()
            },
            // The star's own clock, not the field's ([`Replay::star_elapsed`]): a won episode holds
            // the time it won in while the ghosts behind it are still out there. The field's clock
            // is the animation's, and reading a time off it made the number beside "won" climb for
            // as long as the replay was left open.
            unit: format!(
                "{:.1}s, score {}",
                replay.star_elapsed(),
                format_fitness(replay.star_score())
            ),
            colour: if !finished {
                TEXT
            } else if outcome == Outcome::Won {
                GOOD
            } else {
                WARN
            },
        },
    ]
}

/// The replay's panel: the same shape as the run's, and quieter still.
///
/// It used to be a page of prose — whose episode this is, what it scored, the generation the ghosts
/// came from, how many of them were still going and how many were at the flag, a legend for the four
/// ranks on the canvas, a note about the ghost cap, and the key list. The canvas answers most of that
/// by being looked at; what belongs beside a replay is the transport, and the rest is one press away
/// in the Details window ([`replay_details`]).
///
/// The ghosts and the camera are rows rather than checkboxes: a row that says what it does whether or
/// not it is on, and turns gold when it is, is the same control as the dock's tools — and it is what
/// the other panel in this mode is made of, so the two views of a run share one vocabulary.
pub fn replay_panel(
    ui: &mut egui::Ui,
    atlas: Option<&Atlas>,
    replay: &mut Replay,
    view: &ViewState,
) -> Actions {
    let mut actions = Actions::default();

    for reading in replay_readings(replay) {
        reading_line(ui, &reading);
    }

    ui.add_space(8.0);
    let width = ui.available_width();
    let pause = ui::menu_row(
        ui,
        atlas,
        Some(if replay.paused {
            ui::GLYPH_PLAY
        } else {
            ui::GLYPH_PAUSE
        }),
        if replay.paused { "Play" } else { "Pause" },
        width,
        replay.paused,
        true,
    )
    .on_hover_text(if replay.paused {
        "let the episode run — space"
    } else {
        "hold the episode where it is — space"
    });
    if pause.clicked() {
        actions.pause = true;
    }
    if ui::menu_row(
        ui,
        atlas,
        Some(ui::GLYPH_RESTART),
        "Again",
        width,
        false,
        true,
    )
    .on_hover_text("run these episodes again, from the first tick — R")
    .clicked()
    {
        actions.restart = true;
    }
    // The ghost icon is a walker out of the game's own art, which is what a ghost *is*: another agent
    // of the generation, drawn in its own outline colour.
    if ui::menu_row(
        ui,
        atlas,
        Some("walker_a"),
        "Ghosts",
        width,
        replay.ghosts,
        true,
    )
    .on_hover_text("show the rest of the generation, each replaying its own episode — G")
    .clicked()
    {
        actions.ghosts = true;
    }
    if ui::menu_row(
        ui,
        atlas,
        Some("player_run_1"),
        "Follow the star",
        width,
        view.follow,
        true,
    )
    .on_hover_text("stay on the agent being replayed, or show the whole level — F")
    .clicked()
    {
        actions.follow = true;
    }
    if ui::menu_row(
        ui,
        atlas,
        Some(ui::GLYPH_INFO),
        "Details",
        width,
        false,
        true,
    )
    .on_hover_text("whose record this is, the field behind it, and how fast it plays")
    .clicked()
    {
        actions.details = true;
    }

    // The same stepper the run's panel has, on the same ladder: an episode and a run are two things
    // watched at a pace, and the one control for it belongs where the transport is. The replay's rung
    // is named as real time when it is, because that is the only rung of the ladder whose *meaning*
    // is not just its number.
    ui.add_space(6.0);
    actions.speed = pace_row(
        ui,
        atlas,
        replay.speed,
        if replay.speed == crate::replay::REAL_TIME {
            "the pace it plays at, in real time"
        } else {
            "the pace it plays at"
        },
    );

    actions
}

/// The replay's instrument board: the record this episode is, the field behind it, and its clock.
///
/// The half of the old replay panel that a viewer *looks up*. Two of these readings are a check
/// rather than a fact about the run — the episode as it was recorded beside the episode as it is
/// being replayed, which should be the same number twice — and a check belongs where a person goes on
/// purpose rather than in the corner of their eye.
pub fn replay_details(ui: &mut egui::Ui, replay: &mut Replay) -> Actions {
    // No actions, in fact: everything this board *controls* is a plate on the panel beside it, and
    // what is left here is read rather than pressed. The return type is the same one the panel uses
    // anyway, so the window that draws both handles one vocabulary instead of two.
    let actions = Actions::default();
    let star = replay.star().clone();

    // --- whose episode this is --------------------------------------------
    section(ui, "The episode");
    // Named, and not the level the editor is showing: an episode is scored on the run's terrain, so
    // an edit made since the run started puts a different level under the replay's canvas. Which one
    // a viewer is looking at is exactly the sort of thing a board is read to find out.
    small_dim(ui, format!("scored on {}", replay.level_name()));
    // Which record the episode is, in the words the strip and the panel use: the star is the run's
    // quickest win, which is *not* the champion — quickness is part of the fitness a champion is
    // picked on and not all of it — so a viewer who came here to ask why the field is overtaking the
    // agent on screen gets the answer in the first two rows rather than by inference.
    ui::stat_row(ui, "record", star.basis.label().to_string());
    if star.basis == StarBasis::BestScore {
        small_dim(
            ui,
            "the run has not won on this level yet, so this is the best-scoring agent it has \
             rather than a time to beat",
        );
    }
    ui::stat_row(ui, "generation", (star.generation + 1).to_string());
    ui::stat_row(ui, "genome", star.genome_id.to_string());
    // The recorded episode as *the file* has it, and then the episode as it is being replayed right
    // now — the same two rows, one under the other, which is the whole point of the pair: same
    // episode, same numbers if the replay is faithful, and a mismatch is a bug here rather than news
    // about the run. That is why they are printed together and why they were never two sections.
    ui::stat_row(
        ui,
        "recorded",
        format!(
            "{} in {} ({} frames)",
            star.outcome.as_str(),
            sim::format_time(star.frames),
            star.frames
        ),
    );
    ui::stat_row(ui, "score", format_fitness(star.fitness));
    // Live rather than recorded, and the *star's* frames rather than the field's clock: the two rows
    // above are the episode as the packet has it and these two are the episode as it is running, and
    // a time that kept counting past the flag would make the pair disagree about a replay that is
    // faithful.
    let live = replay.star_frames();
    let outcome = replay.star_sim().outcome();
    if replay.star_finished() {
        ui.colored_label(
            if outcome == Outcome::Won { GOOD } else { WARN },
            format!(
                "replayed: {} in {} ({} frames), score {}",
                outcome.as_str(),
                sim::format_time(live),
                live,
                format_fitness(replay.star_score())
            ),
        );
    } else {
        ui.colored_label(
            TEXT,
            format!(
                "replayed: {} in so far, score {}",
                sim::format_time(live),
                format_fitness(replay.star_score())
            ),
        );
    }

    // --- the ghosts -------------------------------------------------------
    section(ui, "The field");
    // Named, and named as a *different* generation from the star's: the field is the run's newest
    // scored generation and the agent above it is a record set back where it was made, which are two
    // numbers the moment the search has moved past it.
    small_dim(
        ui,
        format!(
            "the population of generation {}, each replaying its own episode",
            replay.population().generation + 1
        ),
    );
    ui::stat_row(ui, "ghosted", replay.ghost_count().to_string());
    ui::stat_row(ui, "still going", replay.ghosts_running().to_string());
    ui::stat_row(ui, "at the flag", replay.ghosts_won().to_string());
    if let Some(note) = capped_roster_note(replay.population(), replay.ghost_count()) {
        small_dim(ui, note);
    }
    // The legend for the colours on the canvas, in the canvas's own palette
    // (`view::cohort_swatch`): a field drawn in four tints and explained nowhere would be four tints
    // of noise, and the one thing a viewer cannot guess from the picture is which end of the rank
    // order a colour is. It is drawn whether or not the ghosts are showing — the bands are a fact
    // about the field rather than about the toggle.
    ui.add_space(4.0);
    small_dim(
        ui,
        "the field by rank, best first — each band fainter than the one above it",
    );
    for (band, ranks) in replay.cohorts() {
        ui.horizontal(|ui| {
            ui.colored_label(cohort_swatch(band), "\u{25a0}");
            reading(
                ui,
                format!("ranks {}–{}", ranks.start + 1, ranks.end),
                &format!("{} agents", ranks.len()),
            );
        });
    }
    if replay.all_finished() {
        // Every agent has stopped, so nothing is moving any more: a viewer who wants the field to run
        // again is told how, rather than watching a still frame wonder.
        ui.colored_label(
            theme::ACCENT_INK,
            "the whole field has finished — R runs it again",
        );
    }

    // --- the clock --------------------------------------------------------
    section(ui, "Playback");
    // Read here and *set* on the panel beside this window, the same split the run's board keeps: the
    // pace is changed while watching the episode, and this is where a viewer looks it up. Real time is
    // named as well as numbered, because it is the one rung whose meaning is not just its number.
    ui::stat_row(
        ui,
        "pace",
        if replay.speed == crate::replay::REAL_TIME {
            format!("{}x (real time)", replay.speed)
        } else {
            format!("{}x", replay.speed)
        },
    );

    // Reaching for the ghosts from *here* is reaching for a state the panel already shows as a plate,
    // so the only thing this needs to offer is the way back to the run — which is the dock's switch
    // too, and the reason this is a sentence rather than another row.
    ui.add_space(4.0);
    small_dim(
        ui,
        "space pause   R again   G ghosts   +/- speed   F follow   Esc or the switch: back to the run",
    );

    actions
}
/// The bottom panel: the fitness chart, with every generation that reached the flag
/// marked. `scrub` is the recorded generation being shown, when there is one.
pub fn chart_panel(ui: &mut egui::Ui, history: &[HistoryPoint], height: f32, scrub: Option<u32>) {
    let width = ui.available_width().max(200.0);
    let (frame, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(frame);
    // The strip is chrome — the same cream and gold lip as the panels either side of it — and
    // the chart is a *display* set into it: a dark screen with a rounded frame, drawn by the
    // one instrument in the window that is not a control. Everything from here on is inside
    // `rect`, which is that screen rather than the strip it sits in.
    painter.rect_filled(frame, CornerRadius::same(theme::RADIUS), theme::PANEL);
    let rect = frame.shrink(3.0);
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), PANEL_BG);
    painter.rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS),
        egui::Stroke::new(2.0_f32, theme::GOLD_EDGE),
        egui::StrokeKind::Inside,
    );

    // Monospace, because every label on this chart is a number: the axis values, the two
    // generation ticks and the count in the heading, and digits in a proportional face do
    // not line up in a column.
    let font = FontId::monospace(11.0);
    let heading = match history.last() {
        Some(last) => format!(
            "fitness — {} generations, latest max {} ({})",
            history.len(),
            format_fitness(last.max_fitness),
            if last.wins > 0 {
                "an agent reached the flag"
            } else {
                "no flag yet"
            }
        ),
        None => "fitness".to_string(),
    };
    painter.text(
        rect.left_top() + Vec2::new(8.0, 5.0),
        Align2::LEFT_TOP,
        heading,
        font.clone(),
        theme::ACCENT,
    );

    // An empty history gets a note, not axes: there is nothing to scale against, and an
    // empty box with labels on it reads as a bug.
    let plot = Rect::from_min_max(
        rect.min + Vec2::new(LABEL_W, LABEL_H + 12.0),
        rect.max - Vec2::new(8.0, LABEL_H),
    );
    if plot.width() <= 1.0 || plot.height() <= 1.0 {
        return;
    }
    painter.rect_filled(plot, CornerRadius::same(2), PLOT_BG);

    let geometry = chart_geometry(history, plot);
    if geometry.max.is_empty() && geometry.avg.is_empty() {
        painter.rect_stroke(
            plot,
            CornerRadius::same(2),
            Stroke::new(1.0_f32, AXIS),
            StrokeKind::Inside,
        );
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "no generation has been scored yet",
            font,
            SCREEN_TEXT,
        );
        return;
    }

    // The y rule: three lines, and the two numbers the series must be read against. The
    // **baseline is the frame's weight and the two rules above it are gridlines**: a chart whose
    // three rules are all the frame's colour has no floor, and one whose floor is a gridline has
    // no edge. That hierarchy is the whole of the drawing's depth.
    for (i, value) in [geometry.hi, (geometry.hi + geometry.lo) * 0.5, geometry.lo]
        .into_iter()
        .enumerate()
    {
        let y = plot.bottom() - plot.height() * (value - geometry.lo) / (geometry.hi - geometry.lo);
        let y = y.clamp(plot.top(), plot.bottom());
        let ink = if i == 2 { AXIS } else { GRID };
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, ink),
        );
        painter.text(
            Pos2::new(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format_fitness(value),
            font.clone(),
            SCREEN_TEXT_DIM,
        );
    }

    // A very quiet wash under the max series: the line says *where* the run has been and the
    // wash says *how much* of the plot it owns, which is the read a reader takes before reading
    // any number. Low enough alpha that the gridlines stay visible through it.
    if geometry.max.len() > 1 {
        // A quad per segment rather than one polygon: a fitness curve is not convex, and a fan
        // triangulation of one would paint over itself. Two triangles per point pair is exact
        // whatever the run's shape does.
        let tint = MAX_LINE.gamma_multiply(0.10);
        let mut mesh = egui::Mesh::default();
        for pair in geometry.max.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let base = mesh.vertices.len() as u32;
            mesh.colored_vertex(a, tint);
            mesh.colored_vertex(b, tint);
            mesh.colored_vertex(Pos2::new(b.x, plot.bottom()), tint);
            mesh.colored_vertex(Pos2::new(a.x, plot.bottom()), tint);
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base, base + 2, base + 3);
        }
        painter.add(mesh);
    }

    // Average first, so the max series is never hidden behind it.
    draw_series(&painter, &geometry.avg, AVG_LINE);
    draw_series(&painter, &geometry.max, MAX_LINE);

    // A win is the one event on this chart that is not a number, so it is marked on
    // the top rule where the eye can count how long ago it happened.
    for win in &geometry.wins {
        painter.line_segment(
            [
                Pos2::new(win.x, plot.top()),
                Pos2::new(win.x, plot.top() + 7.0),
            ],
            Stroke::new(2.0_f32, WIN_TICK),
        );
    }

    // The scrubbed generation, when the viewer is looking at the past.
    let scrubbed = scrub.filter(|generation| {
        *generation >= geometry.first_generation && *generation <= geometry.last_generation
    });
    if let Some(generation) = scrubbed {
        let span = geometry
            .last_generation
            .saturating_sub(geometry.first_generation);
        let x = if span == 0 {
            plot.center().x
        } else {
            let along = (generation - geometry.first_generation) as f32 / span as f32;
            plot.left() + plot.width() * along
        };
        painter.line_segment(
            [Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())],
            Stroke::new(1.5_f32, SCRUB_LINE),
        );
    }

    // The x rule: the run's own first and last generation, so the width means time.
    painter.text(
        Pos2::new(plot.left(), plot.bottom() + 3.0),
        Align2::LEFT_TOP,
        format!("gen {}", geometry.first_generation + 1),
        font.clone(),
        SCREEN_TEXT_DIM,
    );
    painter.text(
        Pos2::new(plot.right(), plot.bottom() + 3.0),
        Align2::RIGHT_TOP,
        format!("gen {}", geometry.last_generation + 1),
        font,
        SCREEN_TEXT_DIM,
    );
}

/// Draw one series: a polyline through its points, or a single dot when there is only
/// one generation so far. A one-point polyline would draw nothing at all, which on the
/// very first generation looks like a broken chart.
fn draw_series(painter: &egui::Painter, points: &[Pos2], colour: Color32) {
    match points {
        [] => {}
        [only] => {
            painter.circle_filled(*only, 2.5, colour);
        }
        _ => {
            painter.add(Shape::line(points.to_vec(), Stroke::new(1.6_f32, colour)));
        }
    }
}

/// Chart geometry, split out from the drawing so it can be tested without a window.
///
/// Points run left to right in generation order and the y range covers both series
/// with a little padding. Non-finite values are **skipped, never plotted**, which is
/// `render::chart`'s own rule: a NaN that reached the plot would silently rescale the
/// axis and hide the run that produced it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChartGeometry {
    pub max: Vec<Pos2>,
    pub avg: Vec<Pos2>,
    pub wins: Vec<Pos2>,
    pub lo: f32,
    pub hi: f32,
    pub first_generation: u32,
    pub last_generation: u32,
}

pub fn chart_geometry(history: &[HistoryPoint], plot: Rect) -> ChartGeometry {
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    let mut first: Option<u32> = None;
    let mut last: Option<u32> = None;
    let mut usable = 0usize;
    for point in history {
        if !(point.max_fitness.is_finite() || point.avg_fitness.is_finite()) {
            continue;
        }
        usable += 1;
        first.get_or_insert(point.generation);
        last = Some(point.generation);
        for value in [point.max_fitness, point.avg_fitness] {
            if value.is_finite() {
                lo = lo.min(value);
                hi = hi.max(value);
            }
        }
    }
    let (Some(first), Some(last)) = (first, last) else {
        return ChartGeometry::default();
    };
    if !lo.is_finite() || !hi.is_finite() {
        return ChartGeometry::default();
    }

    // A flat run is a real run (a stalled population in particular), so the span is
    // never zero: with one it would divide by zero, and with none the flat line would
    // sit on the axis where it reads as "zero fitness".
    let span = (hi - lo).max(1.0);
    let lo = lo - span * 0.08;
    let hi = hi + span * 0.08;
    let y = |value: f32| plot.bottom() - plot.height() * (value - lo) / (hi - lo);

    let mut geometry = ChartGeometry {
        lo,
        hi,
        first_generation: first,
        last_generation: last,
        ..ChartGeometry::default()
    };

    // **The chart is at most one point per pixel column.** A run of twelve thousand
    // generations has more points than the plot has pixels, so drawing them all costs
    // tessellation for points that land on the same column — and the chart is redrawn
    // every frame, which is how a long run came to make even a paused window lag. A
    // history that fits in the plot's width is drawn exactly as before; a longer one is
    // bucketed by column, keeping the **maximum** of each column for the max series (a
    // spike must survive the reduction) and the mean for the average.
    let columns = plot.width().floor().max(1.0) as usize;
    if usable <= columns {
        let x = |generation: u32| {
            if last > first {
                plot.left() + plot.width() * (generation - first) as f32 / (last - first) as f32
            } else {
                plot.center().x
            }
        };
        for point in history {
            if !(point.max_fitness.is_finite() || point.avg_fitness.is_finite()) {
                continue;
            }
            let at = |value: f32| {
                Pos2::new(
                    x(point.generation).clamp(plot.left(), plot.right()),
                    y(value).clamp(plot.top(), plot.bottom()),
                )
            };
            if point.max_fitness.is_finite() {
                geometry.max.push(at(point.max_fitness));
            }
            if point.avg_fitness.is_finite() {
                geometry.avg.push(at(point.avg_fitness));
            }
            if point.wins > 0 {
                // A win is a tick on the **top rule**, not a point at its own value: the chart
                // exists to show the learning arc, and a flag reached is an event to count
                // rather than a number to read off the axis. This is also where
                // `chart_panel` draws them, so the geometry and the drawing agree instead of
                // leaving the marker's y to mean nothing in particular.
                geometry.wins.push(Pos2::new(
                    x(point.generation).clamp(plot.left(), plot.right()),
                    plot.top(),
                ));
            }
        }
        return geometry;
    }

    #[derive(Clone, Default)]
    struct Column {
        max: f32,
        max_seen: bool,
        avg_sum: f64,
        avg_count: u32,
        won: bool,
    }
    let mut buckets = vec![Column::default(); columns];
    // Integer arithmetic, so a generation lands in one bucket and the buckets stay in
    // order: `(last - first) * columns` overflows u32 long before this does.
    let span_generations = last.saturating_sub(first) as u64;
    let width = columns as u64;
    for point in history {
        if !(point.max_fitness.is_finite() || point.avg_fitness.is_finite()) {
            continue;
        }
        let along = point.generation.saturating_sub(first) as u64;
        let column = if span_generations == 0 {
            0
        } else {
            ((along * width) / (span_generations + 1)).min(width - 1) as usize
        };
        let bucket = &mut buckets[column];
        if point.max_fitness.is_finite() && (!bucket.max_seen || point.max_fitness > bucket.max) {
            bucket.max = point.max_fitness;
            bucket.max_seen = true;
        }
        if point.avg_fitness.is_finite() {
            bucket.avg_sum += point.avg_fitness as f64;
            bucket.avg_count += 1;
        }
        bucket.won |= point.wins > 0;
    }
    for (column, bucket) in buckets.iter().enumerate() {
        if !bucket.max_seen && bucket.avg_count == 0 {
            continue;
        }
        let x = if columns > 1 {
            plot.left() + plot.width() * column as f32 / (columns - 1) as f32
        } else {
            plot.center().x
        };
        if bucket.max_seen {
            geometry
                .max
                .push(Pos2::new(x, y(bucket.max).clamp(plot.top(), plot.bottom())));
        }
        if bucket.avg_count > 0 {
            let avg = (bucket.avg_sum / bucket.avg_count as f64) as f32;
            geometry
                .avg
                .push(Pos2::new(x, y(avg).clamp(plot.top(), plot.bottom())));
        }
        if bucket.won {
            geometry.wins.push(Pos2::new(x, plot.top()));
        }
    }
    geometry
}

#[cfg(test)]
mod tests {
    use super::*;
    use evo::{default_shape, Genome};
    use render::trace::{ChampionTrace, TracePoint};

    fn history(values: &[(u32, f32, f32, usize)]) -> Vec<HistoryPoint> {
        values
            .iter()
            .map(|(generation, max, avg, wins)| HistoryPoint {
                generation: *generation,
                max_fitness: *max,
                avg_fitness: *avg,
                wins: *wins,
            })
            .collect()
    }

    fn lane(rank: usize, id: u32, fitness: f32) -> Lane {
        Lane {
            name: if rank == 0 {
                "champion".to_string()
            } else {
                format!("best {}", rank + 1)
            },
            best: true,
            rank,
            genome_id: id,
            fitness,
            mean_fitness: fitness,
            colour: [200, 200, 200],
            genome: Genome::zeros(id, default_shape()),
            seed: 7,
        }
    }

    fn trace(generation: u32, fitness: f32, won: bool) -> ChampionTrace {
        ChampionTrace {
            generation,
            max_fitness: fitness,
            won,
            points: vec![
                TracePoint {
                    frame: 0,
                    x: 1.0,
                    y: 5.0,
                },
                TracePoint {
                    frame: 300,
                    x: 55.0,
                    y: 5.0,
                },
            ],
        }
    }

    fn packet(history: Vec<HistoryPoint>, deaths: Vec<(f32, f32, u32)>, won: bool) -> Packet {
        Packet {
            generation: history.last().map(|h| h.generation).unwrap_or(0),
            max_fitness: history.last().map(|h| h.max_fitness).unwrap_or(0.0),
            avg_fitness: history.last().map(|h| h.avg_fitness).unwrap_or(0.0),
            min_fitness: -40.0,
            wins: if won { 3 } else { 0 },
            // A deterministic level's fixture: every genome that touched the flag also won every
            // trial of it, which is the invariant the panel's strict reading exists to keep.
            solved: if won { 3 } else { 0 },
            genius: false,
            pop_size: 200,
            // A quiet, fixed-sigma run: the panel's search line then has nothing to say
            // beyond the step itself, which is the common case a fixture should be.
            mutation_sigma: 0.30,
            stagnation: 0,
            immigrants: 0,
            // A run whose archive has filled a few niches: the fixture is a run that has been
            // going for a while, and a diversity reading of zero is the collapsed case rather
            // than the common one.
            archive_children: 4,
            repairs: 0,
            archive_bits: 9,
            // Two of the four finish bands, so the board's speed coverage is a count rather than a
            // zero: the fixture is a run that has been going for a while.
            archive_time_bits: 2,
            // And a timed win, because the clock is the one reading the panel gained: a fixture
            // without one would leave every assertion about it untested. 505 frames is 8.42 s of the
            // flat level's own 40 s budget.
            fastest: Some(evo::FastWin {
                generation: 4,
                genome_id: 40,
                frames: 505,
                budget_frames: 2400,
            }),
            // The episode that time came from. The board does not read it — the playtest does,
            // to race the ghost — but every `Packet` carries one when the run has won, so the
            // fixture does too rather than standing for a state no real run can be in.
            fastest_agent: None,
            lanes: vec![lane(0, 12, 431.2), lane(1, 40, 120.0)],
            // The population the replay ghosts. The chart tests are about the run's shape,
            // so the roster they never draw is empty here.
            agents: Vec::new(),
            history,
            deaths,
            traces: vec![trace(0, 10.0, false), trace(4, 411.0, won)],
            target: Some(300),
            level_name: "l1-flat-run".to_string(),
            level_hash: "test".to_string(),
            best: None,
            curriculum: None,
            finished: false,
            // The fixture is a run with no find in this generation and an empty codex; the
            // discovery line's own tests build their packet with both set.
            discovery: None,
            codex_len: 0,
            // A generation with a solved handful, a timed win and a population that ended
            // deepest on leg 2: the section only reads as intended when every one of its rows
            // has a number in it, so the fixture is a real census rather than a default.
            telemetry: evo::Telemetry {
                generation: 4,
                level: "l1-flat-run".to_string(),
                pop: 200,
                solved: if won { 3 } else { 0 },
                wins: if won { 3 } else { 0 },
                legs: 4,
                best_frames: if won { Some(505) } else { None },
                median_frames: if won { Some(620) } else { None },
                best_end_leg: if won { 4 } else { 2 },
                best_died: !won,
                best_end_frame: if won { None } else { Some(412) },
                best_end_decision: if won { None } else { Some(103) },
                ended_legs: vec![40, 25, 10, 2],
                died_legs: vec![12, 8, 4, 1],
                reached_legs: vec![160, 120, 60, 3],
                first_actions: [50, 35, 28, 22, 18, 14, 11, 9, 6, 4, 3],
                first_action_entropy: 3.06,
            },
        }
    }

    fn plot() -> Rect {
        Rect::from_min_max(Pos2::new(60.0, 40.0), Pos2::new(660.0, 200.0))
    }

    #[test]
    fn an_empty_history_has_no_geometry_and_does_not_panic() {
        let geometry = chart_geometry(&[], plot());
        assert_eq!(geometry, ChartGeometry::default());
        assert!(geometry.max.is_empty() && geometry.avg.is_empty());

        // The panel draws the placeholder rather than dividing by a zero range.
        draw_in_headless_context(&[], 120.0, None);
    }

    #[test]
    fn the_y_range_covers_both_series() {
        let points = history(&[
            (0, -12.0, -30.0, 0),
            (1, 400.0, 90.0, 0),
            (2, 411.0, 120.0, 0),
        ]);
        let geometry = chart_geometry(&points, plot());
        assert!(
            geometry.lo <= -30.0,
            "lo {} misses the average",
            geometry.lo
        );
        assert!(
            geometry.hi >= 411.0,
            "hi {} misses the maximum",
            geometry.hi
        );
        assert!(geometry.lo < geometry.hi);
    }

    #[test]
    fn the_best_generation_is_the_highest_point_on_the_chart() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 0), (2, 300.0, 50.0, 0)]);
        let geometry = chart_geometry(&points, plot());
        let highest = geometry
            .max
            .iter()
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .copied()
            .expect("three points");
        assert_eq!(highest.y, geometry.max[1].y);
        for other in &geometry.max {
            assert!(
                highest.y <= other.y,
                "a point sits above the best generation: {highest:?} vs {other:?}"
            );
        }
    }

    #[test]
    fn points_run_left_to_right_in_generation_order() {
        let points = history(&[(0, 10.0, 5.0, 0), (5, 40.0, 20.0, 0), (9, 411.0, 60.0, 0)]);
        let geometry = chart_geometry(&points, plot());
        assert_eq!(geometry.first_generation, 0);
        assert_eq!(geometry.last_generation, 9);
        for pair in geometry.max.windows(2) {
            assert!(pair[0].x < pair[1].x, "generations out of order: {pair:?}");
        }
        // The two series share the x axis, so the average lines up under the max.
        for (max, avg) in geometry.max.iter().zip(geometry.avg.iter()) {
            assert_eq!(max.x, avg.x);
        }
    }

    #[test]
    fn non_finite_values_are_skipped_rather_than_plotted() {
        let points = history(&[
            (0, 10.0, 5.0, 0),
            (1, f32::NAN, 6.0, 0),
            (2, 400.0, f32::INFINITY, 0),
        ]);
        let geometry = chart_geometry(&points, plot());
        assert_eq!(geometry.max.len(), 2, "the NaN maximum was plotted");
        assert_eq!(geometry.avg.len(), 2, "the infinite average was plotted");
        assert!(geometry
            .max
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite()));
        assert!(geometry.lo.is_finite() && geometry.hi.is_finite());
        // And a history that is *entirely* non-finite is not drawable at all.
        let poisoned = history(&[(0, f32::NAN, f32::NAN, 0)]);
        assert_eq!(chart_geometry(&poisoned, plot()), ChartGeometry::default());
    }

    #[test]
    fn a_single_generation_and_a_flat_run_are_both_drawable() {
        let one = chart_geometry(&history(&[(0, 7.0, 3.0, 0)]), plot());
        assert_eq!(one.max.len(), 1);
        assert_eq!(one.max[0].x, plot().center().x, "one point sits mid-plot");

        // A stalled run: identical fitness for twenty generations. The line must sit
        // inside the plot rather than collapse onto an axis with a zero span.
        let flat: Vec<(u32, f32, f32, usize)> = (0..20).map(|g| (g, 100.0, 100.0, 0)).collect();
        let geometry = chart_geometry(&history(&flat), plot());
        assert!(geometry.lo < 100.0 && geometry.hi > 100.0);
        for point in &geometry.max {
            assert!(
                point.y > plot().top() && point.y < plot().bottom(),
                "a flat series was drawn on the plot's edge: {point:?}"
            );
        }
        assert!(geometry.lo.is_finite() && geometry.hi.is_finite());
    }

    /// **A long run costs the chart its width, not its generation count.**
    ///
    /// The bug this pins: at ~12,000 generations the window was laggy even when paused,
    /// because every frame tessellated one point per generation plus a win tick for every
    /// winning generation. Measured in a fresh headless context in the release build, this
    /// fixture drew **177,308 vertices in 7.0 ms** before the reduction and **17,348 in
    /// 2.6 ms** after it. The chart cannot resolve more than one point per pixel column, so
    /// the rest was geometry the screen could not show. The reduced chart is bounded by
    /// [`plot`]'s width, keeps the maximum of each column (a spike is not smoothed away), and
    /// still reaches both ends.
    #[test]
    fn a_long_run_does_not_make_the_chart_expensive() {
        let mut points: Vec<HistoryPoint> = (0..12_000u32)
            .map(|g| HistoryPoint {
                generation: g,
                max_fitness: 100.0 + (g % 500) as f32,
                avg_fitness: 50.0 + (g % 200) as f32,
                wins: usize::from(g % 3 == 0),
            })
            .collect();
        // One spike, far into the run: the reduction must keep it.
        points[6_000].max_fitness = 10_000.0;

        let columns = plot().width().floor() as usize;
        let geometry = chart_geometry(&points, plot());
        assert!(geometry.max.len() <= columns, "max: {}", geometry.max.len());
        assert!(geometry.avg.len() <= columns, "avg: {}", geometry.avg.len());
        assert!(
            geometry.wins.len() <= columns,
            "wins: {}",
            geometry.wins.len()
        );
        assert_eq!(geometry.first_generation, 0);
        assert_eq!(geometry.last_generation, 11_999);
        assert_eq!(
            geometry.max.last().map(|p| p.x),
            Some(plot().right()),
            "the latest generation is still the right-hand edge"
        );
        let spike = plot().bottom()
            - plot().height() * (10_000.0 - geometry.lo) / (geometry.hi - geometry.lo);
        assert!(
            geometry.max.iter().any(|p| (p.y - spike).abs() < 0.1),
            "the spike was averaged away"
        );

        let started = std::time::Instant::now();
        let vertices = draw_in_headless_context(&points, 120.0, None);
        let elapsed = started.elapsed();
        assert!(
            vertices < 30_000,
            "12k generations drew {vertices} vertices for a ~900 px chart ({elapsed:?})"
        );
    }

    #[test]
    fn only_a_generation_that_reached_the_flag_gets_a_win_tick() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 1), (2, 411.0, 70.0, 2)]);
        let geometry = chart_geometry(&points, plot());
        assert_eq!(geometry.wins.len(), 2);
        assert_eq!(geometry.wins[0].x, geometry.max[1].x);
        assert_eq!(geometry.wins[1].x, geometry.max[2].x);
        for tick in &geometry.wins {
            assert_eq!(tick.y, plot().top(), "a win is marked on the top rule");
        }
    }

    #[test]
    fn a_generation_that_found_something_carries_it_onto_the_panel() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 1)]);
        let mut found = packet(points.clone(), Vec::new(), true);
        found.discovery = Some(evo::Discovery {
            name: "Corpse Bridge".to_string(),
            line: "chained three stomps without touching the ground".to_string(),
            kind: evo::DiscoveryKind::Technique,
            signature: "technique:Corpse Bridge".to_string(),
            level_name: "l1-flat-run".to_string(),
            level_hash: "test".to_string(),
            generation: 1,
            genome_id: 7,
            frames: 200,
            seed: 3,
            skip_percent: None,
            replay: Vec::new(),
        });
        found.codex_len = 4;
        let summary = RunSummary::new(&found, &points);
        assert_eq!(
            summary.discovery.as_ref().map(|d| d.name.as_str()),
            Some("Corpse Bridge")
        );
        assert_eq!(summary.codex_len, 4);
        // A rediscovery is not news: a packet with none leaves the panel's line absent.
        let quiet = RunSummary::new(&packet(points.clone(), Vec::new(), true), &points);
        assert!(quiet.discovery.is_none());
    }

    #[test]
    fn the_summary_reads_the_run_the_way_the_terminal_does() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 1)]);
        let summary = RunSummary::new(&packet(points.clone(), Vec::new(), true), &points);
        assert_eq!(summary.trend, fitness_trend(&points));
        assert_eq!(summary.stall, fitness_stall(&points));
        assert_eq!(summary.generations_run, 2);
        assert_eq!(summary.generation, 1);
        assert_eq!(summary.target, Some(300));
        assert!(summary.solved());
        // The recorded improvements arrive in generation order for the scrubber.
        assert_eq!(summary.recorded.len(), 2);
        assert_eq!(summary.recorded[0].generation, 0);
        assert_eq!(summary.recorded[1].generation, 4);
        // `best` and the death cluster need information the packet does not carry.
        assert!(summary.best.is_none());
        assert!(summary.deaths_at.is_none());
    }

    #[test]
    fn the_death_cluster_is_named_only_after_a_stall_and_only_inside_the_level() {
        // A stalled run: 30 generations at the same fitness, with the population dying
        // at tile 33 over and over, plus one corpse outside the level entirely.
        let flat: Vec<(u32, f32, f32, usize)> = (0..30).map(|g| (g, 100.0, 60.0, 0)).collect();
        let points = history(&flat);
        let deaths = vec![
            (33.0, 5.0, 28),
            (33.0, 5.0, 29),
            (33.0, 5.0, 29),
            (12.0, 5.0, 20),
            (900.0, 5.0, 29),
        ];
        let summary = RunSummary::new(&packet(points.clone(), deaths.clone(), false), &points);
        assert!(
            summary.stall.is_some(),
            "this run is flat for 29 generations"
        );
        let summary = summary.with_level(40);
        let expected = death_cluster(&deaths, 40).expect("a cluster");
        assert_eq!(summary.deaths_at, Some(expected.0));
        assert_eq!(summary.deaths_count, expected.1);
        assert_eq!(summary.deaths_at, Some(33));
        assert_eq!(
            summary.deaths_count, 3,
            "the mark outside the level is ignored"
        );

        // Before a stall the same deaths say nothing: nobody is stuck yet.
        let young = history(&[(0, 100.0, 60.0, 0), (1, 101.0, 61.0, 0)]);
        let summary = RunSummary::new(&packet(young.clone(), deaths, false), &young).with_level(40);
        assert!(summary.stall.is_none());
        assert!(summary.deaths_at.is_none());
    }

    #[test]
    fn the_curriculum_note_comes_from_the_packet_and_only_when_there_was_a_step() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 20.0, 9.0, 0)]);
        let plain = RunSummary::new(&packet(points.clone(), Vec::new(), false), &points);
        assert!(
            plain.curriculum.is_none(),
            "a run with no easier level has no step to report"
        );

        let note = "curriculum: 40 generations on l1-flat-run from the stalled population";
        let mut stepped = packet(points.clone(), Vec::new(), false);
        stepped.curriculum = Some(note.to_string());
        let summary = RunSummary::new(&stepped, &points);
        assert_eq!(summary.curriculum.as_deref(), Some(note));
        // And the window says it: a rescue a viewer could never read would be invisible, and the
        // board behind the panel is where a run's own account of itself is read.
        draw_details(&summary, &mut ViewState::default());
    }

    #[test]
    fn a_champions_remaining_distance_is_measured_in_tiles_not_in_fitness() {
        let won = BestLine::from_episode(4, 12, 431.2, "won", true, 210, 55.0, 58.0);
        assert_eq!(won.distance_to_goal, 0.0, "a won episode is at the flag");
        let died = BestLine::from_episode(7, 40, 120.0, "died", false, 190, 20.0, 58.0);
        assert_eq!(died.distance_to_goal, 38.0);
        // A run that overshot the flag (a stomp bounce can carry it past) reads as zero
        // rather than as a negative distance.
        let past = BestLine::from_episode(8, 41, 300.0, "died", false, 100, 70.0, 58.0);
        assert_eq!(past.distance_to_goal, 0.0);
    }

    #[test]
    fn the_search_line_says_the_step_and_nothing_it_has_not_earned() {
        // A run that is still finding new bests: the step, and no stall to report. This is
        // the default configuration's usual shape, since the reheated batch only ever arrives
        // after a plateau.
        let points = history(&[(0, 10.0, 5.0, 0), (4, 411.0, 60.0, 0)]);
        let quiet = RunSummary::new(&packet(points.clone(), Vec::new(), false), &points);
        assert_eq!(search_line(&quiet), "search   sigma 0.300");

        // A plateau, and then the plateau with the burst it bought.
        let mut stalled = summary_with_search(7, 0);
        assert_eq!(search_line(&stalled), "search   sigma 0.300   still for 7");
        stalled = summary_with_search(21, 8);
        assert_eq!(
            search_line(&stalled),
            "search   sigma 0.300   still for 21   +8 reheated genomes"
        );

        // The step is the run's own number, not a constant of the panel: a tuned run's reading
        // has to reach the line unchanged.
        let mut tuned = summary_with_search(0, 0);
        tuned.mutation_sigma = 0.091;
        assert_eq!(search_line(&tuned), "search   sigma 0.091");

        // And the two grid readings are *not* on it any more, however many niches the archive
        // holds: they are rows of the board (§[`lab_details`]), and a line that still carried them
        // would be the wall of prose this split removed. A run with a full archive says exactly the
        // same sentence as the quiet one.
        let mut full = summary_with_search(0, 0);
        full.archive_bits = 48;
        assert_eq!(search_line(&full), search_line(&quiet));
    }

    /// A summary whose only interesting fact is what the search is doing to itself.
    fn summary_with_search(stagnation: u32, immigrants: usize) -> RunSummary {
        let points = history(&[(0, 10.0, 5.0, 0), (4, 411.0, 60.0, 0)]);
        let mut summary = RunSummary::new(&packet(points.clone(), Vec::new(), false), &points);
        summary.stagnation = stagnation;
        summary.immigrants = immigrants;
        summary
    }

    #[test]
    fn the_scrubber_walks_generations_and_the_panel_can_leave_the_past() {
        let points = history(&[(0, 10.0, 5.0, 0), (4, 411.0, 60.0, 0)]);
        let summary = RunSummary::new(&packet(points.clone(), Vec::new(), false), &points);

        // Live: the board offers the way back in only when there is a past to leave, and with no
        // pointer anywhere near it a frame of the scrubber asks for nothing.
        let actions = draw_details(&summary, &mut ViewState::default());
        assert_eq!(actions, Actions::default());

        // Looking at a recorded generation: the note says so and Resume is offered.
        let mut view = ViewState {
            scrub: Some(0),
            ..ViewState::default()
        };
        let actions = draw_details(&summary, &mut view);
        assert_eq!(actions, Actions::default());
        assert_eq!(view.scrub, Some(0), "the widgets did not move the cursor");
    }

    /// Every documented key does something, does what its button does, and cannot reach a
    /// value the panel's own controls could not. This is the window's answer to the
    /// terminal's `every_documented_key_maps_to_an_action`, and it exists because the one
    /// thing a test cannot do here is press a real key in a real window.
    #[test]
    fn every_documented_key_maps_to_a_command() {
        let view = ViewState::default();
        let one = |keys: Keys| commands_for(keys, &view);

        assert_eq!(
            one(Keys {
                pause: true,
                ..Default::default()
            }),
            vec![Command::TogglePause]
        );
        assert_eq!(
            one(Keys {
                step: true,
                ..Default::default()
            }),
            vec![Command::StepOne]
        );
        assert_eq!(
            one(Keys {
                faster: true,
                ..Default::default()
            }),
            vec![Command::Speed(1)]
        );
        assert_eq!(
            one(Keys {
                slower: true,
                ..Default::default()
            }),
            vec![Command::Speed(-1)]
        );
        assert_eq!(
            one(Keys {
                live: true,
                ..Default::default()
            }),
            vec![Command::Live]
        );
        assert_eq!(
            one(Keys {
                follow: true,
                ..Default::default()
            }),
            vec![Command::ToggleFollow]
        );
        assert_eq!(
            one(Keys {
                save: true,
                ..Default::default()
            }),
            vec![Command::Save]
        );
        assert_eq!(
            one(Keys {
                continue_champion: true,
                ..Default::default()
            }),
            vec![Command::ContinueChampion]
        );
        assert_eq!(
            one(Keys {
                ghosts: true,
                ..Default::default()
            }),
            vec![Command::ToggleGhosts]
        );
        assert_eq!(
            one(Keys {
                older: true,
                ..Default::default()
            }),
            vec![Command::Scrub(-1)]
        );
        assert_eq!(
            one(Keys {
                newer: true,
                ..Default::default()
            }),
            vec![Command::Scrub(1)]
        );
        // An idle frame asks for nothing: the run must not drift while nobody is looking.
        assert!(one(Keys::default()).is_empty());

        // The mutation keys carry a rate one rung from the one on screen, so a key can only
        // reach a rate the slider can also reach.
        match one(Keys {
            mutate_up: true,
            ..Default::default()
        })
        .as_slice()
        {
            [Command::MutationRate(rate)] => assert_eq!(
                *rate,
                crate::lab::step_mutation_rate(view.mutation_rate, true)
            ),
            other => panic!("expected one mutation-rate command, got {other:?}"),
        }

        // Two keys on one frame both act, in the order this module documents.
        assert_eq!(
            commands_for(
                Keys {
                    newer: true,
                    pause: true,
                    ..Default::default()
                },
                &view
            ),
            vec![Command::TogglePause, Command::Scrub(1)]
        );
    }

    #[test]
    fn the_ladders_the_widgets_offer_are_the_ladders_the_run_understands() {
        // A value that is not on the rungs (a hand-edited config, a future key) is read as the
        // nearest rung rather than leaving the control unrepresentable.
        let mut view = ViewState {
            speed: 9,
            mutation_rate: 0.2,
            ..ViewState::default()
        };
        let points = history(&[(0, 10.0, 5.0, 0), (1, 20.0, 9.0, 0)]);
        let summary = RunSummary::new(&packet(points.clone(), Vec::new(), false), &points);

        // The pace stepper *reads* a rung rather than writing one — it reports a step and the window
        // applies it — so what it owes an off-ladder value is that the rung it names is real and that
        // stepping from it lands on a rung the run's own rule can reach. Both ends of the stepper
        // and the number between them come from this one lookup, so neither can describe a rung that
        // does not exist.
        assert_eq!(speed_rung(9), SPEEDS.iter().position(|s| *s == 8).unwrap());
        assert!(SPEEDS.contains(&crate::lab::step_speed(SPEEDS[speed_rung(9)], true)));
        assert!(SPEEDS.contains(&crate::lab::step_speed(SPEEDS[speed_rung(9)], false)));
        assert_eq!(
            pace_step(speed_rung(9), true),
            1,
            "9 is nowhere near an end"
        );
        assert_eq!(pace_step(speed_rung(9), false), -1);
        draw_panel(&summary, &mut view);

        // The mutation ladder is still a slider on the board, and that one *does* snap the state it
        // is handed: a rate off the ladder is not a thing the search can be told.
        draw_details(&summary, &mut view);
        assert!(
            MUTATION_RATES.contains(&view.mutation_rate),
            "mutation rate {} is off the ladder",
            view.mutation_rate
        );
    }

    #[test]
    fn the_chart_draws_geometry_a_window_could_show() {
        let points = history(&[
            (0, -20.0, -40.0, 0),
            (1, 120.0, 10.0, 0),
            (2, 411.0, 90.0, 1),
        ]);
        let vertices = draw_in_headless_context(&points, 120.0, Some(1));
        assert!(
            vertices > 100,
            "the chart drew almost nothing: {vertices} vertices"
        );

        // And with no history at all it still draws the placeholder.
        let placeholder = draw_in_headless_context(&[], 120.0, None);
        assert!(placeholder > 0, "the placeholder did not draw");
        assert!(
            placeholder < vertices,
            "the placeholder drew more than a real chart"
        );
    }

    /// Draw the chart into a headless context and report how many vertices came out.
    ///
    /// The same pattern `view`'s tests use: a real egui frame, tessellated, so the
    /// assertion is about the geometry that would reach the screen rather than about
    /// the functions that produced it.
    fn draw_in_headless_context(
        history: &[HistoryPoint],
        height: f32,
        scrub: Option<u32>,
    ) -> usize {
        let ctx = egui::Context::default();
        let size = Vec2::new(900.0, 200.0);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| chart_panel(ui, history, height, scrub));
            },
        );
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
        primitives
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn the_panel_says_when_the_ghost_limit_left_agents_out() {
        // A field under the limit is not described as capped, including at exactly the
        // limit with the champion in the population (its slot is the one not ghosted).
        assert_eq!(
            capped_roster_note(
                Population {
                    generation: 3,
                    size: 200
                },
                199
            ),
            None
        );
        assert_eq!(
            capped_roster_note(
                Population {
                    generation: 3,
                    size: 201
                },
                200
            ),
            None
        );
        // Nor when the champion is not in the population at all and every agent is drawn.
        assert_eq!(
            capped_roster_note(
                Population {
                    generation: 20,
                    size: 200
                },
                200
            ),
            None
        );

        // A capped roster says what was drawn and what the generation actually held.
        let note = capped_roster_note(
            Population {
                generation: 3,
                size: 1_001,
            },
            256,
        )
        .expect("a capped roster is reported");
        assert!(note.contains("best 256"), "{note}");
        assert!(note.contains("1001"), "{note}");
    }

    /// Draw the whole run panel headlessly, for the tests that care what the widgets do to
    /// the view state rather than what they look like.
    ///
    /// No atlas: the widgets take one to draw their pictures and every one of them is optional
    /// (`ui::icon_button`'s rule), so a panel with no textures in it is a panel with captions and no
    /// glyphs — which is what a tileset drawn before the `ui_*` sprites existed looks like, and a
    /// state a test can hold without a renderer.
    fn draw_panel(summary: &RunSummary, view: &mut ViewState) -> Actions {
        draw(summary, view, false)
    }

    /// Draw the instrument board headlessly — the other half of one run's panel, inside the
    /// Details window.
    fn draw_details(summary: &RunSummary, view: &mut ViewState) -> Actions {
        draw(summary, view, true)
    }

    fn draw(summary: &RunSummary, view: &mut ViewState, details: bool) -> Actions {
        let ctx = egui::Context::default();
        let mut actions = Actions::default();
        let size = Vec2::new(300.0, 700.0);
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            actions = if details {
                                lab_details(ui, summary, view)
                            } else {
                                lab_panel(ui, None, summary, view, false)
                            };
                        });
                    });
            },
        );
        // A panel that drew nothing at all would still report the right actions, so the
        // frame is checked too.
        let drawn: usize = ctx
            .tessellate(output.shapes, output.pixels_per_point)
            .iter()
            .map(|p| match &p.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                _ => 0,
            })
            .sum();
        assert!(drawn > 0, "the panel drew nothing");
        actions
    }

    /// An idle frame of the menu asks for nothing.
    ///
    /// The five things the menu can ask for are all destructive in the same way — pause a run,
    /// restart it, write a champion, continue one, run a generation — so a panel that fired one
    /// without a pointer on it would be a control nobody pressed, acting. Drawn with no atlas, too:
    /// a machine with a tileset older than the `ui_*` glyphs gets captioned plates and no pictures,
    /// and it still has to be a panel rather than a crash.
    #[test]
    fn an_idle_frame_of_the_menu_asks_for_nothing() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 1)]);
        let mut summary = RunSummary::new(&packet(points.clone(), Vec::new(), true), &points);
        // With and without a champion on disk: the row exists in both states (disabled until
        // there is a file), and neither state may press itself.
        assert_eq!(
            draw_panel(&summary, &mut ViewState::default()),
            Actions::default()
        );
        summary.champion_saved = true;
        assert_eq!(
            draw_panel(&summary, &mut ViewState::default()),
            Actions::default()
        );
    }

    /// The panel says four things, and they are what a viewer watching a run asks for: where it has
    /// got to, whether anything has reached the flag, **how long that took**, and how good the best
    /// agent is.
    ///
    /// The *count* is the assertion rather than the wording: the panel's whole problem was being
    /// longer than the question it answers, and a reading added to it later would pass every test
    /// that only checked what the others said. The clock is the fourth because it is the one a
    /// viewer acts on — it is what they take to the playtest to race.
    #[test]
    fn the_run_panel_says_four_things() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 1)]);
        let summary = RunSummary::new(&packet(points.clone(), Vec::new(), true), &points);
        let readings = menu_readings(&summary);
        assert_eq!(readings.len(), 4, "{readings:?}");

        // The generation, against the run's own target, one-based — the number a viewer reads off
        // the chart's right-hand edge too.
        assert_eq!(readings[0].value, "2 / 300");
        assert_eq!(readings[0].unit, "generations");
        // The flag: the generation's wins over the population, in the growth colour once the run has
        // been solved. *That* is the colour's job — a run that has been solved is the run's answer.
        assert_eq!(readings[1].value, "3 / 200");
        assert_eq!(readings[1].unit, "reached the flag");
        assert_eq!(readings[1].colour, GOOD);
        // The clock, in the sim's own notation rather than the panel's: the number on the panel is
        // the number the episode was scored on, which is the number the playtest races.
        assert_eq!(readings[2].value, sim::format_time(505));
        assert_eq!(readings[2].value, "8.42s");
        assert_eq!(readings[2].unit, "the quickest win");
        assert_eq!(readings[2].colour, GOOD);
        // The best, from the run's own formatter, so the panel and the chart cannot disagree about
        // what 411.2 looks like.
        assert_eq!(readings[3].value, format_fitness(411.0));
        assert_eq!(readings[3].unit, "best fitness");

        // A run nobody has solved says the same thing in a dimmer colour rather than in a word: a
        // count of zero is the reading.
        let unsolved = RunSummary::new(
            &packet(points, Vec::new(), false),
            &history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 0)]),
        );
        assert_eq!(menu_readings(&unsolved)[1].value, "0 / 200");
        assert_eq!(menu_readings(&unsolved)[1].colour, TEXT_DIM);

        // And a run with no win has no time. An em dash and a sentence rather than `0.00s`, which
        // would read as "the flag was reached instantly" — the opposite of what has happened.
        let mut untimed = RunSummary::new(
            &packet(history(&[(0, 10.0, 5.0, 0)]), Vec::new(), false),
            &history(&[(0, 10.0, 5.0, 0)]),
        );
        untimed.fastest = None;
        let readings = menu_readings(&untimed);
        assert_eq!(readings[2].value, "\u{2014}", "{readings:?}");
        assert_eq!(readings[2].unit, "nothing has reached the flag yet");
        assert_eq!(readings[2].colour, TEXT_DIM);
    }

    /// The replay's panel names the record on screen and quotes the star's own clock.
    ///
    /// Two claims, and they are the two a viewer reported as wrong: the agent being watched was
    /// labelled the run's best when it is the run's *quickest win*, and the time beside it kept
    /// counting after the flag. Both are read here off [`replay_readings`] rather than off a drawn
    /// frame, because the readings are a pure function of the replay and the panel is the only place
    /// in this mode that prints a time beside an agent.
    #[test]
    fn the_replay_panel_names_the_record_and_quotes_the_stars_own_clock() {
        let level = sim::Level::starter("panel-replay", 40, 8);
        let packet = crate::lab::scored_packet(&level, "panel-reading");

        // Whether *this* fixture's population has won is the genome draw's business, and the draw
        // moves with the observation layout — a sensor bump reshuffles the whole weight stream, so a
        // level this generation solved is one the next build's first generation may not (and the
        // other way round, which is how this test was found to be reading a record it did not
        // intend). The claim below is about a run that has **never** won, so it is stated by taking
        // the record off the packet: the same generation, the same episodes, read as a run with no
        // time to beat. What the packet holds as written is checked beside it instead of assumed.
        let mut no_record = packet.clone();
        no_record.fastest = None;
        no_record.fastest_agent = None;
        let mut replay =
            crate::replay::Replay::new(&no_record, &level, &evo::EvalConfig::default())
                .expect("a scored generation holds an agent to replay");
        // And when the draw did win, the same packet with its record intact says so: one word per
        // record, from the record's own name rather than from this module's choice of phrase.
        if packet.fastest_agent.is_some() {
            let won = crate::replay::Replay::new(&packet, &level, &evo::EvalConfig::default())
                .expect("a winning packet replays");
            assert_eq!(won.star().basis.label(), "the run's quickest win");
        }

        let readings = replay_readings(&replay);
        assert_eq!(readings.len(), 3);
        // The label is the record's own name (`StarBasis::label`), not a word this module picked: with
        // no record on the packet there is no time to race, so it is the best attempt.
        assert_eq!(readings[1].unit, replay.star().basis.label());
        assert_eq!(readings[1].unit, "the run's best attempt");
        assert_eq!(
            readings[2].unit,
            format!("0.0s, score {}", format_fitness(replay.star_score())),
            "an unstarted episode is at 0.0s"
        );

        // Run the star out, then read the panel again and keep the field going for ten seconds. The
        // time it prints is the episode's own and it does not move again: the field's clock is still
        // climbing under it, which is exactly the number that used to be shown here.
        let mut ticks = 0;
        while !replay.star_finished() && ticks < 4_000 {
            replay.advance(1.0 / 60.0);
            ticks += 1;
        }
        assert!(ticks < 4_000, "the star's episode never ended");
        let finished = replay_readings(&replay);
        assert_eq!(finished[2].value, replay.star_sim().outcome().as_str());
        let stopped = finished[2].unit.clone();
        assert!(
            stopped.starts_with(&format!("{:.1}s, score ", replay.star_elapsed())),
            "{stopped}"
        );
        for _ in 0..600 {
            replay.advance(1.0 / 60.0);
        }
        assert!(
            replay.clock() > replay.star_elapsed(),
            "the field's clock stopped too, so this proves nothing"
        );
        assert_eq!(
            replay_readings(&replay)[2].unit,
            stopped,
            "the panel's clock kept counting after the episode ended"
        );
    }

    /// The panel's pace stepper moves one rung at a time, stops at both ends of the ladder, and
    /// reports the step rather than applying it.
    #[test]
    fn the_panel_has_a_pace_stepper_that_walks_the_same_ladder_as_the_keys() {
        // The ends are *requests of zero*, not a step the run would have to refuse: the same number
        // decides whether the button is drawn enabled, so there is no state where a lit button asks
        // for nothing.
        assert_eq!(
            pace_step(0, false),
            0,
            "the bottom of the ladder has nowhere to go"
        );
        assert_eq!(pace_step(0, true), 1);
        assert_eq!(pace_step(SPEEDS.len() - 1, true), 0, "nor does the top");
        assert_eq!(pace_step(SPEEDS.len() - 1, false), -1);
        for (rung, here) in SPEEDS.iter().enumerate() {
            // One rung, never a jump: `lab::step_speed` is what turns the request into a rung, and a
            // request of two would skip one the viewer could not get back to.
            assert!(pace_step(rung, true).abs() <= 1 && pace_step(rung, false).abs() <= 1);
            // And a press that does move the run lands on a rung of the same ladder the keys and the
            // terminal use — 1x, 2x, 4x ... 50x.
            let asked = if pace_step(rung, true) == 1 {
                crate::lab::step_speed(*here, true)
            } else {
                crate::lab::step_speed(*here, false)
            };
            assert!(SPEEDS.contains(&asked), "{asked}x is not a rung");
        }

        // Drawn, the panel reports nothing at rest and does not move the pace itself: it reports and
        // the window applies, which is how one command serves a button, a key and a slider.
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 1)]);
        let summary = RunSummary::new(&packet(points.clone(), Vec::new(), true), &points);
        let mut view = ViewState::default();
        assert!(SPEEDS.contains(&view.speed), "the default pace is a rung");
        let idle = draw_panel(&summary, &mut view);
        assert_eq!(idle.speed, 0);
        assert_eq!(view.speed, SPEEDS[speed_rung(view.speed)]);
    }

    /// Draw one panel headlessly at a stated content width, and report the rectangle it used.
    ///
    /// The rectangle is the *content's* own idea of how wide it is, before any clipping — which is
    /// exactly what egui uses to size a side panel. A row that cannot wrap therefore shows up here
    /// as a frame wider than the column it was given.
    fn used_rect(width: f32, mut draw: impl FnMut(&mut egui::Ui)) -> Rect {
        let ctx = egui::Context::default();
        let mut used = Rect::NOTHING;
        let size = Vec2::new(width, 700.0);
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::default())
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            draw(ui);
                            used = ui.min_rect();
                        });
                    });
            },
        );
        used
    }

    /// Neither panel is wider than the column it is drawn in.
    ///
    /// This is the bug the README's screenshots caught: a row that cannot wrap does not get clipped
    /// by a side panel, it *sizes* the panel. The run's panel came out about 230 points wide while
    /// the level beside it was laid out against the width it had reserved a frame earlier, so the
    /// canvas painted over the panel's left edge and the panel's text ran off the window. Both the
    /// reading rows and the pace description wrap now; 280 is the panel's content width at its
    /// 300-point floor.
    #[test]
    fn neither_panel_grows_past_the_column_it_is_drawn_in() {
        let points = history(&[(0, 10.0, 5.0, 0), (1, 411.0, 60.0, 1)]);
        let mut won = packet(points.clone(), Vec::new(), true);
        // The strict reading — "reached the flag — 1 on every trial" — is the longest qualifier the
        // panel can carry, and the one that used to make it grow.
        won.solved = 1;
        let summary = RunSummary::new(&won, &points);
        let mut view = ViewState::default();
        let used = used_rect(280.0, |ui| {
            lab_panel(ui, None, &summary, &mut view, false);
        });
        assert!(
            used.width() <= 280.5,
            "the run's panel is {:.1} points wide: {used:?}",
            used.width()
        );

        // The replay's own long row is its pace description — "the pace it plays at, in real time"
        // — which sat beside the stepper and widened the panel the same way.
        let level = sim::Level::starter("panel-width", 40, 8);
        let scored = crate::lab::scored_packet(&level, "panel-width");
        let mut replay = crate::replay::Replay::new(&scored, &level, &evo::EvalConfig::default())
            .expect("a scored generation holds an agent to replay");
        let view = ViewState::default();
        let used = used_rect(280.0, |ui| {
            replay_panel(ui, None, &mut replay, &view);
        });
        assert!(
            used.width() <= 280.5,
            "the replay's panel is {:.1} points wide: {used:?}",
            used.width()
        );
    }
}
