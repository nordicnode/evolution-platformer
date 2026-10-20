#!/usr/bin/env python3
"""Phase 2's acceptance gate: does leg-targeted teaching get the plan clone further, on how many seeds?

Three arms, same level (`l11-the-elements`), same planner budget (depth 64), same fit budget (the
window's 300 epochs), same population size, differing only in the teacher:

* `imitate`     — the Phase 1 clone: one fit, no DAgger, no curriculum (`--dagger-rounds 0
                  --leg-rounds 0`). This is the arm that died on the first spike.
* `dagger`      — replanning DAgger: corrections labelled by a fresh **leg-targeted** search from
                  the state the student reached (`--dagger-rounds 2 --leg-rounds 0`).
* `curriculum`  — the same, plus leg-curriculum rounds that roll out from the leg before the first
                  one the student failed, at the state the student actually reaches.

The reading is the **per-leg mastery** the run reports — legs cleared and the leg the episode ended
in — plus the episode's outcome, death cause, frames and best x. It is a distribution over seeds,
not one number: a clone clone's first failure is a draw, and a claim about the teacher has to hold
across draws.

Usage:
    cargo build --release -p train-cli
    python3 tools/phase2_gate.py [--seeds 8] [--out DIR]

The runs' artifacts land under `out/phase2` (gitignored); the record is FINDINGS.md.
"""

import argparse
import json
import re
import statistics
import subprocess
import sys
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "train-cli"
LEVEL = ROOT / "levels" / "l11-the-elements.json"
PLAN_DEPTH = 64
CLONE_EPOCHS = 300
ARMS = {
    "imitate": (0, 0),
    "dagger": (2, 0),
    "curriculum": (2, 2),
}
MASTERY = re.compile(
    r"mastery (\d+)/(\d+) legs cleared, ended in leg (\d+) \((.*?) after (\d+) frames, best_x ([\d.]+)\)"
)
PLAYS = re.compile(r"clone plays (\w+) after (\d+) frames .*best_x ([\d.]+)(?:, killed by (\w+))?")
PLAN_TIME = re.compile(r"\(seed \d+, ([\d.]+)s\)")


def run_arm(seed, arm, out):
    dagger, leg = ARMS[arm]
    cmd = [
        str(BIN), "train",
        "--level", str(LEVEL),
        "--from-plan",
        "--clone-epochs", str(CLONE_EPOCHS),
        "--pop", "8",
        "--generations", "0",
        "--plan-depth", str(PLAN_DEPTH),
        "--route-seed", str(seed),
        "--dagger-rounds", str(dagger),
        "--leg-rounds", str(leg),
        "--out", str(out / f"{arm}-{seed}"),
    ]
    done = subprocess.run(cmd, capture_output=True, text=True)
    text = done.stdout + done.stderr
    row = {
        "seed": seed,
        "arm": arm,
        "exit": done.returncode,
        "legs": None,
        "total_legs": None,
        "end_leg": None,
        "outcome": None,
        "frames": None,
        "best_x": None,
        "cause": None,
        "seconds": None,
    }
    if m := MASTERY.search(text):
        row["legs"] = int(m.group(1))
        row["total_legs"] = int(m.group(2))
        row["end_leg"] = int(m.group(3))
    if m := PLAYS.search(text):
        row["outcome"] = m.group(1)
        row["frames"] = int(m.group(2))
        row["best_x"] = float(m.group(3))
        row["cause"] = m.group(4)
    if m := PLAN_TIME.search(text):
        row["seconds"] = float(m.group(1))
    if row["legs"] is None:
        row["error"] = text.strip().splitlines()[-1] if text.strip() else "no output"
    return row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seeds", type=int, default=8)
    ap.add_argument("--out", default=str(ROOT / "out" / "phase2"))
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    rows = []
    for arm in ARMS:
        for seed in range(1, args.seeds + 1):
            print(f"{arm:11s} seed {seed}", flush=True)
            rows.append(run_arm(seed, arm, out))
    (out / "phase2-gate.json").write_text(json.dumps(rows, indent=2))

    lines = [
        "## Phase 2 gate: l11's plan clone, per seed",
        "",
        f"`l11-the-elements`, timed plan at depth {PLAN_DEPTH}, clone at {CLONE_EPOCHS} epochs"
        f" (the window's fit budget), pop 8, seeds 1-{args.seeds}.",
        "",
        "| arm | seeds | legs cleared (mean / median / min-max) | past the first spike | deaths by spike | end legs | plan+fit s |",
        "|---|---|---|---|---|---|---|",
    ]
    for arm in ARMS:
        arm_rows = [r for r in rows if r["arm"] == arm and r["legs"] is not None]
        if not arm_rows:
            lines.append(f"| {arm} | 0 | — | — | — | — | — |")
            continue
        legs = [r["legs"] for r in arm_rows]
        past = sum(1 for r in arm_rows if r["legs"] >= 2)
        spike = sum(1 for r in arm_rows if r["cause"] == "spike")
        ends = ",".join(str(r["end_leg"]) for r in arm_rows)
        secs = [r["seconds"] for r in arm_rows if r["seconds"] is not None]
        clock = f"{statistics.mean(secs):.2f}" if secs else "—"
        lines.append(
            f"| {arm} | {len(arm_rows)} | {statistics.mean(legs):.2f} / "
            f"{statistics.median(legs):.1f} / {min(legs)}-{max(legs)} | {past}/{len(arm_rows)} | "
            f"{spike}/{len(arm_rows)} | {ends} | {clock} |"
        )
    lines += [
        "",
        "`past the first spike` = the clone closed leg 1 (the leg the Phase 1 clone died in,",
        "102 frames, x 16.3). `end legs` lists, seed by seed, the leg the mastery reading named.",
        "",
        "| seed | " + " | ".join(ARMS) + " |",
        "|---|" + "---|" * len(ARMS),
    ]
    for seed in range(1, args.seeds + 1):
        cells = []
        for arm in ARMS:
            r = next((r for r in rows if r["arm"] == arm and r["seed"] == seed), None)
            if r is None or r["legs"] is None:
                cells.append("—")
            else:
                cells.append(
                    f"{r['legs']}/{r['total_legs']} legs, {r['outcome'].lower()}"
                    + (f" ({r['cause']})" if r["cause"] else "")
                    + f", x {r['best_x']:.1f}"
                )
        lines.append(f"| {seed} | " + " | ".join(cells) + " |")

    # The gate's own reading: the teacher has to move the distribution, and the curriculum must not
    # take legs back from the arm it refines.
    by = {arm: [r for r in rows if r["arm"] == arm and r["legs"] is not None] for arm in ARMS}
    report = []
    ok = True
    for arm in ARMS:
        missing = args.seeds - len(by[arm])
        if missing:
            ok = False
            report.append(f"{arm}: {missing} of {args.seeds} seeds produced no mastery reading")
    for arm in ("dagger", "curriculum"):
        base = [r["legs"] for r in by["imitate"]]
        got = [r["legs"] for r in by[arm]]
        if not base or not got:
            ok = False
            report.append(f"{arm}: no measurements")
            continue
        better = statistics.mean(got) > statistics.mean(base)
        ok = ok and better
        report.append(
            f"{arm}: mean legs {statistics.mean(got):.2f} vs imitation {statistics.mean(base):.2f}"
            f" ({'PASS' if better else 'FAIL'})"
        )
    if not by["dagger"] or not by["curriculum"]:
        ok = False
        report.append("curriculum vs dagger: no measurements")
    else:
        cur = [r["legs"] for r in by["curriculum"]]
        dag = [r["legs"] for r in by["dagger"]]
        no_regress = min(cur) >= min(dag) and statistics.mean(cur) >= statistics.mean(dag)
        ok = ok and no_regress
        report.append(
            f"curriculum vs dagger: mean {statistics.mean(cur):.2f} vs {statistics.mean(dag):.2f},"
            f" min {min(cur)} vs {min(dag)} ({'PASS' if no_regress else 'FAIL'})"
        )

    lines += ["", "### Gate", ""] + [f"- {r}" for r in report]
    table = "\n".join(lines) + "\n"
    (out / "phase2-table.md").write_text(table)
    print("\n" + table)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
