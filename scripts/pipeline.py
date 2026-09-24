#!/usr/bin/env python3
"""Drive the Spec Kit pipeline across the feature map.

    pipeline.py next [FEATURE]      where the feature stands, and the next command
    pipeline.py verify [FEATURE]    run the deterministic checks for its current phase
    pipeline.py run [FEATURE]       execute phases headlessly until a human gate

Phase is derived from the artifacts on disk rather than stored in a state file.
The artifacts are the source of truth (Principle II), and derived state cannot
drift from what actually exists.

The one thing that is written down is `analysis.md`, because whether an analysis
pass ran and what it concluded is not recoverable from any other artifact -- and
without it a loop has no way to know that analysis has converged.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FEATURE_MAP = ROOT / ".specify/extensions/featuremap/scripts/python/feature_map.py"

PLAN_ARTIFACTS = (
    "plan.md",
    "research.md",
    "data-model.md",
    "quickstart.md",
    "architecture.md",
    "design.md",
)

# An analysis is stale when the design it examined has changed underneath it.
# tasks.md is deliberately excluded: implementation mutates it by ticking boxes,
# and treating that as a design change would bounce the loop back to analysis
# forever. Remediation that adds tasks nearly always edits spec.md or plan.md too.
ANALYSIS_INPUTS = ("spec.md", "plan.md")

MAX_ANALYSIS_PASSES = 6

TASK_LINE = re.compile(r"^- \[([ Xx])\] (T\d+)\b(.*)$", re.M)
CHECKBOX_LINE = re.compile(r"^\s*- \[[ Xx]\]", re.M)
PATH_TOKEN = re.compile(r"[A-Za-z0-9_.@${}-]+(?:/[A-Za-z0-9_.@${}-]+)+")
MD_LINK_TARGET = re.compile(r"\]\([^)]*\)")
PASS_HEADING = re.compile(r"^## Pass (\d+)", re.M)
VERDICT = re.compile(r"^VERDICT:\s*(\S+)", re.M)


class Phase(str, Enum):
    SPECIFY = "specify"
    CLARIFY = "clarify"
    PLAN = "plan"
    TASKS = "tasks"
    ANALYZE = "analyze"
    IMPLEMENT = "implement"
    COMPLETE = "complete"
    DONE = "done"


# Phases a human owns. The runner stops before them rather than guessing.
#   clarify  -- the answers are product decisions; a self-answered clarification
#               is internally consistent, so analysis will never catch it.
#   complete -- ticking a map box is an evidence claim, and it lands with the
#               reviewed merge rather than with the machine that did the work.
HUMAN_PHASES = frozenset({Phase.CLARIFY, Phase.COMPLETE})

COMMANDS = {
    Phase.SPECIFY: "/speckit-specify {feature}",
    Phase.CLARIFY: "/speckit-clarify {feature}",
    Phase.PLAN: "/speckit-plan {feature}",
    Phase.TASKS: "/speckit-tasks {feature}",
    Phase.ANALYZE: "/speckit-analyze {feature}",
    Phase.IMPLEMENT: "/speckit-implement {feature}",
}


@dataclass(frozen=True)
class Feature:
    identity: str
    slug: str
    done: bool
    spec_path: str | None

    @property
    def directory(self) -> Path | None:
        if not self.spec_path or not self.spec_path.startswith("specs/"):
            return None
        return ROOT / self.spec_path


MAP_ENTRY = re.compile(
    r"^- \[([ Xx])\] \*\*(F\d+) ([a-z0-9-]+)\*\*.*?\n\s+- Spec: (\S+)", re.M
)


def locate(identity: str) -> Feature | None:
    """Find a feature in the map by reading it.

    Sequencing stays with feature_map.py, which refuses a feature that is
    already complete -- correct when choosing what to work on next, and
    unhelpful when checking the artifacts of something already shipped.
    """
    for done, found, slug, spec in MAP_ENTRY.findall(read(ROOT / "specs/features-map.md")):
        if found == identity:
            path = spec if spec.startswith("specs/") else None
            return Feature(found, slug, done.strip().upper() == "X", path)
    return None


def resolve(identity: str | None, *, for_sequencing: bool = True) -> Feature:
    """Ask the feature map which feature is next, or about a named one."""
    argv = [sys.executable, str(FEATURE_MAP), "resolve"]
    if identity:
        argv.append(identity)
    argv.append("--json")
    done = subprocess.run(argv, capture_output=True, text=True, cwd=ROOT)
    payload = json.loads(done.stdout or "{}")
    if payload.get("STATUS") == "ready":
        f = payload["FEATURE"]
        return Feature(f["FEATURE_ID"], f["SLUG"], f["DONE"], f.get("SPEC_PATH"))
    if identity and not for_sequencing:
        located = locate(identity)
        if located is not None:
            return located
    raise SystemExit(
        f"feature map says {payload.get('STATUS')}: {payload.get('REASON', 'no reason given')}"
    )


def read(path: Path) -> str:
    return path.read_text(encoding="utf-8") if path.exists() else ""


def clarification_sessions(spec: str) -> int:
    section = re.search(r"^## Clarifications\s*$(.*?)(?=^## )", spec, re.M | re.S)
    return len(re.findall(r"^### Session", section.group(1), re.M)) if section else 0


def analysis_state(directory: Path) -> tuple[int, str | None, bool]:
    """Passes recorded, last verdict, and whether it still reflects the design."""
    report = directory / "analysis.md"
    if not report.exists():
        return 0, None, False
    text = read(report)
    passes = [int(n) for n in PASS_HEADING.findall(text)]
    verdicts = VERDICT.findall(text)
    stamp = report.stat().st_mtime
    current = all(
        (directory / name).stat().st_mtime <= stamp
        for name in ANALYSIS_INPUTS
        if (directory / name).exists()
    )
    return (max(passes) if passes else 0, verdicts[-1].upper() if verdicts else None, current)


def unchecked_tasks(text: str) -> list[str]:
    return [m.group(2) for m in TASK_LINE.finditer(text) if m.group(1) == " "]


def derive_phase(feature: Feature) -> Phase:
    return Phase.DONE if feature.done else phase_of(feature.directory)


def phase_of(directory: Path | None) -> Phase:
    """Read the phase off the artifacts. Kept separate from Feature so the
    derivation can be tested against a directory without a feature map."""
    if directory is None or not (directory / "spec.md").exists():
        return Phase.SPECIFY
    if clarification_sessions(read(directory / "spec.md")) == 0:
        return Phase.CLARIFY
    if any(not (directory / name).exists() for name in PLAN_ARTIFACTS):
        return Phase.PLAN
    if not (directory / "contracts").is_dir():
        return Phase.PLAN
    if not (directory / "tasks.md").exists():
        return Phase.TASKS
    _, verdict, current = analysis_state(directory)
    if not (current and verdict == "CLEAN"):
        return Phase.ANALYZE
    if unchecked_tasks(read(directory / "tasks.md")):
        return Phase.IMPLEMENT
    return Phase.COMPLETE


# --- deterministic checks -------------------------------------------------
# Each returns the problems it found. These are the claims a model should not
# be trusted to self-report: this project has three tasks marked complete that
# were not, and a gate that reported success because `tail` exited zero.


def check_specify(directory: Path) -> list[str]:
    spec = directory / "spec.md"
    if not spec.exists():
        return [f"{spec.relative_to(ROOT)} is missing"]
    text = read(spec)
    problems = []
    markers = text.count("[NEEDS CLARIFICATION")
    if markers:
        problems.append(f"{markers} unresolved [NEEDS CLARIFICATION] marker(s)")
    for heading in ("## Requirements", "## Success Criteria", "## User Scenarios"):
        if heading not in text:
            problems.append(f"spec.md has no '{heading}' section")
    if not (directory / "checklists/requirements.md").exists():
        problems.append("checklists/requirements.md is missing")
    return problems


def check_clarify(directory: Path) -> list[str]:
    problems = check_specify(directory)
    if clarification_sessions(read(directory / "spec.md")) == 0:
        problems.append("spec.md has no '### Session' entry under '## Clarifications'")
    checklist = directory / "checklists/requirements.md"
    if checklist.exists():
        open_items = read(checklist).count("- [ ]")
        if open_items:
            problems.append(f"{open_items} unchecked item(s) in checklists/requirements.md")
    return problems


def check_plan(directory: Path) -> list[str]:
    problems = [
        f"missing plan artifact: {name}"
        for name in PLAN_ARTIFACTS
        if not (directory / name).exists()
    ]
    if not (directory / "contracts").is_dir():
        problems.append("missing plan artifact: contracts/")
    if "NEEDS CLARIFICATION" in read(directory / "plan.md"):
        problems.append("plan.md still carries NEEDS CLARIFICATION")
    return problems


def edited_paths(description: str) -> set[str]:
    """The files a task writes, as distinct from the paths it merely cites.

    Calibrated against F003's shipped task list, where a naive reading of every
    path-shaped token reported two collisions that were not collisions: a
    markdown link to `data-model.md` that two tasks cite for their field
    definitions, and a screenshot output directory two tasks drop distinct
    captures into. Both are references. Neither is a file two tasks edit.
    """
    text = MD_LINK_TARGET.sub("", description)
    paths = set()
    for token in PATH_TOKEN.findall(text):
        path = token.rstrip(".,;:")
        if path.startswith(("./", "../")) or path.startswith("specs/"):
            continue  # a sibling planning document, cited rather than written
        if "${" in path:
            continue  # a templated output directory, not a source file
        if "." not in path.rsplit("/", 1)[-1]:
            continue  # a directory; two tasks may both write into one safely
        paths.add(path)
    return paths


def parallel_collisions(text: str) -> dict[str, list[str]]:
    """Tasks marked [P] that edit the same file.

    Two of these shipped in F003's first task list and survived an analysis pass
    that never cross-referenced paths against the marker. A regex does not get
    bored, which is the whole argument for checking it here.
    """
    owners: dict[str, list[str]] = {}
    for match in TASK_LINE.finditer(text):
        task_id, rest = match.group(2), match.group(3)
        if "[P]" not in rest:
            continue
        for path in sorted(edited_paths(rest)):
            owners.setdefault(path, []).append(task_id)
    return {path: ids for path, ids in owners.items() if len(ids) > 1}


def check_tasks(directory: Path) -> list[str]:
    tasks = directory / "tasks.md"
    if not tasks.exists():
        return ["tasks.md is missing"]
    text = read(tasks)
    problems = []

    malformed = [
        line
        for line in text.splitlines()
        if CHECKBOX_LINE.match(line) and not TASK_LINE.match(line)
    ]
    for line in malformed[:5]:
        problems.append(f"task line does not match the required format: {line.strip()[:70]}")

    identifiers = [m.group(2) for m in TASK_LINE.finditer(text)]
    duplicates = sorted({i for i in identifiers if identifiers.count(i) > 1})
    if duplicates:
        problems.append(f"duplicate task ids: {', '.join(duplicates)}")

    for path, ids in sorted(parallel_collisions(text).items()):
        problems.append(f"[P] tasks share {path}: {', '.join(ids)}")

    if not identifiers:
        problems.append("tasks.md contains no tasks")
    return problems


def check_analyze(directory: Path) -> list[str]:
    passes, verdict, current = analysis_state(directory)
    if verdict is None:
        return ["analysis.md is missing or records no VERDICT line"]
    problems = []
    if verdict != "CLEAN":
        problems.append(f"last analysis verdict is {verdict}, not CLEAN")
    if not current:
        newer = [
            name
            for name in ANALYSIS_INPUTS
            if (directory / name).exists()
            and (directory / name).stat().st_mtime > (directory / "analysis.md").stat().st_mtime
        ]
        problems.append(f"analysis predates changes to {', '.join(newer)}")
    if passes >= MAX_ANALYSIS_PASSES and problems:
        problems.append(
            f"{passes} analysis passes without convergence -- stop and read the findings "
            "by hand rather than running another"
        )
    return problems


def check_implement(directory: Path) -> list[str]:
    text = read(directory / "tasks.md")
    remaining = unchecked_tasks(text)
    if remaining:
        shown = ", ".join(remaining[:8]) + ("..." if len(remaining) > 8 else "")
        return [f"{len(remaining)} task(s) still unchecked: {shown}"]
    return []


CHECKS = {
    Phase.SPECIFY: check_specify,
    Phase.CLARIFY: check_clarify,
    Phase.PLAN: check_plan,
    Phase.TASKS: check_tasks,
    Phase.ANALYZE: check_analyze,
    Phase.IMPLEMENT: check_implement,
}


def verify_completed(feature: Feature, phase: Phase) -> list[str]:
    """Check the phase that just finished -- the one before the current one."""
    order = list(COMMANDS)
    if phase in (Phase.DONE, Phase.SPECIFY):
        return []
    previous = order[order.index(phase) - 1] if phase in order else Phase.IMPLEMENT
    directory = feature.directory
    if directory is None:
        return []
    return CHECKS[previous](directory)


def run_gate() -> int:
    print("running `make gate` -- the only evidence that counts", flush=True)
    return subprocess.run(["make", "gate"], cwd=ROOT).returncode


def cmd_next(args: argparse.Namespace) -> int:
    feature = resolve(args.feature, for_sequencing=False)
    phase = derive_phase(feature)
    print(f"{feature.identity} {feature.slug}: {phase.value}")
    if phase is Phase.DONE:
        print("nothing to do -- it is marked complete in the map")
        return 0
    if phase is Phase.COMPLETE:
        print(f"all tasks done. verify with `make gate`, then tick {feature.identity} in specs/features-map.md")
        return 0
    print(f"next: {COMMANDS[phase].format(feature=feature.identity)}")
    if phase in HUMAN_PHASES:
        print("this phase is yours -- the runner will not do it unattended")
    return 0


def cmd_verify(args: argparse.Namespace) -> int:
    feature = resolve(args.feature, for_sequencing=False)
    phase = derive_phase(feature)
    directory = feature.directory
    if directory is None:
        print(f"{feature.identity}: not specified yet, nothing to verify")
        return 0
    target = Phase(args.phase) if args.phase else phase
    checker = CHECKS.get(target)
    problems = checker(directory) if checker else []
    label = f"{feature.identity} {target.value}"
    if problems:
        print(f"{label}: {len(problems)} problem(s)")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print(f"{label}: ok")
    return 0


def cmd_run(args: argparse.Namespace) -> int:
    feature = resolve(args.feature)
    while True:
        phase = derive_phase(feature)
        if phase in HUMAN_PHASES or phase is Phase.DONE:
            print(f"\n{feature.identity} {feature.slug}: stopping at {phase.value}")
            if phase in COMMANDS:
                print(f"over to you: {COMMANDS[phase].format(feature=feature.identity)}")
            elif phase is Phase.COMPLETE:
                print(f"tick {feature.identity} in specs/features-map.md once the merge lands")
            return 0

        problems = verify_completed(feature, phase)
        if problems:
            print(f"\n{feature.identity}: the previous phase did not verify")
            for problem in problems:
                print(f"  - {problem}")
            return 1

        command = COMMANDS[phase].format(feature=feature.identity)
        print(f"\n{feature.identity} {feature.slug}: {phase.value}")
        if not args.execute:
            print(f"  would run: claude -p {command!r}")
            return 0

        argv = ["claude", "-p", command, "--permission-mode", args.permission_mode]
        code = subprocess.run(argv, cwd=ROOT).returncode
        if code != 0:
            print(f"  claude exited {code}; stopping")
            return code

        after = derive_phase(feature)
        if after is phase:
            print(f"  {phase.value} ran but the artifacts did not advance; stopping")
            return 1
        if phase is Phase.IMPLEMENT and run_gate() != 0:
            print("  `make gate` failed; stopping before the map is touched")
            return 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)

    nxt = sub.add_parser("next", help="report phase and the next command")
    nxt.add_argument("feature", nargs="?")
    nxt.set_defaults(func=cmd_next)

    ver = sub.add_parser("verify", help="run the checks for a phase")
    ver.add_argument("feature", nargs="?")
    ver.add_argument("--phase", choices=[p.value for p in CHECKS])
    ver.set_defaults(func=cmd_verify)

    run = sub.add_parser("run", help="execute phases until a human gate")
    run.add_argument("feature", nargs="?")
    run.add_argument(
        "--execute",
        action="store_true",
        help="actually invoke claude; without it the runner only says what it would do",
    )
    run.add_argument("--permission-mode", default="acceptEdits")
    run.set_defaults(func=cmd_run)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
