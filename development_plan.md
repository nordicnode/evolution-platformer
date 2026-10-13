# Evolution Platformer — Development Plan

**Working title:** TBD (no name picked yet)
**Date:** 2026-10-12
**Author:** Muse (research by two deep-research passes, Oct 2026)
**Status:** Planning only — no implementation yet. This document is written to be handed directly to an AI coding agent.

---

## 1. Concept

A 2D side-scrolling platformer where the player **builds levels** (Super Mario Maker style: tiles, gaps, enemies, hazards, moving platforms, goal flag) and then **watches a population of AI agents evolve** to beat the level. The core loop:

1. Build (or pick) a level.
2. Press "Evolve" — a population of small neural-network controllers trains headlessly, thousands of times faster than realtime.
3. Watch the learning arc: generation counter, fitness charts, death markers, "genius agent" moments, speed controls.
4. A champion emerges that beats the level. Save it, replay it, share it, race it against friends' champions.

The entertainment product is the **visible learning arc** (stands still → walks → jumps → beats the level), not the algorithm. Research found **no shipped game occupying this space** — the closest prior art is icyAI (itch.io, 2021), a single-mechanic Icy Tower clone where you train one NEAT AI. The "AI learns to play X" YouTube genre (Code Bullet et al.) is a durable multi-million-view category and the obvious marketing channel.

---

## 2. Locked decisions (from research)

These are settled. Do not relitigate without new evidence.

1. **Algorithm for MVP: fixed-topology genetic algorithm, NOT full NEAT.** A 2021 Nature Communications robot-swarm study found plain fixed-topology GA tied CMA-ES, xNES, and NEAT; MarI/O's winning network had under a dozen neurons, so topology search buys almost nothing at this scale. Hand-rolling is ~150 lines of Rust. The Rust NEAT crate ecosystem is thin (only live crate is CTRNN-based, not classic NEAT); the JS neataptic library is unmaintained. Add NEAT-style speciation/complexification later **only if** fixed topologies visibly plateau.
2. **Headless custom sim, never realtime training.** MarI/O's famous 24-hour run was an artifact of training inside a realtime emulator. A headless hand-rolled tile sim runs several thousand times faster than realtime; on a 6-core CPU a 300-genome generation should take ~1–3 seconds wall-clock, putting a Mario-scale solve at roughly 1–2 minutes. (Caveat: these are extrapolations from a 2009 benchmark — benchmark the real sim early, see M2.)
3. **Deterministic fixed-timestep simulation, decoupled from rendering.** Same genome + same level must produce identical fitness, or selection is noise. Determinism also gives free replays and verifiable shared champions.
4. **Hand-rolled AABB tile physics, not a physics engine** (no Rapier, no Godot physics). Simpler, fully deterministic, no cross-version drift.
5. **No deep RL, no Unity ML-Agents.** Confirmed overkill: nothing here needs gradients; ES/GA is competitive with PPO at this scale and trivially parallelizable.
6. **GPU is irrelevant.** NEAT-scale evolution over hundreds of tiny nets is CPU-bound. Target dev machine (i5-9400, 6 cores, 16 GB RAM) is plenty.
7. **Ship single-threaded web.** Godot 4.3+ single-threaded web export is the default and works on itch.io with no special headers; threaded WASM needs COOP/COEP headers that itch.io's support for is unverified. Rayon code compiles to WASM and silently falls back to sequential iteration — one codebase, N cores natively, 1 core on web, identical results.
8. **Stall/anti-frustration UX is the differentiator, not algorithm sophistication.** Solve time is dominated by level design, not the algorithm (34 generations vs 111+ on different Mario levels). No prior art exists for "evolution is stuck" UX.

---

## 3. Tech stack

| Layer | Choice |
|---|---|
| Language | Rust (workspace, Cargo) |
| Sim | Hand-rolled, pure-logic crate. No engine. `f32` physics, fixed `DT = 1/60`. |
| RNG | `rand` + `rand_xoshiro` (`Xoshiro256PlusPlus`), seeded per episode. No `thread_rng()` in sim code. |
| Parallelism | `rayon` for population evaluation (Pattern A, §5). Sequential fallback on WASM is automatic. |
| Serialization | `serde` + `serde_json` for levels, genomes, champions. |
| Native renderer (M1–M3) | `macroquad` (fast to working pixels) or a 200-line ASCII terminal view for week 1. |
| Editor (M4) | `egui` tile painter (fastest to functional). Pretty web editor later. |
| Web (M5) | `wasm-bindgen` wrapper around `sim`+`evo`; TypeScript UI (canvas + charts); training in a Web Worker. |
| Version control | git. Commit the determinism hash test early and keep it green forever. |

**Deferred / rejected:** Bevy (net-negative for weeks 1–3: compile time, WASM bloat, determinism footguns — revisit only if its renderer/editor ecosystem is wanted later), Godot 4 + GDExtension (overkill for MVP; only if Godot becomes the long-term renderer), Unity ML-Agents (rejected), fixed-point physics (only if native≡WASM bit-identity is ever required — WASM `f32` is already bit-identical across browsers/OSs per the strict IEEE-754 spec).

---

## 4. Workspace layout

```
workspace/
├── Cargo.toml                  # workspace root
├── crates/
│   ├── sim/                    # THE deterministic core. No rendering, no I/O, no time, no threads.
│   │   │                       # Deps: serde, rand_xoshiro ONLY.
│   │   ├── lib.rs              # pub struct Sim; pub fn step(&mut self, &[AgentAction]); pub const DT
│   │   ├── level.rs            # Level format (serde JSON), tile grid, platform extraction
│   │   ├── physics.rs          # AABB move+collide, enemies, hazards, goal
│   │   ├── sensors.rs          # observation vector builder (VERSIONED — see §8)
│   │   └── rng.rs              # episode_seed(); no global RNG
│   ├── evo/                    # Neuroevolution. Depends on sim only.
│   │   ├── genome.rs           # fixed-topology genome: Vec<f32> weights + biases (+ serde)
│   │   ├── ga.rs               # selection, crossover, mutation; evaluate() uses rayon
│   │   └── fitness.rs          # fitness function (versioned alongside sensors)
│   ├── render/                 # Reads &Sim ONLY. Never mutates sim state.
│   ├── app/                    # egui game shell (M4): pixel renderer, tile painter
│   │                           # and playtest. Depends on sim + render. (Planned as
│   │                           # `editor/`; it grew into the game's window instead.)
│   └── train-cli/              # headless binary: train → champions/*.json. Native speed.
├── web/                        # wasm-bindgen wrapper + TS UI (M5)
└── levels/                     # *.json level files (hand-written in M1–M3, painted in
                                # the editor after that)
```

**Boundary rules (non-negotiable):**
1. `crates/sim` contains no `std::time`, no `HashMap` in hot paths (use `BTreeMap`), no threads, no `println!` in `step`.
2. Everything the sim needs crosses the boundary as plain data: `&Level`, `&[AgentAction]`, `seed: u64`.
3. The renderer only ever takes `&Sim`.
4. Version the trio together and stamp all three into every champion file: `sim_version` + `level_format_version` + `sensor_version`.

---

## 5. Determinism contract

Every item below is a hard rule. The determinism hash test (§11) enforces them.

1. **Fixed timestep:** `pub const DT: f32 = 1.0 / 60.0`. Headless training calls `sim.step()` in a plain loop — no accumulator needed there. The accumulator loop (Gaffer-on-Games style, clamped to avoid spiral of death) is only for visible realtime play/editor.
2. **Floats:** `f32` everywhere in sim. WASM float semantics are strict IEEE-754, so the same WASM build is bit-identical on every browser/OS/CPU. Native vs WASM builds *can* diverge (x86 FMA contraction, libm transcendentals) — accepted for MVP; if cross-build bit-identity is ever required, switch sim arithmetic to fixed-point (`fixed` crate or scaled integers).
3. **No NaN-dependent behavior:** if full state is ever hashed/serialized, canonicalize NaNs first (`if x.is_nan() { 0.0 }`), or assert none occur. NaN payloads are nondeterministic per the WASM spec.
4. **No `HashMap`** in sim hot paths (SipHash has a per-process random seed). Use `BTreeMap` or index-based structures.
5. **One seeded RNG for all gameplay randomness.** `Xoshiro256PlusPlus`, constructed per episode:
   ```rust
   fn episode_seed(master_seed: u64, generation: u32, genome_id: u32, trial: u32) -> u64 {
       // SplitMix64-style avalanche; integer-only, bijective.
       let mut z = master_seed
           .wrapping_add(0x9E3779B97F4A7C15)
           .wrapping_add(((generation as u64) << 32) | genome_id as u64)
           .wrapping_add((trial as u64).wrapping_mul(0xBF58476D1CE4E5B9));
       z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
       z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
       z ^ (z >> 31)
   }
   ```
6. **Parallel determinism (rayon Pattern A):** RNG is constructed *inside* the `par_iter().map()` closure, seeded per genome. `collect::<Vec<_>>()` preserves genome order, so `fitness[i]` always aligns with `genomes[i]` regardless of thread scheduling. Never share one `&mut Rng` across the closure. On `wasm32` without threads, rayon falls back to sequential iteration with no code changes — same results.
7. **No wall-clock, no `thread_rng()`, no relaxed-SIMD vector instructions** in sim code.

---

## 6. Sim spec (M1)

**Player physics (AABB vs tile grid):**
- Run acceleration/friction, jump with fixed initial velocity `v0`, gravity `g` per tick². Coyote time optional (defer unless playtesting demands it).
- Constants are the physics bible — record them in `sim::PHYSICS` and version them. Jump capability derives from the *same* constants (§9): `max_jump_height h = v0²/(2g)`, `air_time t = 2·v0/g`, `max_jump_distance d = run_speed·t`.

**Entities (M1 scope):** player, walker enemy (patrols platform edges), spikes, pits, goal flag. **Defer:** flyers, moving platforms, powerups, springs.

**Tile encoding:** small integer ids (0 = empty, 1 = solid, 2 = spike, 3 = goal, …). Levels stored as JSON: metadata + grid (array of strings, one char per tile — human-writable) + entity list.

**Example level JSON (hand-written in M1):**
```jsonc
{
  "format": "evo-platformer-level",
  "level_format_version": 1,
  "name": "L2-first-gap",
  "width": 60, "height": 15,
  "spawn": {"x": 2, "y": 12},
  "tiles": [
    "............................................................",
    // ... one string per row
  ],
  "entities": [
    {"type": "walker", "x": 30, "y": 12},
    {"type": "goal", "x": 57, "y": 12}
  ]
}
```

**Three hand-written test levels (M1):**
- **L1** "flat-run": flat ground, spawn → goal, no obstacles. (Smoke test: GA should solve in a handful of generations.)
- **L2** "first-gap": one pit + one staircase + elevated goal.
- **L3** "gauntlet": gap + walker enemy + elevated goal. (The M2 stretch goal.)

---

## 7. Evolution spec (M2)

### 7.1 Genome (fixed topology)

```rust
pub struct Genome {
    pub id: u32,
    pub weights: Vec<f32>,  // row-major: input→hidden, then hidden→output
    pub biases: Vec<f32>,
    pub fitness: f32,
}
```

### 7.2 Sensor vector v1 (~90 inputs, `sensor_version = 1`)

| Component | Count | Encoding |
|---|---|---|
| Tile patch around agent | 81 (9×9) | solid = 1, enemy = −1, empty = 0 |
| Nearest enemy dx, dy | 2 | normalized by view distance |
| Gap-ahead signal | 1 | boolean: no ground within N tiles ahead at foot level |
| Grounded | 1 | boolean |
| Velocity vx, vy | 2 | normalized |
| Can-jump (coyote/grounded) | 1 | boolean |
| Goal direction x sign | 1 | −1 / +1 |
| Bias | 1 | constant 1.0 |
| **Total** | **90** | |

Rationale: Togelius 2009 showed 21-input controllers beating 101-input ones — input size multiplies the search space. Start at ~90; widen the patch only if evolution plateaus on levels that should be solvable. (heaven-hm's 185-input recipe — 13×13 patch + 15 features — is the documented upgrade path.)

### 7.3 Actions (6 discrete macro-actions)

Outputs are **argmax over 6 action scores** (simpler and more evolvable than binary button thresholds):
`run`, `run_jump`, `retreat`, `brake`, `jump_in_place`, `walk`.

**Action repeat:** re-query the network every **4 physics ticks** (60 Hz sim → a decision every ~67 ms), repeating the chosen action between decisions. Expose as a tuning knob (3–6 range).

### 7.4 Fitness function v1 (`fitness_version = 1`)

Additive, evaluated per episode:
1. **Progress (primary):** `+1.0` per new rightmost pixel reached.
2. **Survival:** `+0.05` per frame alive (small — rewards staying alive without rewarding camping).
3. **Death:** `−120`.
4. **Goal:** `+10,000` on touching the flag — must dominate any progress-only local optimum.
5. **No-progress timeout:** episode ends after **600 frames** with no new rightmost-x. (Without this, campers burn the whole compute budget standing still.)
6. **Episode cap:** `level_width_tiles × 40` frames (scales with level length).

**Anti-exploit rules** (from "Surprising Creativity of Digital Evolution"): reward *task progress + survival*, never proxies like jump height or airtime — agents will farm proxies (static towers, somersaulting). Treat "the AI broke my physics" as a QA feature: run champions against candidate physics builds.

**Optional shaping v1.1** (only if v1 stalls): klingj3-style per-frame `10·vx + 5·kills + 50·height_gain_on_landing`, mild negative on damage/leftward movement, and `|xChange|` scoring while stuck so backing up to reassess scores positively.

### 7.5 GA hyperparameters (v1)

| Parameter | Value |
|---|---|
| Population | 200 native (64–128 on web) |
| Selection | tournament, size 3–5 |
| Elitism | 4 (top genomes carried over unchanged) |
| Crossover | uniform per-weight |
| Mutation | Gaussian, σ 0.2–0.5; per-weight mutation rate 0.03–0.10 (expose as live slider in watch mode) |
| Network | 90 → 12–16 (tanh) → 6 (tanh, argmax) |
| Speciation | **No** (defer to M5, only on observed premature convergence) |

### 7.6 Champion file format

```jsonc
{
  "format": "evo-platformer-champion",
  "version": 1,
  "network": {
    "n_inputs": 90, "n_hidden": [16], "n_outputs": 6,
    "activation_hidden": "tanh", "activation_output": "tanh",
    "weights": [0.123, -0.456],
    "biases": [0.01]
  },
  "fitness": 10234.5,
  "metadata": {
    "generation": 87,
    "genome_id": 42,
    "level_hash": "sha256:…",
    "level_format_version": 1,
    "sim_version": "0.3.1",
    "sensor_version": 1,
    "fitness_version": 1,
    "episode_seed": 987654321,
    "frames": 5210,
    "training_config": {"pop_size": 200, "mutation_sigma": 0.3, "elite": 4}
  }
}
```

Every field in `metadata` is required for bit-identical re-simulation by another player. Refuse to load champions whose versions mismatch the running sim.

---

## 8. Sensor/fitness versioning rule

`sensors.rs` and `fitness.rs` each carry a `VERSION` constant. Any change to input layout, action mapping, or fitness terms bumps the version, and champion files record all three versions. A champion is only loadable when `sim_version`, `sensor_version`, and `fitness_version` all match. This prevents silent "my old champion doesn't work anymore" corruption.

---

## 9. Level validation — jump-reachability oracle (M2)

Before training ever runs, the editor answers "is this level beatable at all" in milliseconds:

1. **Extract platforms:** scan the tile grid for maximal horizontal runs of solid tiles with empty space above. Each gets an id, x-interval `[x0, x1]`, surface height `y`. Ground counts as one long platform.
2. **Jump capability** from the sim's own physics constants: `h = v0²/(2g)`, `d = run_speed · 2v0/g`.
3. **Directed graph:** nodes = platforms. Edge A→B iff a canonical jump trajectory (still/walk/run takeoff × short/full jump-hold, simulated with the real collision query, ~50 lines) launched from A's edges lands on B's surface without intersecting solid tiles. **Directed:** jumping down is always possible; jumping up is limited by `h`.
4. **BFS** from the spawn platform to the goal platform. Reachable = beatable-in-principle. On failure, highlight the first unreachable jump in the editor.

**Deferred:** Morai Maker-style "survival analysis" difficulty estimation (N suboptimal agents attempt an A* path; survival fraction ≈ difficulty). In this game the evolving population is itself the difficulty oracle — "generation at which the population first beats the level" is a free, better-grounded signal. Revisit only if a pre-training difficulty number is needed.

---

## 10. Watch-mode UX spec (M3)

Priority-ordered. Tier 1 is the M3 exit gate.

**Tier 1 — non-negotiable:**
1. Generation counter + alive/total agents, always visible.
2. Max/avg fitness line chart (both rising = learning; max-only rising = few good agents; flat = converged; falling = degenerating).
3. Per-generation progress bars + **"genius agent" splash** when `max_fitness(gen) > 1.5 × max_fitness(gen-1)` — the cheapest drama-generator in the genre.
4. Speed control 1×–50× (physics steps per rendered frame) + pause/resume.
5. Render **best-4 + random-4** genomes per generation (champion + diversity in one view).

**Tier 2 — watchable past 10 minutes:**
6. Death markers: × at each agent's death position, fading over generations — instantly shows *where* the population is stuck.
7. Timeline scrubber: record champion trajectory per generation (positions every N frames); scrub back to *see* the moment the jump was learned.
8. Champion save/load JSON (§7.6) + "watch champion replay" mode.
9. Mutation-rate slider (0.03–0.10) — the boxcar2d lesson: tuning-while-watching is itself fun.
10. CSV export of (generation, max, avg, max score).

**Tier 3 — depth (M5+):** species coloring + legend (NEAT phase), weight heatmaps, per-agent genome inspector, "only draw on improvement" mode for unattended runs, live neural-net visualization.

**Architecture note:** never train on the UI thread. Native: rayon pool + main render thread, snapshots a few times per second. Web: WASM sim+GA in a Web Worker ↔ main thread canvas/charts via message passing.

---

## 11. Testing strategy

| Test | What it enforces |
|---|---|
| `determinism_hash` | Step the sim 10k frames twice from the same seed → identical state hash. **The project's spine. Keep green forever; run in CI.** |
| `episode_seed_stability` | Same (master, generation, genome_id, trial) → same RNG stream across runs. |
| `parallel_determinism` | rayon evaluation with 1 vs 6 threads → identical fitness vectors. |
| Physics unit tests | AABB collision corners: landing, head-bump, wall-slide, one-way platforms (if added), enemy stomp vs side-hit, spike death, goal trigger. |
| Reachability tests | Hand-built levels: known-beatable → BFS passes; known-impossible (gap wider than `d`, wall taller than `h`) → BFS fails with the right culprit jump. |
| GA smoke test | L1 flat-run level: population reaches the goal within N generations (loose bound — guards against fitness/sign bugs, not a performance benchmark). |
| Champion round-trip | Save champion → load → re-simulate episode → identical trajectory hash. |
| Sensor snapshot | Fixed level + fixed agent state → fixed observation vector (catches accidental sensor layout changes; bump `sensor_version` instead). |

---

## 12. Milestones

### M0 — Scaffolding (~2–3 days)
- Cargo workspace with `crates/sim` (+ `evo`, `render`, `train-cli` shells), `.gitignore`, README.
- `sim`: `Level` JSON load, tile grid, `Sim::new`, `step()` skeleton, `DT` const, `rng.rs` with `episode_seed()`.
- Determinism hash test written first, failing, then passing.
- **Exit:** `cargo test` green; `train-cli --help` runs.

### M1 — Sim + hand-written levels (week 1)
- AABB player physics (run/jump/gravity), walker enemy, spikes, pits, goal flag. All constants in one versioned `PHYSICS` module.
- JSON level format (§6) + L1/L2/L3 hand-written.
- ASCII terminal renderer or minimal macroquad view — just enough to *see* the sim.
- Human keyboard playtest of L1–L3 (sanity: levels are actually beatable by a human).
- **Exit:** all physics unit tests + determinism hash test green; a human can beat L1–L3.

### M2 — GA + headless training + reachability oracle (week 2)
- `evo` crate: genome, tournament selection, uniform crossover, Gaussian mutation, elitism, rayon evaluation (Pattern A).
- Sensors v1 (~90 inputs), 6 macro-actions, action repeat every 4 ticks, fitness v1 (§7.4).
- `train-cli`: trains on a level, prints gen/max/avg per generation, writes `champions/*.json`.
- **Benchmark the real numbers:** record seconds-per-generation for pop 200 on the dev machine. If far off the ~1–3 s estimate, profile before proceeding.
- Reachability BFS (§9) as a `train-cli` subcommand; run over L1–L3.
- **Exit:** beats L1 and L2 in ≤5 minutes wall-clock on 6 cores; determinism + parallel-determinism tests green; reachability agrees with human playtesting on L1–L3.

### M3 — Watch mode + fun gate (week 3) ⭐ THE DECISION POINT
- Watch mode: Tier 1 UX (§10) — best-4 + random-4 rendering, generation counter, fitness chart, speed control, pause, death markers, genius-agent splash.
- Champion save/load + replay mode.
- **The fun question:** build one fresh level, watch evolution for 10+ minutes. **GO/NO-GO gate:** if watching isn't fun, the fix is in UX (speed, markers, charts, drama), not a bigger network. Do not proceed to M4 on a "no-go" without a UX redesign.
- **Exit:** fun-gate passed (owner's call); Tier 1 watch UX complete; champion round-trip test green.

### M4 — Editor + content (2–3 weeks) — started
The window, the painter, the playtest and live validation are in `crates/app`; the new
entity types and the curated pack are open. README findings 24–25 record the choices.
- egui tile painter: paint/erase tiles, entity palette (walker, spikes, goal, spawn), undo, save/load JSON. **Done** — drag out of the palette onto the level, plus grid resize; saving goes through `sim`'s writer, which can only emit documents the loader accepts.
- Human playtest mode inside the editor. **Done** — in-canvas, and it shares `render::play::action_for` with the terminal mode so the two cannot diverge.
- Reachability validation live in the editor (warn on unreachable goal, highlight first failing jump). **Done** — the verdict, the culprit jump and the unreachable tint, re-run whenever the level's content hash changes.
- New entities: flyer enemy, moving platform (horizontal/vertical, deterministic — driven by tick count, not wall-clock). **Open.**
- Curated starter level pack (8–12 levels, easy→brutal) built with the editor. **Open** — L1–L6 are hand-written so far.
- **Exit:** a non-developer can build a level, validate it, and evolve a solver without touching JSON.

### M5 — Evolution polish + web + release (2–3 weeks)
- **Only if M2/M3 showed the need:** NEAT-style speciation (compatibility distance, fitness sharing) or wider sensors — evidence-gated, not scheduled.
- Stall UX: stall detector (best-fitness flat for K generations) → death-spot highlight → **curriculum resume** (restart training from a checkpoint at the stuck point) → honest fitness charts. Cap generations; keep best-so-far; frame stalling as level-difficulty information.
- Timeline scrubber (Tier 2).
- `web/`: wasm-bindgen wrapper, TS UI (canvas + charts), training in Web Worker, level share via URL hash (RLE tile grid + entity list compresses to KBs), champion JSON download.
- itch.io page: HTML5 upload (ZIP ≤1000 files, ≤500 MB extracted — a WASM game is orders of magnitude under), devlog-style trailer in the Code Bullet genre grammar.
- **Exit:** shippable itch.io release; a stranger can open the page, build a level, and watch AI learn to beat it.

### Deliberately deferred (do not touch before M3's fun gate)
NEAT/speciation · survival-analysis difficulty · online champion sharing portal · sound/music · mobile · powerups · achievements/tutorial · threaded WASM/SharedArrayBuffer.

---

## 13. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Watching gets boring (the #1 risk) | M3 fun gate; invest in watch UX (Tier 1→2), not algorithms. Speed control + death markers + genius splashes are the levers. |
| Hard levels stall forever | Stall detector + curriculum resume + reachability oracle; frame stall as difficulty info, never brick the player's level. |
| Scope creep on editor features | Editor is M4, *after* the fun gate. Weeks 1–3 use hand-written JSON. |
| Physics exploits (agents farm collision bugs) | Treat as QA: run champions against candidate physics builds; fix the physics, don't patch the fitness. |
| Determinism regressions | `determinism_hash` test in CI, kept green forever; the boundary rules (§4) are review checklist items. |
| Premature convergence | Tournament selection + elitism + healthy mutation first; speciation only on evidence. |
| Web training too slow (1 core) | Cap web population (64–128), short episodes, fast sim; single-threaded WASM is still ~100× realtime. Threads only if measured wall-time exceeds ~20 s/generation. |

---

## 14. Open decisions (owner's call)

1. **Name.** Nothing picked yet.
2. **GA vs NEAT day one.** Research recommends fixed-topology GA; starting with minimal NEAT is defensible if topology growth is intrinsically motivating. Owner's call on where the fun is.
3. **Sensor size:** 9×9+features (~90 inputs, faster learning) vs 13×13+features (185 inputs, sees farther). Only experiments on real levels settle it — start at ~90.
4. **What "beaten" means:** flag-touch only, or flag + time bonus + style (fewest jumps, no damage)? The fitness function *is* the game design for the AI audience.
5. **Editor ambition:** egui-painter-for-self vs Mario-Maker-polish-for-players — roughly a 10× cost difference. Web canvas editor is the best end-state; egui is the fastest functional one.
6. **Art direction:** programmer-art tiles vs a real tileset (LDtk/Tiled-compatible). Affects editor choice and itch.io appeal, not the sim.
7. **Cross-platform determinism:** WASM-identical (f32, easy) vs native≡WASM bit-identical (fixed-point, real work). Needed only if champions must replay across desktop and browser builds.

---

## 15. Reference codebases (study first, in this order)

1. **heaven-hm/mario-ai-neat** — the sensor/fitness bible. Read `docs/learning.md` first (observation/action/fitness spec), then sensor construction + fitness function in the Lua sources. https://github.com/heaven-hm/mario-ai-neat
2. **FlappyBird-AI-EvoLab** — the save/load/share-champion loop to copy ("Save Best Model" → JSON, "Load Model & Play", CSV export) and the fitness-chart reading guide. https://github.com/thieuquangphuoc/FlappyBird-AI-EvoLab
3. **moagilah/ai-game-controller** — C++ custom platformer + NEAT dissertation project; study `AIController.{cpp,h}` and `NEAT/CParams.cpp` for environmental-inputs→network→controls architecture and determinism discipline. https://github.com/moagilah/ai-game-controller
4. **cookiengineer/goneat** (Go) — JSON snapshots including RNG state so runs resume exactly; deterministic vs concurrent runners. The save/resume pattern to copy. https://github.com/cookiengineer/goneat
5. **outerframehq/frame-engine** — `DESIGN.md`: "simulation is pure logic over state," engine-as-own-crate. The architectural north star. https://github.com/outerframehq/frame-engine/blob/HEAD/DESIGN.md
6. **swastikjha008-jpg/Neuroplay-lab** — live NN visualization + snapshot streaming ideas for the web UI. https://github.com/swastikjha008-jpg/Neuroplay-lab
7. **lkdev03/8bit-mario-rl** — *unverified*: could not confirm this repo's contents during research (search tool failure). Confirm the URL manually before citing; described as having per-generation best-K/random-K rendering and training checkpoints.

---

## 16. Effort estimate

- **M0–M3 (fun-validation prototype): ~3 weeks** part-time.
- **M4–M5 (shippable itch.io release): ~5–8 more weeks.**
- **Total: ~8–11 weeks part-time** to a shippable release, assuming the M3 fun gate passes.

---

*Research sources: two deep-research passes (Oct 2026). Raw notes live under `~/workspace/research_notes/neat-evolution-platformer-game-research-20261013-0003/` and `~/workspace/research_notes/neat-platformer-implementation-deep-dive-20261013-0013/`. Full findings also rendered as a web report page (artifact `neat-platformer-game-research`).*
