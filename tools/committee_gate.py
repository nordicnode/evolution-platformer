#!/usr/bin/env python3
"""Item 10's pack-wide gate: does the committee ever lose to its best member?

The plan's gate is "committee against its own best member on the validation phases across the pack:
completion and clock". `train --committee 5` builds a committee from the run's top genomes and
validates it on the run's own fixed validation episodes; `train-cli committee` re-runs the same gate
from the written file and exits non-zero if the vote lost. This script does that for every pack level
and prints one table.

The budget is pop 200 and 100 generations per level: the gate is a property of the *vote* against
its best member on fixed validation episodes, not of how good the run was, and the pack's hardest
levels do not solve at any budget (so completion there is a tie at zero for both arms). The budget
is not arbitrary — at pop 100 and 40 generations the top five have not converged and the vote loses
`l8-long-jump` 0/12 against 12/12, which is recorded beside the passing table in `FINDINGS.md`.
Defaults here match the recorded run.

Committees are written to `committees/` relative to the working directory, so the runs are made in a
scratch directory and the level paths are absolute: the gate leaves the repository clean.

Usage:
    cargo build --release -p train-cli
    python3 tools/committee_gate.py [--generations 40] [--pop 100]
"""

import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "train-cli"
BUILT = re.compile(
    r"committee (\d+) members, (\d+) of (\d+) validation episodes \(best member (\d+) of (\d+)\)"
)
GATE = re.compile(
    r"gate\s+committee won (\d+) of (\d+), best member won (\d+) of (\d+) \(frames (\d+) against (\d+)\)"
)


def pack_levels():
    return sorted(p for p in (ROOT / "levels").glob("l*.json") if p.suffix == ".json")


def run_one(level, out_dir, cwd, generations, pop):
    row = {"level": level.stem, "build": None, "gate": None, "exit": None, "error": None}
    built = subprocess.run(
        [
            str(BIN), "train",
            "--level", str(level),
            "--pop", str(pop),
            "--generations", str(generations),
            "--committee", "5",
            "--out", str(out_dir),
        ],
        capture_output=True, text=True, cwd=cwd,
    )
    text = built.stdout + built.stderr
    if m := BUILT.search(text):
        row["build"] = [int(m.group(i)) for i in range(1, 6)]
    else:
        row["error"] = text.strip().splitlines()[-1] if text.strip() else "no output"
        row["exit"] = built.returncode
        return row
    # The gate command reads the file the run just wrote (`committees/<stem>.committee.json`) and
    # exits non-zero when the vote lost an episode its best member won.
    committee_path = cwd / "committees" / f"{level.stem}.committee.json"
    gate = subprocess.run(
        [str(BIN), "committee", "--file", str(committee_path), "--level", str(level)],
        capture_output=True, text=True, cwd=cwd,
    )
    gtext = gate.stdout + gate.stderr
    if m := GATE.search(gtext):
        row["gate"] = [int(m.group(i)) for i in range(1, 7)]
    row["exit"] = gate.returncode
    if gate.returncode != 0:
        row["error"] = gtext.strip().splitlines()[-1] if gtext.strip() else "no output"
    return row


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--generations", type=int, default=100)
    ap.add_argument("--pop", type=int, default=200)
    ap.add_argument("--out", default=str(ROOT / "out" / "committee"))
    args = ap.parse_args()

    out = pathlib.Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    cwd = out / "work"
    cwd.mkdir(parents=True, exist_ok=True)

    rows = []
    for level in pack_levels():
        print(f"{level.stem:24s}", flush=True)
        rows.append(run_one(level, out, cwd, args.generations, args.pop))

    lines = [
        "## Item 10's pack-wide gate: the committee against its best member",
        "",
        f"`train --committee 5`, pop {args.pop}, {args.generations} generations per level, then",
        "`train-cli committee` re-validating from the written file. Completion is the plan's rule:",
        "the vote may tie its best member and may never lose.",
        "",
        "| level | members | committee won | best member won | frames (vote / member) | gate |",
        "|---|---|---|---|---|---|",
    ]
    for r in rows:
        if r["build"] is None:
            lines.append(f"| {r['level']} | — | — | — | — | build failed: {r['error']} |")
            continue
        members, won, episodes, best, best_episodes = r["build"]
        if r["gate"]:
            gw, ge, bw, be, gf, bf = r["gate"]
            frames = f"{gf} / {bf}"
        else:
            gw = bw = ge = be = None
            frames = "—"
        verdict = "pass" if r["exit"] == 0 else f"**LOST**: {r['error']}"
        cells = (
            f"{members} | {won}/{episodes} | {best}/{best_episodes} | {frames} | {verdict}"
        )
        lines.append(f"| {r['level']} | {cells} |")
    failed = [r for r in rows if r["exit"] != 0]
    lines += [
        "",
        f"Levels where the vote lost to its best member: {len(failed)} of {len(rows)}"
        + ("" if not failed else f" ({', '.join(r['level'] for r in failed)})"),
        "",
    ]
    report = "\n".join(lines)
    (out / "committee-gate.md").write_text(report + "\n")
    print()
    print(report)
    if failed:
        sys.exit(1)


if __name__ == "__main__":
    main()
