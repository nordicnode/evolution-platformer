<p align="center">
  <img src="docs/images/logo.png" alt="One Two Hundred" width="720">
</p>

# One Two Hundred

A 2D platformer you build yourself, and an AI that learns to beat what you built.

Draw a level, play it yourself, then hand it to a population of 200 neural
networks and watch them evolve: the first generations stand still or walk into
pits, and a few hundred generations later one of them runs the level clean.
Everything happens in one window, and the training can also run headless from
the terminal.

## Screenshots

![Title screen](docs/images/title-screen.png)

*Title screen.*

![The blocks and the cast](docs/images/blocks-gallery.png)

*The blocks and enemies a level is built from.*

![Editor, on the showcase level](docs/images/editor-showcase.png)

*The editor, on `levels/scratch-showcase.json`, a level that uses every block
and enemy at once.*

![Evolve view: a run in progress on L3](docs/images/evolve.png)

*Evolve view: 20 generations into a run on L3, with the fitness chart below.*

![Replay view: the best agent, with the population ghosted beside it](docs/images/replay.png)

*Replay view: the best agent's run, with the rest of its generation shown as ghosts.*

## How it plays

**Edit.** Draw a level with the mouse: drag blocks, spikes, springs, enemies
and the flag out of the palette onto the canvas. A status line tells you
whether the level can be beaten at all before you waste time on it.

**Play.** Run the level yourself, against the clock. Once the AI has a best
time on the level, the playtest races you against it, and you can turn on a
ghost of the record-holding run to chase.

**Evolve.** Start a run and watch the population learn the level live: how
far the best agents get, how many have reached the flag, and the quickest
win so far. Save the best agent as a champion, carry it into the next run,
or open the replay to watch its winning run with the rest of its generation
ghosted beside it.

## Features

- **Level editor** with a parts palette, undo/redo, eyedropper, line and
  rectangle tools, a browsable shelf of levels, and a mirror tool that flips
  the whole level in one step.
- **Four building materials** (grass, sand, brick, timber), all solid, plus
  one-way platforms and moving platforms with a visible travel guide.
- **Trap tiles:** cracked ground that crumbles shortly after something stands
  on it, and spring pads that throw whatever lands on them.
- **Elements:** water you swim in, lava that kills and throws fireballs, and
  vines you climb.
- **Enemies:** patrolling walkers, chasers that hunt you and stop at ledges,
  flyers on a set flight path, and turrets that shoot along their row.
- **A reachability check on every level**, so the editor can say "beatable"
  or point at the jump that cannot be made.
- **Evolution you can watch:** a live fitness chart, champions you can save,
  continue from and race against, and replays with ghost populations.
- **Runs that get unstuck:** if a population stalls, the run can fall back to
  an easier version of the level, learn there, and carry what it learned back.
  Once a level is solved, a mastery mode keeps evolving for a faster time
  instead of stopping.
- **A planner, as well as a learner:** the deliberate controller (`T` in the
  playtest) works out a route through the level with the game's own physics
  and plays it, which answers levels whose trick is timing or waiting.
- **Headless training** with `train-cli`: train, watch, replay champions,
  check levels, and run seed benchmarks, all from the terminal. A run started
  in the window can be resumed in the terminal and the other way round.

## The levels

Twelve levels ship with the game, each teaching one thing:

| Level | What it teaches |
|---|---|
| `l1-flat-run` | Nothing. A flat run to the flag |
| `l2-first-gap` | The first pit, and jumping on purpose |
| `l3-gauntlet` | Pits and spikes in sequence |
| `l4-rhythm` | A jump that has to be timed |
| `l5-tightrope` | Narrow ground and landing precision |
| `l6-marathon` | A long level, start to finish |
| `l7-ascent` | Wall jumps, as the way up |
| `l7-wall-school` | Wall jumps with nothing else going on |
| `l8-long-jump` | Sprinting, and the gap only a sprint jump clears |
| `l9-wait-and-climb` | A chain of everything, including waiting for a patrol |
| `l10-the-hunt` | Enemies: flyers, a chaser, a turret, a moving platform |
| `l11-the-elements` | Water, lava and vines |

There is also a set of small adversarial levels (`levels/bat-*.json`), one per
skill the AI is meant to have, and working levels named `levels/scratch-*.json`,
including the showcase level in the screenshot above.

## Quick start

You need a Rust toolchain. From the top of the repo:

```bash
cargo build --release

# The game: title screen, then the editor on the first level
cargo run --release -p app

# Open a level straight into a run, a playtest, or a replay
cargo run --release -p app -- --evolve --level levels/l2-first-gap.json
cargo run --release -p app -- --play --level levels/l2-first-gap.json
cargo run --release -p app -- --replay --generations 20 --level levels/l3-gauntlet.json

# Headless: train a level, check whether a level can be beaten at all
./target/release/train-cli train --level levels/l3-gauntlet.json --generations 200 --pop 200 --stop-on-win
./target/release/train-cli reach --level levels/l11-the-elements.json

cargo test --workspace   # the test suite
```

`train-cli --help` lists everything the terminal can do.

## Controls

**Title screen:** up/down and Enter choose, `P` goes straight to the
playtest, `E` straight to a run.

**Edit:** click paints, drag a palette item to place it, right-drag erases,
wheel zooms, middle-drag (or alt-drag) pans, ctrl+click picks the tile under
the pointer as your brush, shift-drag draws a line. `Ctrl+S` saves, `Ctrl+Z`
and `Ctrl+Y` undo and redo, `G` toggles the grid, `F` fits the level, `P`
plays.

**Play:** arrows or WASD move and jump, `R` restarts, `G` toggles the
record-holder ghost, `T` hands control to the planner, `P` or Esc goes back
to the editor.

**Evolve:** space pauses, `N` steps one generation, `-` and `+` change the
speed, `S` saves the champion, `C` continues from a saved champion, `V`
opens the replay, `F` toggles following the leader.

Press `?` in any view for the full key card for that view.

## More detail

This page is the short version, on purpose. The measurements, the numbered
findings and the version history of the physics, sensors and scoring are in
[FINDINGS.md](FINDINGS.md), and the forward plan for the AI is in
[AI_IMPROVEMENT_PLAN.md](AI_IMPROVEMENT_PLAN.md).

The game is in active development (currently sim 0.16.0, level format v3).
Champion files record the exact physics and scoring they were trained under
and are refused, rather than mis-replayed, if the game has moved on since.
