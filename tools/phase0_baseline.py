#!/usr/bin/env python3
"""Phase 0's baseline gate: the pack, the battery and the generated suite, both arms.

The question this answers is the one a fitness scalar cannot: **does a run learn, or does it
scout?** For every level it records, from the run's own learning telemetry (`--telemetry`):

* solves over the seeds (and the generation and episode cost of the first win),
* the clock among the winners (best and median),
* per-leg success at the end of the run: what share of the population crossed each of the first
  three route legs,
* the first-action entropy at spawn, at generation 0 and at the end,
* the timed planner's cost for that level (the Phase 1 route compiler's stand-in).

Arms:

* **default** — the shipped search, pop 200, at most 100 generations, stopped on the first
  generation that solved. One episode per genome (`--no-auto-trials --trials 1`) so the episode
  cost is the same on every level and the table compares searches rather than patrol budgets, and
  `--curriculum-generations 0` so no detour rewrites the landscape mid-run.
* **random** — the same population and level with **one** generation, so its row is what an
  unevolved population does: the floor the default has to beat.

Usage:
    cargo build --release -p train-cli
    python3 tools/phase0_baseline.py [--out OUT] [--gens N] [--seeds 1,2]

The full per-level JSON walks are written under OUT (default `out/phase0`), and the markdown
table is printed to stdout. `out/` is gitignored: the record is FINDINGS.md, and this script is
how the record is taken again.
"""

import argparse
import json
import pathlib
import re
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "train-cli"
POP = 200


def level_sets():
    pack = sorted((ROOT / "levels").glob("l*.json"))
    battery = sorted((ROOT / "levels").glob("bat-*.json"))
    practice = sorted((ROOT / "levels" / "practice").glob("*.json"))
    return [("pack", pack), ("battery", battery), ("generated", practice)]


def run(level, seed, generations, outdir, stop):
    tele = outdir / f"{level.stem}-{seed}.jsonl"
    cmd = [
        str(BIN), "train",
        "--level", str(level),
        "--pop", str(POP),
        "--seed", str(seed),
        "--generations", str(generations),
        "--no-auto-trials", "--trials", "1",
        "--curriculum-generations", "0",
        "--quiet",
        "--out", str(outdir / "champions"),
        "--telemetry", str(tele),
    ]
    if stop:
        cmd.append("--stop-on-win")
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(f"{level.name} seed {seed}: {proc.stdout}\n{proc.stderr}")
    if not tele.exists():
        sys.exit(f"{level}: no telemetry written")
    rows = [json.loads(line) for line in tele.read_text().splitlines() if line.strip()]
    if not rows:
        sys.exit(f"{level}: empty telemetry")
    return rows


def first_win(rows):
    for row in rows:
        if row["wins"] > 0:
            return row["generation"], (row["generation"] + 1) * POP
    return None, None


def fmt_clock(frames):
    return "—" if frames is None else f"{frames / 60:.2f}s"


def leg_shares(row, upto=3):
    legs = row["legs"]
    if not legs:
        return "—"
    shares = []
    for leg in range(min(upto, legs)):
        through = row["reached_legs"][leg]
        shares.append(f"{100.0 * through / max(1, row['pop']):.0f}")
    return "/".join(shares)


def plan_times():
    """Timed-planner cost in ms, per level stem, for the whole suite.

    The practice suite comes from `fuzz` itself (depth 48, the suite's own budget). The pack and
    the battery are planned by `reach --timed` at the shipped default depth, because that is the
    question the shipped command asks. Both are the Phase 1 compiler's stand-in, and the column
    says which depth it was taken at in the table's footnote.
    """
    times = {}
    fuzz = subprocess.run([str(BIN), "fuzz"], capture_output=True, text=True)
    if fuzz.returncode != 0:
        sys.exit(f"fuzz failed:\n{fuzz.stdout}\n{fuzz.stderr}")
    for line in fuzz.stdout.splitlines():
        fields = line.split()
        if len(fields) >= 7 and re.fullmatch(r"p\d\d", fields[0][:3]):
            try:
                times[fields[0]] = float(fields[-1])
            except ValueError:
                pass
    for path in sorted((ROOT / "levels").glob("l*.json")) + sorted(
        (ROOT / "levels").glob("bat-*.json")
    ):
        proc = subprocess.run(
            [str(BIN), "reach", "--level", str(path), "--timed"],
            capture_output=True, text=True,
        )
        match = re.search(r"\((\d+) expansions, ([\d.]+)s\)", proc.stdout)
        if match:
            times[path.stem] = float(match.group(2)) * 1000.0
    return times


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default=str(ROOT / "out" / "phase0"))
    ap.add_argument("--gens", type=int, default=100)
    ap.add_argument("--seeds", default="1,2")
    args = ap.parse_args()
    outdir = pathlib.Path(args.out)
    outdir.mkdir(parents=True, exist_ok=True)
    seeds = [int(s) for s in args.seeds.split(",")]
    if not BIN.exists():
        sys.exit(f"missing {BIN}; run `cargo build --release -p train-cli`")
    plans = plan_times()

    print(
        "| level | D solved | D first win | D clock best/med | D legs 0/1/2 % | "
        "D entropy 0→end | R solved | R legs 0/1/2 % | R entropy | plan ms |"
    )
    print("|---|---|---|---|---|---|---|---|---|---|")
    summary = {}
    for title, levels in level_sets():
        if not levels:
            continue
        print(f"| **{title}** ({len(levels)} levels) | | | | | | | | | |")
        solved_default = 0
        solved_random = 0
        for level in levels:
            default_rows = []
            per_seed_best = []
            per_seed_median = []
            per_seed_first = []
            for seed in seeds:
                rows = run(level, seed, args.gens, outdir, stop=True)
                default_rows.append(rows)
                gen, cost = first_win(rows)
                if gen is not None:
                    per_seed_first.append((gen, cost))
                    per_seed_best.append(min(r["best_frames"] for r in rows if r["best_frames"]))
                    med = rows[-1]["median_frames"]
                    if med is not None:
                        per_seed_median.append(med)
            random_rows = run(level, 99, 1, outdir, stop=False)
            rr = random_rows[-1]

            solved = len(per_seed_first)
            solved_default += solved
            if rr["wins"] > 0:
                solved_random += 1

            if per_seed_first:
                gen = int(statistics.median(g for g, _ in per_seed_first))
                cost = (gen + 1) * POP
                first = f"g{gen} / {cost} ep"
            else:
                first = "—"
            clock_best = fmt_clock(min(per_seed_best)) if per_seed_best else "—"
            clock_med = (
                fmt_clock(int(statistics.median(per_seed_median))) if per_seed_median else "—"
            )
            # The deepest-reaching seed's final row: a level two seeds stalled on should not be
            # summarised by whichever one happened to get least far.
            final = max(default_rows, key=lambda rows: rows[-1]["best_end_leg"])[-1]
            entropy = f"{default_rows[0][0]['first_action_entropy']:.2f}→{final['first_action_entropy']:.2f}"
            print(
                f"| {level.stem} | {solved}/{len(seeds)} | {first} | {clock_best} / {clock_med} | "
                f"{leg_shares(final)} | {entropy} | "
                f"{'yes' if rr['wins'] else 'no'} | {leg_shares(rr)} | "
                f"{rr['first_action_entropy']:.2f} | {plans.get(level.stem, float('nan')):.0f} |"
            )
        summary[title] = (solved_default, len(levels) * len(seeds), solved_random, len(random_rows))
    print()
    for title, (s, total, rs, _) in summary.items():
        print(f"{title}: default {s}/{total} seed-runs solved; random {rs} level(s) won at gen 0")
    print(
        f"\nprotocol: pop {POP}, one episode per genome, <= {args.gens} generations, "
        f"default stopped on the first solving generation, curriculum off; random is one generation."
    )
    print(
        "plan ms: the timed planner, depth 48 for the generated suite (its certification budget), "
        "shipped default depth for the pack and battery. Legs and entropy are the deepest-reaching "
        "seed's final row; the clock's median is the median of the seeds' final-row median clocks."
    )


if __name__ == "__main__":
    main()
