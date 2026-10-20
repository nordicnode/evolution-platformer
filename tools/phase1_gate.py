#!/usr/bin/env python3
"""Phase 1's acceptance gate: the route plan, and whether warming from it beats the alternatives.

Two parts, both re-runnable:

**A. The artifact.** Compile a `RoutePlan` for every certified practice level at depth 64 and check
the boundary arithmetic on the written JSON rather than trusting the printer:

* one leg per route crossing, and as many legs as the level's route has rungs;
* legs contiguous in blocks, the first starting at block 0 and the last ending at `actions.len()`;
* the legs' frames summing to the plan's frames, and the last leg's margin equal to the plan's own
  win margin (`budget_frames - frames`);
* margins non-increasing along the plan.

It also asks for the plan of a level the oracle certifies but the planner cannot route
(`l9-wait-and-climb`) and requires the loud refusal, not a plan.

**B. The warm start.** On the suite's hard tail and the two pack levels the plan exists for, run
the Phase 0 protocol with three arms and record the first generation that solved:

* `random` — no warm start;
* `route`  — `--from-route`, the tick-indexed walk the plan replaces;
* `plan`   — `--from-plan` at the window's own fit budget (300 epochs) and plan depth (64).

The arms differ only in their warm start: same population, same seed, same episode policy. The
reading is the one that matters for a warm start — solved at generation 0, and generations to the
first solve — not the fitness scalar.

Usage:
    cargo build --release -p train-cli
    python3 tools/phase1_gate.py [--gens N] [--seeds 1] [--out DIR]

The plans, the runs' JSONL and the champions land under `out/phase1` (gitignored); the record is
FINDINGS.md.
"""

import argparse
import json
import pathlib
import statistics
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "train-cli"
POP = 200
PLAN_DEPTH = 64  # deep enough for l11's 11-leg route; l9 is still refused at 128.
WINDOW_EPOCHS = 300  # app::lab's `ImitateConfig::default()`; the CLI's --from-plan default is 10x.

HARD_TAIL = [
    "p02-spike-t0",
    "p14-spike-t2",
    "p17-mover-t2",
    "p18-mixed-t2",
    "p19-gap-t3",
    "p20-spike-t3",
    "p22-crumble-t3",
    "p23-mover-t3",
]
PACK = ["l9-wait-and-climb", "l11-the-elements"]


def run(cmd):
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(f"{' '.join(cmd)}\n{proc.stdout}\n{proc.stderr}")
    return proc.stdout


def part_a(outdir):
    plans = outdir / "plans"
    run([
        str(BIN), "routeplan", "--all", "--levels-dir", str(ROOT / "levels" / "practice"),
        "--plan-depth", str(PLAN_DEPTH), "--out", str(plans),
    ])
    # The pack levels the phase is about: `l11` must compile, `l9` must be refused loudly (it is
    # the oracle-certified-but-unroutable control, asserted below).
    run([
        str(BIN), "routeplan", "--level", str(ROOT / "levels" / "l11-the-elements.json"),
        "--plan-depth", str(PLAN_DEPTH), "--out", str(plans),
    ])
    rows = []
    for path in sorted(plans.glob("*.routeplan.json")):
        plan = json.loads(path.read_text())
        legs = plan["legs"]
        assert legs, f"{path.stem}: no legs"
        assert legs[0]["first_block"] == 0, f"{path.stem}: first leg does not start at 0"
        assert legs[-1]["end_block"] == len(plan["actions"]), f"{path.stem}: tail block missing"
        assert len(legs) == plan["hops"], (
            f"{path.stem}: {len(legs)} legs for {plan['hops']} rungs; a leg is a span"
        )
        assert plan["covered"] <= plan["hops"], f"{path.stem}: more crossings than rungs"
        assert sum(leg["hops"] for leg in legs) == plan["covered"], f"{path.stem}: hops do not add up"
        assert sum(leg["frames"] for leg in legs) == plan["frames"], f"{path.stem}: frames do not add up"
        assert legs[-1]["margin_frames"] == plan["budget_frames"] - plan["frames"], (
            f"{path.stem}: the last margin is not the plan's own"
        )
        previous_margin = 1 << 31
        previous_end = 0
        for leg in legs:
            assert leg["first_block"] == previous_end, f"{path.stem}: legs are not contiguous"
            assert leg["hops"] in (0, 1), f"{path.stem}: a leg closes at most one crossing"
            assert leg["margin_frames"] <= previous_margin, f"{path.stem}: margin grew"
            previous_margin = leg["margin_frames"]
            previous_end = leg["end_block"]
        rows.append({
            "stem": path.stem.replace(".routeplan", ""),
            "legs": len(legs),
            "rungs": f"{plan['covered']}/{plan['hops']}",
            "blocks": len(plan["actions"]),
            "frames": plan["frames"],
            "margin": legs[-1]["margin_frames"],
            "first_margin": legs[0]["margin_frames"],
        })

    # The oracle-certifiable level the planner cannot route: the refusal must name the case.
    refusal = subprocess.run(
        [str(BIN), "routeplan", "--level", str(ROOT / "levels" / "l9-wait-and-climb.json"),
         "--plan-depth", str(PLAN_DEPTH)],
        capture_output=True, text=True,
    )
    assert refusal.returncode != 0, "l9 must not produce a plan at this budget"
    assert "beatable in principle" in refusal.stdout, refusal.stdout
    line = next(
        line.strip() for line in refusal.stdout.splitlines() if "beatable in principle" in line
    )
    return rows, line


def first_solve(rows):
    for row in rows:
        if row["solved"] > 0:
            return row["generation"], row["solved"]
    return None, 0


def arm(level, seed, tag, outdir, gens, warm):
    tele = outdir / f"{level.stem}-{seed}-{tag}.jsonl"
    cmd = [
        str(BIN), "train", "--level", str(level),
        "--pop", str(POP), "--seed", str(seed), "--generations", str(gens),
        "--no-auto-trials", "--trials", "1", "--curriculum-generations", "0",
        "--stop-on-win", "--quiet",
        "--out", str(outdir / f"champions-{tag}"),
        "--telemetry", str(tele),
    ]
    if warm == "route":
        cmd.append("--from-route")
    elif warm == "plan":
        cmd += ["--from-plan", "--clone-epochs", str(WINDOW_EPOCHS),
                "--plan-depth", str(PLAN_DEPTH)]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if proc.returncode != 0:
        sys.exit(f"{level.name} {tag}: {proc.stdout}\n{proc.stderr}")
    rows = [json.loads(line) for line in tele.read_text().splitlines() if line.strip()]
    if not rows:
        sys.exit(f"{level}: {tag}: empty telemetry")
    gen, solved = first_solve(rows)
    clone = "—"
    for line in proc.stdout.splitlines():
        if line.startswith("clone plays"):
            clone = line.split()[2]
    return {
        "gen0": rows[0]["solved"],
        "first": gen,
        "solved_at": solved,
        "clone": clone,
        "best_leg": rows[-1]["best_end_leg"],
        "legs": rows[-1]["legs"],
    }


def part_b(outdir, gens, seeds):
    levels = []
    for name in HARD_TAIL:
        levels.append(ROOT / "levels" / "practice" / f"{name}.json")
    for name in PACK:
        levels.append(ROOT / "levels" / f"{name}.json")
    table = []
    for path in levels:
        row = {"level": path.stem, "plan": None}
        for tag, warm in (("random", None), ("route", "route"), ("plan", "plan")):
            if warm == "plan" and not (outdir / "plans" / f"{path.stem}.routeplan.json").exists():
                # The oracle-certified level the planner cannot route: no warm start is possible,
                # and the refusal is the result. Running the arm would only re-report it.
                row["plan"] = {"unroutable": True}
                continue
            runs = [arm(path, seed, tag, outdir, gens, warm) for seed in seeds]
            row[tag] = {
                "gen0": max(r["gen0"] for r in runs),
                "first": min((r["first"] for r in runs if r["first"] is not None), default=None),
                "clone": runs[0]["clone"],
                "best_leg": max(r["best_leg"] for r in runs),
                "legs": runs[0]["legs"],
            }
        table.append(row)
    return table


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", default=str(ROOT / "out" / "phase1"))
    ap.add_argument("--gens", type=int, default=60)
    ap.add_argument("--seeds", default="1")
    args = ap.parse_args()
    outdir = pathlib.Path(args.out)
    outdir.mkdir(parents=True, exist_ok=True)
    seeds = [int(s) for s in args.seeds.split(",")]
    if not BIN.exists():
        sys.exit(f"missing {BIN}; run `cargo build --release -p train-cli`")

    rows, refusal = part_a(outdir)
    print("## A. The route plan, all 24 certified practice levels\n")
    print("| level | legs | rungs closed | blocks | frames | slack at leg 1 | win margin |")
    print("|---|---|---|---|---|---|---|")
    for row in rows:
        print(
            f"| {row['stem']} | {row['legs']} | {row['rungs']} | {row['blocks']} | "
            f"{row['frames']} | {row['first_margin']} | {row['margin']} |"
        )
    print(f"\n{len(rows)}/{len(rows)} plans compiled, legs contiguous and margins accounting exact.")
    print(f"Unroutable-but-beatable control (l9): {refusal}")

    table = part_b(outdir, args.gens, seeds)
    print("\n## B. The warm start, generation 0 and first solve\n")
    print("| level | legs | plan clone | random g0 / first | route g0 / first | plan g0 / first |")
    print("|---|---|---|---|---|---|")

    def cell(arm_row):
        if "unroutable" in arm_row:
            return "no plan (reported)"
        first = "—" if arm_row["first"] is None else f"g{arm_row['first']}"
        return f"{arm_row['gen0']}/{POP} / {first}"

    for row in table:
        legs = "—" if row["plan"].get("unroutable") else row["plan"]["legs"]
        clone = "—" if row["plan"].get("unroutable") else row["plan"]["clone"]
        print(
            f"| {row['level']} | {legs} | {clone} | "
            f"{cell(row['random'])} | {cell(row['route'])} | {cell(row['plan'])} |"
        )
    planned = [r for r in table if not r["plan"].get("unroutable")]
    solved_plan = sum(1 for r in planned if r["plan"]["gen0"] > 0)
    solved_route = sum(1 for r in planned if r["route"]["gen0"] > 0)
    solved_random = sum(1 for r in planned if r["random"]["gen0"] > 0)
    print(
        f"\nlevels solved at generation 0, over the {len(planned)} levels with a plan: "
        f"random {solved_random}, route {solved_route}, plan {solved_plan}"
    )
    print(f"levels with no plan (reported, population starts random): "
          f"{len(table) - len(planned)}")
    print(
        f"protocol: pop {POP}, one episode per genome, <= {args.gens} generations, stop on first "
        f"solve, curriculum off, seeds {seeds}; plan arm at depth {PLAN_DEPTH} and {WINDOW_EPOCHS} "
        "clone epochs (the window's fit budget)."
    )
    print(f"\nplans and runs: {outdir}")


if __name__ == "__main__":
    main()
