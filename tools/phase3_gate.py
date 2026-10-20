#!/usr/bin/env python3
"""Phase 3's acceptance gate: do jump-margin labels move the seeds that die at the timed jump?

Three arms, same level (`l11-the-elements`), same planner budget (depth 64), same fit budget (the
window's 300 epochs), same population size and the same seed, differing only in the teacher:

* `baseline`  — `--no-meta --no-jump-labels`: the Phase 2 teacher. Corrections are planned only
                when the student is off the demonstration's corridor.
* `labels`    — `--no-meta`: the same, plus a correction planned from an on-corridor state whose
                chosen action disagrees with the plan's nearest action about the jump button.
* `phase3`    — the default: jump-margin labels plus the meta-controller's sequencing.

The reading is the per-seed leg distribution and the death cause, plus the count of jump-margin
corrections the run reports (`; jump corrections N`) and the leg each was planned from (`leg L xN`)
so the widening can be checked against the pinch rather than assumed to reach it.

Usage:
    cargo build --release -p train-cli
    python3 tools/phase3_gate.py [--seeds 8] [--out DIR]

The runs' artifacts land under `out/phase3` (gitignored); the record is FINDINGS.md.
"""

import argparse
import json
import re
import statistics
import subprocess
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "train-cli"
LEVEL = ROOT / "levels" / "l11-the-elements.json"
PLAN_DEPTH = 64
CLONE_EPOCHS = 300
ARMS = {
    "baseline": ["--no-meta", "--no-jump-labels"],
    "labels": ["--no-meta"],
    "phase3": [],
}
MASTERY = re.compile(
    r"mastery (\d+)/(\d+) legs cleared, ended in leg (\d+) \((.*?) after (\d+) frames, best_x ([\d.]+)\)"
)
PLAYS = re.compile(r"clone plays (\w+) after (\d+) frames .*best_x ([\d.]+)(?:, killed by ([\w ]+))?")
SKILLS = re.compile(r"demo skills route (\d+), commit (\d+), wait (\d+), ride (\d+)[^\n]*")
FIRST = re.compile(r"first (\w+) leg (\d+) block (\d+) frame (\d+)")
JUMP = re.compile(r"jump corrections (\d+)(?: \(([^)]*)\))?")
JUMP_LEG = re.compile(r"leg (\d+) x(\d+)")
PLAN_TIME = re.compile(r"\(seed \d+, ([\d.]+)s\)")


def run_arm(seed, arm, out):
    cmd = [
        str(BIN), "train",
        "--level", str(LEVEL),
        "--from-plan",
        "--clone-epochs", str(CLONE_EPOCHS),
        "--pop", "8",
        "--generations", "0",
        "--plan-depth", str(PLAN_DEPTH),
        "--route-seed", str(seed),
        "--dagger-rounds", "2",
        "--leg-rounds", "2",
        *ARMS[arm],
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
        "skills": None,
        "firsts": [],
        "jump": None,
        "jump_legs": {},
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
        row["cause"] = (m.group(4) or "").strip() or None
    if m := SKILLS.search(text):
        row["skills"] = [int(m.group(i)) for i in range(1, 5)]
        # Only the run's own skill line, not the same trace repeated on the clone line.
        for f in FIRST.finditer(m.group(0)):
            row["firsts"].append(
                {"skill": f.group(1), "leg": int(f.group(2)), "block": int(f.group(3)), "frame": int(f.group(4))}
            )
    if m := JUMP.search(text):
        row["jump"] = int(m.group(1))
        for leg, n in JUMP_LEG.findall(m.group(2) or ""):
            row["jump_legs"][int(leg)] = int(n)
    if m := PLAN_TIME.search(text):
        row["seconds"] = float(m.group(1))
    if row["legs"] is None:
        row["error"] = text.strip().splitlines()[-1] if text.strip() else "no output"
    return row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seeds", type=int, default=8)
    ap.add_argument("--out", default=str(ROOT / "out" / "phase3"))
    args = ap.parse_args()
    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    rows = []
    for arm in ARMS:
        for seed in range(1, args.seeds + 1):
            print(f"{arm:8s} seed {seed}", flush=True)
            rows.append(run_arm(seed, arm, out))
    (out / "phase3-gate.json").write_text(json.dumps(rows, indent=2))

    lines = [
        "## Phase 3 gate: jump-margin labels and the meta-controller, per seed",
        "",
        f"`l11-the-elements`, timed plan at depth {PLAN_DEPTH}, clone at {CLONE_EPOCHS} epochs"
        f" (the window's fit budget), pop 8, seeds 1-{args.seeds}, all three arms on one tree.",
        "",
        "| arm | seeds | legs cleared (mean / median / min-max) | past leg 1 | deaths | end legs | jump corrections | jump legs | plan+fit s |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for arm in ARMS:
        arm_rows = [r for r in rows if r["arm"] == arm and r["legs"] is not None]
        if not arm_rows:
            lines.append(f"| {arm} | 0 | — | — | — | — | — | — | — |")
            continue
        legs = [r["legs"] for r in arm_rows]
        past = sum(1 for r in arm_rows if r["legs"] >= 2)
        causes = {}
        for r in arm_rows:
            causes[r["cause"] or r["outcome"] or "none"] = causes.get(r["cause"] or r["outcome"] or "none", 0) + 1
        cause_txt = ", ".join(f"{k} {v}" for k, v in sorted(causes.items()))
        ends = ",".join(str(r["end_leg"]) for r in arm_rows)
        jumps = sum(r["jump"] or 0 for r in arm_rows)
        legs_seen: dict[int, int] = {}
        for r in arm_rows:
            for leg, n in r["jump_legs"].items():
                legs_seen[leg] = legs_seen.get(leg, 0) + n
        jump_legs = ", ".join(f"{leg}:{n}" for leg, n in sorted(legs_seen.items())) or "—"
        secs = [r["seconds"] for r in arm_rows if r["seconds"] is not None]
        clock = f"{statistics.mean(secs):.2f}" if secs else "—"
        lines.append(
            f"| {arm} | {len(arm_rows)} | {statistics.mean(legs):.2f} / "
            f"{statistics.median(legs):.1f} / {min(legs)}-{max(legs)} | {past}/{len(arm_rows)} | "
            f"{cause_txt} | {ends} | {jumps} | {jump_legs} | {clock} |"
        )
    lines += [
        "",
        "### Paired by seed",
        "",
        "| seed | baseline legs | labels legs | phase3 legs | baseline ended / cause | phase3 ended / cause | jump corrections | phase3 skills (route/commit/wait/ride) | firsts |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    by = {(r["arm"], r["seed"]): r for r in rows}
    improved = []
    for seed in range(1, args.seeds + 1):
        a, b, c = by.get(("baseline", seed)), by.get(("labels", seed)), by.get(("phase3", seed))
        if not a or not b or not c or a["legs"] is None or b["legs"] is None or c["legs"] is None:
            lines.append(f"| {seed} | — | — | — | — | — | — | — | — |")
            continue
        if c["legs"] > a["legs"]:
            improved.append(seed)
        firsts = "; ".join(
            f"{f['skill']} leg {f['leg']} block {f['block']} frame {f['frame']}" for f in c["firsts"]
        ) or "—"
        lines.append(
            f"| {seed} | {a['legs']} | {b['legs']} | {c['legs']} | {a['end_leg']} / {a['cause'] or a['outcome']} | "
            f"{c['end_leg']} / {c['cause'] or c['outcome']} | {c['jump'] or 0}"
            f"{' (' + ', '.join(f'leg {k} x{v}' for k, v in sorted(c['jump_legs'].items())) + ')' if c['jump_legs'] else ''} | "
            f"{'/'.join(map(str, c['skills'])) if c['skills'] else '—'} | {firsts} |"
        )
    lines += [
        "",
        f"Seeds where the meta-controller arm cleared more legs than the Phase 2 teacher: {improved or 'none'}.",
        "",
    ]
    report = "\n".join(lines)
    (out / "phase3-gate.md").write_text(report + "\n")
    print()
    print(report)


if __name__ == "__main__":
    main()
