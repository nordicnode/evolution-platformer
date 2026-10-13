//! Curriculum resume: what a run does when it stops moving (§12 M5, findings 19–22).
//!
//! The plan's stall chain ends here, and this is the module both front-ends call.
//! Its premise is finding 22: **a stalled population is the best starting material a
//! run has**. Neither patience nor the live mutation-rate knob unsticks these levels
//! (findings 19 and 20), so the answer to "this level is not being solved" is a
//! different landscape, not a reset. The population that is stuck already performs
//! most of the jump it keeps dying on; it needs the *other* parts of its policy
//! rearranged, and a level it can actually get somewhere on is what rearranges them.
//!
//! Two artifacts come out of one step, and the difference between them matters:
//!
//! * **A checkpoint** of the stuck population, written *before* anything moves it.
//!   That is the plan's "restart from the checkpoint at the stuck point", and it is
//!   what makes an unrepeatable run repeatable: `watch --resume` (and the window's
//!   own resume) is the way back into a run that would otherwise take a lucky seed to
//!   reproduce. It also means the detour is never destructive — the population as it
//!   was is on disk whatever the excursion learns.
//! * **The trained population**, adopted back into the run it came from. The
//!   excursion trains under the *run's own* hyperparameters, including the mutation
//!   rate the viewer has dialled in, so what comes back is the population this run
//!   would have bred, had the level been easier.
//!
//! The excursion runs on a separate [`Trainer`] and is adopted through
//! [`Trainer::adopt`], which is the whole point of that method: the run keeps its own
//! chart history, its own best-ever champion, its own genius baseline and its own RNG
//! stream. Those belong to *the run*, not to whichever level the population last saw,
//! and a curriculum step that reset them would make the run's chart a lie (a cliff,
//! or a spike, that no agent ever experienced).
//!
//! The step is deliberately **not animated** in either front-end. At pop 200 a
//! generation costs about 14 ms, so the whole detour is over in well under a second,
//! while forty seconds of a *different level* on screen would bury the thing the
//! viewer is actually waiting for — the hard level's next generation. What the viewer
//! gets instead is [`CurriculumStep::note`], naming what happened, and a run whose
//! population has changed underneath it.

use std::path::{Path, PathBuf};

use sim::Level;

use crate::ga::Trainer;

/// Generations spent on the easier level when a run stalls (§12 M5, finding 22).
///
/// Measured, not guessed: a 40-generation detour was the cheapest that rescued both
/// stalled seeds tried, and the longer ones were strictly worse on the seed that was
/// going to solve anyway (L5 seed 3 won at overall generation 100 with 40 detour
/// generations, and at 284 and 321 with 100 and 200).
///
/// It lives here rather than in either front-end because the terminal's
/// `--curriculum-generations` and the window's own knob are the same number: two
/// copies of a detour length are two chances for the two front-ends to disagree about
/// what a curriculum step costs.
pub const DEFAULT_CURRICULUM_GENERATIONS: u32 = 40;

/// What one curriculum step did, as numbers rather than as a sentence.
///
/// The numbers are here and the phrasing is below so that the terminal's log line and
/// the window's panel row cannot disagree about them: the two front-ends differ in
/// where they put the line, never in what it claims.
#[derive(Clone, Debug)]
pub struct CurriculumStep {
    /// Name of the easier level the population was trained on.
    pub easier_level: String,
    /// Generations actually spent there.
    pub generations: u32,
    /// The checkpoint written from the stuck population, before anything moved it.
    pub checkpoint: PathBuf,
    /// Best fitness reached during the excursion.
    pub best: f32,
    /// The easier level's own generation when the flag was first reached there.
    pub flag_at: Option<u32>,
}

impl CurriculumStep {
    /// The step as one line, for the terminal's log and the window's panel.
    ///
    /// Three facts, in the order the viewer needs them: how long the detour took, how
    /// it went, and that the run is back where it was. The flag clause says plainly
    /// when the easier level was *not* solved either — a detour that learned nothing
    /// is a result about the level, and hiding it would invite reading the note as
    /// "this run is fixed now".
    pub fn note(&self, main_level: &str) -> String {
        format!(
            "curriculum: {} generations on {} from the stalled population \
             (checkpoint -> {}, best there {:.2}, {}); back on {}",
            self.generations,
            self.easier_level,
            self.checkpoint.display(),
            self.best,
            match self.flag_at {
                Some(g) => format!("flag reached at its generation {g}"),
                None => "flag not reached there yet".to_string(),
            },
            main_level
        )
    }
}

/// One curriculum step: checkpoint the stuck population, train it on the easier level
/// for a while, and hand it back to the run it came from (§12 M5).
///
/// `checkpoint_path` is the caller's decision rather than this function's, because the
/// two front-ends name their artifacts with the same convention they use for champions
/// (`<level>.population.json` beside `<level>.json`), and a file named here would drift
/// from it. `stall_note` goes into the checkpoint's own note, so a resumed run can say
/// *why* the population exists (the stall that triggered it, in the front-end's words).
///
/// `mutation_rate` is applied to the excursion from its first generation:
/// [`Trainer::resume`] takes the config by value, and the rate the viewer has dialled
/// in is part of "the hyperparameters this run is using".
///
/// Errors only for real failures — a checkpoint that cannot be written, a population
/// that cannot be adopted. A detour that learns nothing is a result, not an error.
pub fn curriculum_step(
    trainer: &mut Trainer,
    main_level: &Level,
    easier: &Level,
    generations: u32,
    checkpoint_path: &Path,
    mutation_rate: f32,
    stall_note: &str,
) -> Result<CurriculumStep, String> {
    // The stuck population on disk, before anything moves it: this is the plan's
    // "checkpoint at the stuck point", and it is what makes an unrepeatable run
    // repeatable — `--resume` is the way back in.
    let checkpoint = crate::population::PopulationFile::new(
        &trainer.genomes,
        trainer.generation,
        main_level,
        &trainer.config,
        format!("stalled: {stall_note}"),
    );
    // A bare file name has an empty parent, and creating "" is an error rather than a
    // no-op, so the directory is only made when there is one.
    if let Some(dir) = checkpoint_path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
    {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    checkpoint.save(checkpoint_path)?;

    // A real training run on the easier level, from the population that is stuck:
    // same genomes, same ids (they are part of every episode seed), same
    // hyperparameters — including the mutation rate the viewer has dialled in.
    let mut cfg = trainer.config.clone();
    cfg.mutation_rate = mutation_rate;
    let mut excursion = Trainer::resume(cfg, trainer.genomes.clone(), trainer.generation)?;
    let mut best = f32::NEG_INFINITY;
    let mut flag_at = None;
    for _ in 0..generations {
        let report = excursion.step(easier);
        best = best.max(report.max_fitness);
        if report.wins > 0 && flag_at.is_none() {
            flag_at = Some(excursion.generation);
        }
    }
    let spent = excursion.generation.saturating_sub(trainer.generation);
    trainer.adopt(excursion.genomes)?;

    Ok(CurriculumStep {
        easier_level: easier.name.clone(),
        generations: spent,
        checkpoint: checkpoint_path.to_path_buf(),
        best,
        flag_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{flat_runway, gap_runway};
    use crate::ga::GaConfig;

    /// A population small enough that a detour costs milliseconds. The GA's own
    /// behaviour is `ga`'s business; what is tested here is that the step writes what
    /// it says it wrote and hands back what it took.
    fn tiny() -> GaConfig {
        GaConfig {
            pop_size: 4,
            elite: 1,
            tournament: 2,
            mutation_rate: 0.10,
            master_seed: 3,
            ..GaConfig::default()
        }
    }

    /// A directory of this run's own, so two tests running at once cannot write over
    /// one another's checkpoint.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "evo-curriculum-{}-{tag}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("a temp directory");
        dir
    }

    /// The population after one real generation: a trained population with ids, which
    /// is the thing a stall actually happens to.
    fn trained(main: &Level, config: &GaConfig) -> Trainer {
        let mut trainer = Trainer::new(config.clone());
        trainer.step(main);
        trainer
    }

    fn genome_ids(trainer: &Trainer) -> Vec<u32> {
        let mut ids: Vec<u32> = trainer.genomes.iter().map(|g| g.id).collect();
        ids.sort_unstable();
        ids
    }

    /// Every weight in the population, in order: the cheap way to ask "is this the
    /// same population?" without depending on how breeding happens to order it.
    fn genome_weights(trainer: &Trainer) -> Vec<f32> {
        trainer
            .genomes
            .iter()
            .flat_map(|g| g.weights.iter().copied())
            .collect()
    }

    #[test]
    fn the_step_writes_a_checkpoint_that_loads_back_with_the_stuck_population() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let easier = flat_runway("easy", 24, 22);
        let config = tiny();
        let mut trainer = trained(&main, &config);
        let ids = genome_ids(&trainer);
        let generation = trainer.generation;

        let dir = temp_dir("checkpoint");
        let path = dir.join("hard.population.json");
        let step = curriculum_step(
            &mut trainer,
            &main,
            &easier,
            2,
            &path,
            0.10,
            "no new best in a long time",
        )
        .expect("a step against a real level succeeds");

        assert_eq!(step.checkpoint, path);
        assert!(path.exists(), "the stuck population is written down");
        let checkpoint =
            crate::population::PopulationFile::load(&path).expect("the checkpoint loads back");
        assert_eq!(checkpoint.generation, generation);
        let mut written = checkpoint.genome_ids.clone();
        written.sort_unstable();
        assert_eq!(
            written, ids,
            "the checkpoint is the population it came from"
        );
        assert!(
            checkpoint.note.contains("stalled"),
            "the checkpoint records why it exists: {:?}",
            checkpoint.note
        );
        assert!(
            checkpoint.note.contains("no new best in a long time"),
            "the stall's own words are kept: {:?}",
            checkpoint.note
        );
        // The level it was taken on is the *main* level, not the one it was just
        // trained on: a resumed run has to be told which level the numbers mean.
        assert!(checkpoint.level_note(&easier).is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_adopted_population_can_still_be_bred_without_issuing_an_id_twice() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let easier = flat_runway("easy", 24, 22);
        let config = tiny();
        let mut trainer = trained(&main, &config);

        let dir = temp_dir("ids");
        let path = dir.join("hard.population.json");
        curriculum_step(&mut trainer, &main, &easier, 2, &path, 0.10, "stalled")
            .expect("a step succeeds");

        // Genome ids are part of every episode seed, so the run has to be able to
        // carry on breeding without handing out an id that is already in the
        // population — which is what `adopt` maintains and what this checks.
        let mut ids = genome_ids(&trainer);
        let unique = {
            let before = ids.len();
            ids.dedup();
            before == ids.len()
        };
        assert!(unique, "the adopted population has duplicate ids: {ids:?}");
        assert!(
            trainer.next_id() > ids.iter().copied().max().unwrap_or(0),
            "the next id {} is not past the highest adopted id",
            trainer.next_id()
        );
        let next = trainer.step(&main);
        assert!(
            next.results.len() == config.pop_size,
            "the run breeds on from the adopted population"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_step_hands_a_changed_population_back_and_leaves_the_runs_clock_alone() {
        let main = gap_runway("hard", 24, 22, &[(8, 3)]);
        let easier = flat_runway("easy", 24, 22);
        let config = tiny();
        let mut trainer = trained(&main, &config);
        let before = genome_weights(&trainer);
        let generation = trainer.generation;

        let dir = temp_dir("changed");
        let path = dir.join("hard.population.json");
        let step = curriculum_step(&mut trainer, &main, &easier, 3, &path, 0.10, "stalled")
            .expect("a step succeeds");

        // The detour's own accounting: three generations were spent, all of them on
        // the easier level.
        assert_eq!(step.generations, 3);
        assert_eq!(step.easier_level, easier.name);
        // The run's own clock is untouched: a detour is not progress on the hard
        // level, and a generation number that jumped by the length of the excursion
        // would claim that three generations of the main run had happened.
        assert_eq!(
            trainer.generation, generation,
            "the excursion must not advance the run's own generation counter"
        );
        // And what comes back is a different population: an excursion on a level with
        // any signal at all has to move it, or the step is a very expensive no-op.
        assert_ne!(
            genome_weights(&trainer),
            before,
            "the population came back unchanged, so the detour did nothing"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_note_says_how_long_it_took_where_it_went_and_that_the_run_is_back() {
        let step = CurriculumStep {
            easier_level: "l1-flat-run".into(),
            generations: 40,
            checkpoint: PathBuf::from("champions/l5.population.json"),
            best: 812.345,
            flag_at: Some(163),
        };
        let note = step.note("l5-tightrope");
        // The fitness is compared against the struct's own number rather than a literal:
        // `{:.2}` of 812.345 is 812.34 or 812.35 depending on which side of the decimal
        // the value's nearest `f32` sits, and a test that pinned one of them would be
        // asserting a rounding rather than the note.
        let best = format!("best there {:.2}", step.best);
        for wanted in [
            "40 generations",
            "l1-flat-run",
            "l5-tightrope",
            "checkpoint -> champions/l5.population.json",
            best.as_str(),
            "flag reached at its generation 163",
        ] {
            assert!(note.contains(wanted), "{note:?} omits {wanted:?}");
        }

        // A detour that solved nothing says so rather than leaving the clause out:
        // the absence of a flag is a fact about the easier level too.
        let unsolved = CurriculumStep {
            flag_at: None,
            ..step
        };
        assert!(unsolved
            .note("l5-tightrope")
            .contains("flag not reached there yet"));
    }

    #[test]
    fn a_run_with_no_population_to_checkpoint_is_an_error_rather_than_a_silent_no_op() {
        // The default rung of the ladder, so the constant the two front-ends share is
        // the one this module's docs promise: a detour long enough to matter.
        assert_eq!(DEFAULT_CURRICULUM_GENERATIONS, 40);

        let main = gap_runway("hard", 16, 14, &[(6, 3)]);
        let easier = flat_runway("easy", 16, 14);
        // Writing into a path whose parent is a *file* cannot work, and the failure
        // has to come back as an error the front-end can put on screen.
        let dir = temp_dir("unwritable");
        let file = dir.join("not-a-directory");
        std::fs::write(&file, b"").expect("a file where a directory should be");
        let mut trainer = trained(&main, &tiny());
        let err = curriculum_step(
            &mut trainer,
            &main,
            &easier,
            1,
            &file.join("hard.population.json"),
            0.05,
            "stalled",
        )
        .expect_err("a checkpoint that cannot be written is an error");
        assert!(err.contains("cannot create"), "{err:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
