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

use crate::lab::{Lane, Packet, MUTATION_RATES, SPEEDS};
use crate::replay::{Population, Replay};
use crate::view::cohort_swatch;
use sim::Outcome;

/// Panel background, one shade lighter than the window's own dark grey.
const PANEL_BG: Color32 = Color32::from_rgb(30, 34, 40);
/// The chart's plot area, darker than the panel so the lines read as data.
const PLOT_BG: Color32 = Color32::from_rgb(22, 25, 30);
const AXIS: Color32 = Color32::from_rgb(70, 78, 90);
const MAX_LINE: Color32 = Color32::from_rgb(120, 216, 120);
const AVG_LINE: Color32 = Color32::from_rgb(120, 170, 230);
const WIN_TICK: Color32 = Color32::from_rgb(250, 214, 102);
const SCRUB_LINE: Color32 = Color32::from_rgb(240, 150, 90);
const TEXT: Color32 = Color32::from_rgb(214, 220, 228);
const TEXT_DIM: Color32 = Color32::from_rgb(160, 168, 180);
const GOOD: Color32 = Color32::from_rgb(120, 216, 120);
const WARN: Color32 = Color32::from_rgb(240, 150, 90);

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
    pub best: Option<BestLine>,
    /// What `render`'s own rule makes of the history.
    pub trend: Trend,
    pub stall: Option<Stall>,
    pub genius: bool,
    /// The note from the run's one curriculum step, if it fell back to an easier level
    /// when it stalled (§12 M5). A fact about the run rather than a reading of its
    /// history, so it is carried through from the packet and not derived here.
    pub curriculum: Option<String>,
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
            max_fitness: packet.max_fitness,
            avg_fitness: packet.avg_fitness,
            min_fitness: packet.min_fitness,
            mutation_sigma: packet.mutation_sigma,
            stagnation: packet.stagnation,
            immigrants: packet.immigrants,
            best: None,
            trend: fitness_trend(history),
            stall: fitness_stall(history),
            genius: packet.genius,
            curriculum: packet.curriculum.clone(),
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

    /// Whether any generation has reached the flag.
    pub fn solved(&self) -> bool {
        self.wins > 0 || self.best.as_ref().is_some_and(|b| b.outcome == "won")
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
    /// Leave the scrubbed view and follow the live run again.
    pub resume: bool,
    /// The follow checkbox was clicked. The window owns the state, as it does for pause:
    /// the widget reports the intent and the state is flipped in one place.
    pub follow: bool,
    /// Watch the run's best agent: the recorded episode replayed at real time, with the
    /// generation's other agents ghosted. Offered by the evolution panel, and only once
    /// there is an agent to watch.
    pub watch: bool,
    /// The ghosts checkbox was clicked, in the replay's own panel.
    pub ghosts: bool,
    /// -1 / +1 from the scrubber's step buttons.
    pub scrub_delta: i32,
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
    if keys.older {
        out.push(Command::Scrub(-1));
    }
    if keys.newer {
        out.push(Command::Scrub(1));
    }
    out
}

/// The right-hand panel: what the run is doing, and every control on it.
/// The panel's one line about the search itself: the step, and — only when it has something
/// to say — how long the best has stood still and what that bought.
///
/// A pure function of the summary so the three shapes can be checked without a window, the
/// same way [`chart_geometry`] is split out of the chart's drawing. The clauses after the
/// step are conditional because a run that is still finding new bests has neither a stall to
/// report nor a restart to explain, and `still for 0   +0 reheated genomes` beside its score
/// would be noise dressed as information.
///    /// `sigma` is a constant on a run with a fixed step — the default — and there the line is
    /// still worth its row: with reheated genomes only ever arriving after a stall, the line is
    /// where a viewer sees the restarts happen.
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

pub fn lab_panel(
    ui: &mut egui::Ui,
    summary: &RunSummary,
    view: &mut ViewState,
    level_changed: bool,
) -> Actions {
    let mut actions = Actions::default();

    ui.heading("Evolution");
    ui.label(summary.level_name.clone());
    ui.separator();

    // --- where the run is -------------------------------------------------
    match summary.target {
        Some(target) => ui.monospace(format!(
            "generation {} of {}{}",
            summary.generation + 1,
            target,
            if summary.finished { "  (finished)" } else { "" }
        )),
        None => ui.monospace(format!("generation {}", summary.generation + 1)),
    };
    let wins_line = format!("pop {}   wins {}", summary.pop_size, summary.wins);
    if summary.solved() {
        ui.colored_label(GOOD, wins_line);
    } else {
        ui.monospace(wins_line);
    }
    ui.monospace(format!(
        "max {}   avg {}   min {}",
        format_fitness(summary.max_fitness),
        format_fitness(summary.avg_fitness),
        format_fitness(summary.min_fitness)
    ));

    // The trend and the stall are `render`'s readings of the same history the chart
    // draws, so they are never phrases invented here.
    let trend_colour = match summary.trend {
        Trend::Solved => GOOD,
        Trend::Stalled | Trend::Degenerating => WARN,
        _ => TEXT_DIM,
    };
    ui.colored_label(trend_colour, summary.trend.label());
    if let Some(stall) = &summary.stall {
        ui.colored_label(WARN, stall.label());
        match summary.deaths_at {
            Some(tile) => ui.small(format!(
                "deaths cluster at x {tile} ({} of them) — the obstacle to fix",
                summary.deaths_count
            )),
            None => ui.small("no death cluster inside the level to point at"),
        };
    }
    if summary.genius {
        ui.colored_label(WIN_TICK, "genius agent: max jumped by more than half");
    }
    // What the search is doing to itself (see [`search_line`]).
    ui.colored_label(TEXT_DIM, search_line(summary));
    // The run's own account of the one detour it took, in the words `evo` gave it: the
    // window has no opinion about how the step went, it only has to say it happened.
    if let Some(note) = &summary.curriculum {
        ui.colored_label(TEXT, note);
    }
    if summary.finished {
        ui.colored_label(
            GOOD,
            format!(
                "this run is over ({} generations) — Restart to train again, or V to \
                 watch the agent it found",
                summary.generations_run
            ),
        );
    }

    if let Some(best) = &summary.best {
        ui.add_space(4.0);
        ui.small(format!(
            "best: gen {} id {} fitness {}",
            best.generation,
            best.genome_id,
            format_fitness(best.fitness)
        ));
        ui.small(format!(
            "{} after {} frames, best_x {:.1}, {:.1} tiles from the flag",
            best.outcome, best.frames, best.best_x, best.distance_to_goal
        ));
    }

    if level_changed {
        ui.add_space(4.0);
        ui.colored_label(
            WARN,
            "the level has changed since this run started — the population is still \
             training on the old one. Restart to train on the edit.",
        );
    }

    ui.add_space(6.0);
    ui.separator();
    ui.label("Controls");
    ui.horizontal(|ui| {
        // The label says what the *next* press does, and the button only reports the
        // intent: flipping `view.paused` here as well would double-toggle it.
        let label = if view.paused { "Resume" } else { "Pause" };
        if ui.button(label).clicked() {
            actions.pause = true;
        }
        if ui
            .button("One generation")
            .on_hover_text("run exactly one generation, then stop")
            .clicked()
        {
            actions.step = true;
        }
    });
    ui.horizontal(|ui| {
        if ui
            .button("Restart run")
            .on_hover_text("a fresh population on the level as it is now")
            .clicked()
        {
            actions.restart = true;
        }
        if ui
            .button("Save champion")
            .on_hover_text("write the best-ever genome to the champions directory")
            .clicked()
        {
            actions.save = true;
        }
    });
    // Watching the run's answer is the one thing the run cannot offer from inside itself:
    // the population advances a generation at a time, and this is one episode at the speed
    // of the game. Offered as soon as there is an agent to watch, and pointed at again in
    // the "this run is over" line above once the search has stopped.
    let watch = ui
        .add_enabled_ui(summary.best.is_some(), |ui| {
            ui.button("Watch the best (V)")
                .on_hover_text(
                    "replay the run's best agent at real time, with every other agent of \
                     its generation ghosted",
                )
                .on_disabled_hover_text("nothing scored yet: a replay needs an agent to watch")
        })
        .inner;
    if watch.clicked() {
        actions.watch = true;
    }

    // Both ladders are sliders over an index rather than a raw value, so every value
    // the mouse can reach is a rung the terminal's keys can also reach (§10 Tier 2.9).
    let mut speed_index = SPEEDS
        .iter()
        .position(|s| *s == view.speed)
        .unwrap_or_else(|| {
            SPEEDS
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.abs_diff(view.speed))
                .map(|(i, _)| i)
                .unwrap_or(0)
        });
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut speed_index, 0..=SPEEDS.len() - 1)
                .text("speed")
                .show_value(false),
        );
        ui.monospace(format!("{}x", SPEEDS[speed_index]));
    });
    view.speed = SPEEDS[speed_index];

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
        ui.monospace(format!("{:.2}", MUTATION_RATES[mut_index]));
    });
    view.mutation_rate = MUTATION_RATES[mut_index];

    ui.checkbox(&mut view.show_deaths, "show death marks");
    // The button reports the intent and the window flips the state, exactly as the pause
    // button does: a widget that changed it here as well would double-toggle.
    let mut follow = view.follow;
    if ui
        .checkbox(&mut follow, "follow the leader")
        .on_hover_text("follow the leading agent, or show the whole level")
        .changed()
    {
        actions.follow = true;
    }

    // --- the lanes --------------------------------------------------------
    ui.add_space(6.0);
    ui.separator();
    ui.label("Agents in this generation");
    ui.small("champion solid, the rest ghosted; the swatch is the agent's outline");
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

    // --- the scrubber -----------------------------------------------------
    ui.add_space(6.0);
    ui.separator();
    ui.label("Recorded champions");

    if summary.recorded.is_empty() {
        ui.small("nothing recorded yet — a route is kept when the best improves");
        return actions;
    }

    // Walked by **generation**, never by index into the record: the log drops its
    // oldest entry when it fills, which shifts every index by one and would silently
    // re-point the cursor at a route the viewer never chose.
    let generations: Vec<u32> = summary.recorded.iter().map(|r| r.generation).collect();
    let current = view
        .scrub
        .and_then(|g| generations.iter().position(|x| *x == g));

    ui.horizontal(|ui| {
        ui.add_enabled_ui(current.is_some(), |ui| {
            if ui.button("<").clicked() {
                actions.scrub_delta = -1;
            }
            if ui.button(">").clicked() {
                actions.scrub_delta = 1;
            }
        });
        ui.add_enabled_ui(view.scrub.is_some(), |ui| {
            if ui.button("Back to live").clicked() {
                actions.resume = true;
            }
        });
    });

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
        None => {
            ui.small("live: the lanes on the canvas are this generation's agents");
        }
    }

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
pub fn replay_panel(ui: &mut egui::Ui, replay: &mut Replay, view: &ViewState) -> Actions {
    let mut actions = Actions::default();
    let star = replay.star().clone();

    ui.heading("Replay");
    ui.label(replay.level_name().to_string());
    ui.separator();

    // --- whose episode this is --------------------------------------------
    ui.label("the run's best agent");
    ui.monospace(format!(
        "generation {}   id {}",
        star.generation + 1,
        star.genome_id
    ));
    ui.small(format!(
        "recorded: {} in {} frames, fitness {}",
        star.outcome.as_str(),
        star.frames,
        format_fitness(star.fitness)
    ));

    if replay.star_finished() {
        // The score beside the record is the replay's own receipt: same episode, same
        // number if it is faithful. A mismatch would be a bug here rather than news about
        // the run, which is exactly why the two are printed together.
        let outcome = replay.star_sim().outcome();
        ui.colored_label(
            if outcome == Outcome::Won { GOOD } else { WARN },
            format!(
                "now: {} in {:.1}s, score {}",
                outcome.as_str(),
                replay.clock(),
                format_fitness(replay.star_score())
            ),
        );
    } else {
        ui.colored_label(
            TEXT,
            format!(
                "now: {:.1}s in, score {}",
                replay.clock(),
                format_fitness(replay.star_score())
            ),
        );
    }

    // --- the ghosts -------------------------------------------------------
    ui.add_space(6.0);
    ui.separator();
    ui.label("Ghosts");
    // Named, and named as a *different* generation from the champion's: the field is the
    // run's newest scored generation and the agent above it is the best-ever record, which
    // are two numbers the moment the search has moved past its best idea.
    ui.small(format!(
        "the population of generation {}, each replaying its own episode",
        replay.population().generation + 1
    ));
    ui.monospace(format!(
        "{} ghosted: {} still going, {} at the flag",
        replay.ghost_count(),
        replay.ghosts_running(),
        replay.ghosts_won()
    ));
    if let Some(note) = capped_roster_note(replay.population(), replay.ghost_count()) {
        ui.small(note);
    }
    // The legend for the colours on the canvas, in the canvas's own palette
    // (`view::cohort_swatch`): a field drawn in four tints and explained nowhere would be
    // four tints of noise, and the one thing a viewer cannot guess from the picture is
    // which end of the rank order a colour is. It is drawn whether or not the ghosts are
    // showing — the bands are a fact about the field rather than about the toggle, and a
    // panel that reflowed every time `G` was pressed would move the controls under the
    // viewer's hand.
    ui.add_space(4.0);
    ui.small("the field by rank, best first — each band fainter than the one above it");
    for (band, ranks) in replay.cohorts() {
        ui.horizontal(|ui| {
            ui.colored_label(cohort_swatch(band), "\u{25a0}");
            ui.small(format!(
                "ranks {}–{} ({})",
                ranks.start + 1,
                ranks.end,
                ranks.len()
            ));
        });
    }
    if replay.all_finished() {
        // Every agent has stopped, so nothing is moving any more: a viewer who wants the
        // field to run again is told how, rather than watching a still frame wonder.
        ui.small("the whole field has finished — R runs it again");
    }
    let mut ghosts = replay.ghosts;
    if ui
        .checkbox(&mut ghosts, "show ghosts (G)")
        .on_hover_text(
            "hidden ghosts keep running, so the field is where the champion has it \
             rather than where it was when you last looked",
        )
        .changed()
    {
        actions.ghosts = true;
    }

    // --- controls ---------------------------------------------------------
    ui.add_space(6.0);
    ui.separator();
    ui.label("Controls");
    ui.horizontal(|ui| {
        // The label says what the *next* press does, and the button only reports the
        // intent: flipping `paused` here as well would double-toggle it.
        let label = if replay.paused { "Play" } else { "Pause" };
        if ui.button(label).clicked() {
            actions.pause = true;
        }
        if ui
            .button("Again (R)")
            .on_hover_text("run the same episodes again, from the first tick")
            .clicked()
        {
            actions.restart = true;
        }
    });

    let mut speed_index = SPEEDS.iter().position(|s| *s == replay.speed).unwrap_or(0);
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut speed_index, 0..=SPEEDS.len() - 1)
                .text("speed")
                .show_value(false),
        );
        // Real time is the rung this view opens on, and the only one worth naming.
        ui.monospace(if SPEEDS[speed_index] == crate::replay::REAL_TIME {
            "1x real time".to_string()
        } else {
            format!("{}x", SPEEDS[speed_index])
        });
    });
    replay.speed = SPEEDS[speed_index];

    let mut follow = view.follow;
    if ui
        .checkbox(&mut follow, "follow the champion")
        .on_hover_text("stay on the agent being replayed, or show the whole level")
        .changed()
    {
        actions.follow = true;
    }

    if ui
        .button("Back to the run (Esc)")
        .on_hover_text("return to the evolution view, which is parked where you left it")
        .clicked()
    {
        actions.resume = true;
    }
    ui.small("space pause   R again   G ghosts   +/- speed   F follow   V or Esc back");

    actions
}

/// The bottom panel: the fitness chart, with every generation that reached the flag
/// marked. `scrub` is the recorded generation being shown, when there is one.
pub fn chart_panel(ui: &mut egui::Ui, history: &[HistoryPoint], height: f32, scrub: Option<u32>) {
    let width = ui.available_width().max(200.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), PANEL_BG);

    let font = FontId::proportional(11.0);
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
        TEXT_DIM,
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
            TEXT_DIM,
        );
        return;
    }

    // The y rule: three lines, and the two numbers the series must be read against.
    for value in [geometry.hi, (geometry.hi + geometry.lo) * 0.5, geometry.lo] {
        let y = plot.bottom() - plot.height() * (value - geometry.lo) / (geometry.hi - geometry.lo);
        let y = y.clamp(plot.top(), plot.bottom());
        painter.line_segment(
            [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
            Stroke::new(1.0_f32, AXIS),
        );
        painter.text(
            Pos2::new(plot.left() - 6.0, y),
            Align2::RIGHT_CENTER,
            format_fitness(value),
            font.clone(),
            TEXT_DIM,
        );
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
                Pos2::new(win.x, plot.top() + 6.0),
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
        TEXT_DIM,
    );
    painter.text(
        Pos2::new(plot.right(), plot.bottom() + 3.0),
        Align2::RIGHT_TOP,
        format!("gen {}", geometry.last_generation + 1),
        font,
        TEXT_DIM,
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
    let usable: Vec<&HistoryPoint> = history
        .iter()
        .filter(|p| p.max_fitness.is_finite() || p.avg_fitness.is_finite())
        .collect();
    if usable.is_empty() {
        return ChartGeometry::default();
    }

    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for point in &usable {
        for value in [point.max_fitness, point.avg_fitness] {
            if value.is_finite() {
                lo = lo.min(value);
                hi = hi.max(value);
            }
        }
    }
    if !lo.is_finite() || !hi.is_finite() {
        return ChartGeometry::default();
    }

    // A flat run is a real run (a stalled population in particular), so the span is
    // never zero: with one it would divide by zero, and with none the flat line would
    // sit on the axis where it reads as "zero fitness".
    let span = (hi - lo).max(1.0);
    let lo = lo - span * 0.08;
    let hi = hi + span * 0.08;

    let first = usable[0].generation;
    let last = usable[usable.len() - 1].generation;
    let x = |generation: u32| {
        if last > first {
            plot.left() + plot.width() * (generation - first) as f32 / (last - first) as f32
        } else {
            plot.center().x
        }
    };
    let y = |value: f32| plot.bottom() - plot.height() * (value - lo) / (hi - lo);

    let mut geometry = ChartGeometry {
        lo,
        hi,
        first_generation: first,
        last_generation: last,
        ..ChartGeometry::default()
    };
    for point in &usable {
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
            genius: false,
            pop_size: 200,
            // A quiet, fixed-sigma run: the panel's search line then has nothing to say
            // beyond the step itself, which is the common case a fixture should be.
            mutation_sigma: 0.30,
            stagnation: 0,
            immigrants: 0,
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
        // And the panel draws it: a rescue the window never mentions would be invisible.
        draw_panel(&summary, &mut ViewState::default());
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

        // Live: the panels offer the way back in only when there is a past to leave.
        let actions = draw_panel(&summary, &mut ViewState::default());
        assert_eq!(actions, Actions::default());

        // Looking at a recorded generation: the note says so and Resume is offered.
        let mut view = ViewState {
            scrub: Some(0),
            ..ViewState::default()
        };
        let actions = draw_panel(&summary, &mut view);
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
        // A value that is not on the rungs (a hand-edited config, a future key) snaps
        // to the nearest one rather than leaving the slider unrepresentable.
        let mut view = ViewState {
            speed: 9,
            mutation_rate: 0.2,
            ..ViewState::default()
        };
        let points = history(&[(0, 10.0, 5.0, 0), (1, 20.0, 9.0, 0)]);
        let summary = RunSummary::new(&packet(points.clone(), Vec::new(), false), &points);
        draw_panel(&summary, &mut view);
        assert!(
            SPEEDS.contains(&view.speed),
            "speed {} is off the ladder",
            view.speed
        );
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

    /// Draw the whole panel headlessly, for the tests that care what the widgets do to
    /// the view state rather than what they look like.
    fn draw_panel(summary: &RunSummary, view: &mut ViewState) -> Actions {
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
                            actions = lab_panel(ui, summary, view, false);
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
}
