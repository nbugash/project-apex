#!/usr/bin/env python3
"""Tests for the pipeline driver.

Run: python3 scripts/pipeline_test.py

The interesting cases are calibration cases. A path-shaped token in a task
description is not necessarily a file the task edits, and getting that wrong
in either direction is what makes an automated check either noisy enough to
ignore or quiet enough to be useless.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

import pipeline
from pipeline import Phase


def write(path: Path, text: str = "x") -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


SPEC = """# Spec

## Clarifications

### Session 2026-09-24

- Q: something? -> A: an answer

## User Scenarios

## Requirements

## Success Criteria
"""

SPEC_UNCLARIFIED = SPEC.replace("### Session 2026-09-24", "")


class PhaseDerivation(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def advance_to_tasks(self) -> None:
        write(self.dir / "spec.md", SPEC)
        for name in pipeline.PLAN_ARTIFACTS:
            write(self.dir / name)
        (self.dir / "contracts").mkdir()
        write(self.dir / "tasks.md", "- [ ] T001 Do a thing in `src/a.rs`\n")

    def test_no_spec_means_specify(self) -> None:
        self.assertIs(pipeline.phase_of(self.dir), Phase.SPECIFY)

    def test_unclarified_spec_means_clarify(self) -> None:
        write(self.dir / "spec.md", SPEC_UNCLARIFIED)
        self.assertIs(pipeline.phase_of(self.dir), Phase.CLARIFY)

    def test_clarified_spec_means_plan(self) -> None:
        write(self.dir / "spec.md", SPEC)
        self.assertIs(pipeline.phase_of(self.dir), Phase.PLAN)

    def test_contracts_directory_is_part_of_plan(self) -> None:
        write(self.dir / "spec.md", SPEC)
        for name in pipeline.PLAN_ARTIFACTS:
            write(self.dir / name)
        self.assertIs(pipeline.phase_of(self.dir), Phase.PLAN)
        (self.dir / "contracts").mkdir()
        self.assertIs(pipeline.phase_of(self.dir), Phase.TASKS)

    def test_tasks_without_analysis_means_analyze(self) -> None:
        self.advance_to_tasks()
        self.assertIs(pipeline.phase_of(self.dir), Phase.ANALYZE)

    def test_clean_analysis_opens_implement(self) -> None:
        self.advance_to_tasks()
        write(self.dir / "analysis.md", "## Pass 1\n\nVERDICT: CLEAN\n")
        self.assertIs(pipeline.phase_of(self.dir), Phase.IMPLEMENT)

    def test_findings_hold_it_at_analyze(self) -> None:
        self.advance_to_tasks()
        write(self.dir / "analysis.md", "## Pass 1\n\nVERDICT: FINDINGS\n")
        self.assertIs(pipeline.phase_of(self.dir), Phase.ANALYZE)

    def age(self, **stamps: int) -> None:
        """Set mtimes explicitly. Every file the derivation reads needs one,
        or the assertion ends up resting on whichever file setUp touched last."""
        for name, when in stamps.items():
            os.utime(self.dir / name.replace("_", "."), (when, when))

    def test_analysis_predating_a_spec_change_is_stale(self) -> None:
        self.advance_to_tasks()
        write(self.dir / "analysis.md", "## Pass 1\n\nVERDICT: CLEAN\n")
        self.age(plan_md=500, analysis_md=1_000, tasks_md=1_000, spec_md=2_000)
        self.assertIs(pipeline.phase_of(self.dir), Phase.ANALYZE)

    def test_analysis_predating_a_plan_change_is_stale(self) -> None:
        self.advance_to_tasks()
        write(self.dir / "analysis.md", "## Pass 1\n\nVERDICT: CLEAN\n")
        self.age(spec_md=500, analysis_md=1_000, tasks_md=1_000, plan_md=2_000)
        self.assertIs(pipeline.phase_of(self.dir), Phase.ANALYZE)

    def test_ticking_a_task_box_does_not_restale_the_analysis(self) -> None:
        # Implementation mutates tasks.md by design. Treating that as a design
        # change would bounce the loop back to analysis forever.
        self.advance_to_tasks()
        write(self.dir / "analysis.md", "## Pass 1\n\nVERDICT: CLEAN\n")
        self.age(spec_md=500, plan_md=500, analysis_md=1_000, tasks_md=2_000)
        self.assertIs(pipeline.phase_of(self.dir), Phase.IMPLEMENT)

    def test_all_tasks_checked_means_complete(self) -> None:
        self.advance_to_tasks()
        write(self.dir / "tasks.md", "- [X] T001 Do a thing in `src/a.rs`\n")
        write(self.dir / "analysis.md", "## Pass 1\n\nVERDICT: CLEAN\n")
        self.assertIs(pipeline.phase_of(self.dir), Phase.COMPLETE)


class ParallelCollisions(unittest.TestCase):
    def test_two_parallel_tasks_on_one_file_collide(self) -> None:
        text = (
            "- [ ] T001 [P] Write `tests/unit/status-bar.test.ts` for state\n"
            "- [ ] T002 [P] Write `tests/unit/status-bar.test.ts` for truncation\n"
        )
        self.assertEqual(
            pipeline.parallel_collisions(text),
            {"tests/unit/status-bar.test.ts": ["T001", "T002"]},
        )

    def test_sequential_tasks_on_one_file_do_not_collide(self) -> None:
        text = (
            "- [ ] T001 Write `src/a.rs`\n"
            "- [ ] T002 Extend `src/a.rs`\n"
        )
        self.assertEqual(pipeline.parallel_collisions(text), {})

    def test_a_cited_document_is_not_an_edit(self) -> None:
        # F003 T011/T014: both cite [data-model.md](./data-model.md) and edit
        # different source files. Reporting this would be a false positive.
        text = (
            "- [ ] T011 [P] Implement types in `client/core/src/domain/workspace.rs`"
            " per [data-model.md](./data-model.md)\n"
            "- [ ] T014 [P] Implement types in `client/core/src/domain/cache.rs`"
            " per [data-model.md](./data-model.md)\n"
        )
        self.assertEqual(pipeline.parallel_collisions(text), {})

    def test_a_shared_output_directory_is_not_a_collision(self) -> None:
        # F003 T008/T039: both drop distinct captures into one directory.
        text = (
            "- [ ] T008 [P] Confirm captures land in `reports/screenshots/${OS}/F003/`\n"
            "- [ ] T039 [P] Write `tests/e2e/workspace-tree.spec.ts`, captures in"
            " `reports/screenshots/${OS}/F003/`\n"
        )
        self.assertEqual(pipeline.parallel_collisions(text), {})

    def test_trailing_punctuation_does_not_split_a_path(self) -> None:
        text = (
            "- [ ] T001 [P] Edit engine/src/a.rs.\n"
            "- [ ] T002 [P] Edit engine/src/a.rs, carefully\n"
        )
        self.assertEqual(
            pipeline.parallel_collisions(text), {"engine/src/a.rs": ["T001", "T002"]}
        )


class TaskChecks(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def test_duplicate_ids_are_reported(self) -> None:
        write(self.dir / "tasks.md", "- [ ] T001 a in `x/a.rs`\n- [ ] T001 b in `x/b.rs`\n")
        self.assertIn("duplicate task ids: T001", " ".join(pipeline.check_tasks(self.dir)))

    def test_a_checkbox_without_a_task_id_is_malformed(self) -> None:
        write(self.dir / "tasks.md", "- [ ] T001 a in `x/a.rs`\n- [ ] no identifier here\n")
        problems = " ".join(pipeline.check_tasks(self.dir))
        self.assertIn("does not match the required format", problems)

    def test_a_well_formed_list_passes(self) -> None:
        write(self.dir / "tasks.md", "- [ ] T001 [P] a in `x/a.rs`\n- [ ] T002 [P] b in `x/b.rs`\n")
        self.assertEqual(pipeline.check_tasks(self.dir), [])

    def test_unchecked_tasks_are_counted(self) -> None:
        text = "- [X] T001 done in `x/a.rs`\n- [ ] T002 not in `x/b.rs`\n"
        self.assertEqual(pipeline.unchecked_tasks(text), ["T002"])


class SpecChecks(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def test_a_leftover_marker_fails_the_specify_check(self) -> None:
        write(self.dir / "spec.md", SPEC + "\n[NEEDS CLARIFICATION: which one?]\n")
        write(self.dir / "checklists/requirements.md", "- [x] ok\n")
        self.assertIn("1 unresolved", " ".join(pipeline.check_specify(self.dir)))

    def test_an_open_checklist_item_fails_the_clarify_check(self) -> None:
        write(self.dir / "spec.md", SPEC)
        write(self.dir / "checklists/requirements.md", "- [x] ok\n- [ ] not yet\n")
        self.assertIn("1 unchecked item", " ".join(pipeline.check_clarify(self.dir)))

    def test_a_clarified_spec_passes(self) -> None:
        write(self.dir / "spec.md", SPEC)
        write(self.dir / "checklists/requirements.md", "- [x] ok\n")
        self.assertEqual(pipeline.check_clarify(self.dir), [])

    def test_sessions_outside_the_clarifications_section_do_not_count(self) -> None:
        self.assertEqual(pipeline.clarification_sessions(SPEC), 1)
        self.assertEqual(pipeline.clarification_sessions(SPEC_UNCLARIFIED), 0)


class Propagation(unittest.TestCase):
    """Amend a requirement, and every artifact citing it must be revisited.

    Built on a real temporary git repository rather than mocks, because the
    check is entirely about what git history says and a mocked history would
    only prove the mock agrees with itself.
    """

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)
        self.dir = self.root / "specs" / "001-thing"
        (self.dir / "contracts").mkdir(parents=True)
        self.previous_root, pipeline.ROOT = pipeline.ROOT, self.root
        self.addCleanup(lambda: setattr(pipeline, "ROOT", self.previous_root))
        self.git("init", "-q")
        self.git("config", "user.email", "t@example.com")
        self.git("config", "user.name", "T")

    def git(self, *args: str) -> None:
        subprocess.run(["git", *args], cwd=self.root, check=True, capture_output=True)

    # Commit timestamps have one-second granularity, and a test writes its
    # whole history in well under a second. Each commit therefore gets an
    # explicit, increasing date -- otherwise every commit is simultaneous and
    # "written before" is unanswerable.
    def commit(self, message: str) -> None:
        self.clock = getattr(self, "clock", 1_700_000_000) + 60
        stamp = f"{self.clock} +0000"
        env = dict(os.environ, GIT_AUTHOR_DATE=stamp, GIT_COMMITTER_DATE=stamp)
        self.git("add", "-A")
        subprocess.run(
            ["git", "commit", "-q", "-m", message],
            cwd=self.root, check=True, capture_output=True, env=env,
        )

    SPEC = (
        "# Spec\n\n## Requirements\n\n"
        "- **FR-001**: The thing must happen.\n"
        "- **FR-002**: The other thing must happen.\n"
    )

    def test_an_artifact_citing_an_amended_requirement_is_flagged(self) -> None:
        write(self.dir / "spec.md", self.SPEC)
        write(self.dir / "design.md", "Implements FR-001 as described.\n")
        self.commit("initial")

        write(self.dir / "spec.md", self.SPEC.replace(
            "The thing must happen.", "The thing must happen, carrying metadata."))
        self.commit("amend FR-001")

        problems = pipeline.check_propagation(self.dir)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("design.md", problems[0])
        self.assertIn("FR-001", problems[0])

    def test_an_artifact_citing_an_untouched_requirement_is_not_flagged(self) -> None:
        write(self.dir / "spec.md", self.SPEC)
        write(self.dir / "design.md", "Implements FR-002 as described.\n")
        self.commit("initial")

        write(self.dir / "spec.md", self.SPEC.replace(
            "The thing must happen.", "The thing must happen, carrying metadata."))
        self.commit("amend FR-001")

        self.assertEqual(pipeline.check_propagation(self.dir), [])

    def test_an_artifact_updated_after_the_amendment_is_not_flagged(self) -> None:
        write(self.dir / "spec.md", self.SPEC)
        write(self.dir / "design.md", "Implements FR-001 as described.\n")
        self.commit("initial")

        write(self.dir / "spec.md", self.SPEC.replace(
            "The thing must happen.", "The thing must happen, carrying metadata."))
        self.commit("amend FR-001")

        write(self.dir / "design.md", "Implements FR-001, metadata included.\n")
        self.commit("propagate to design")

        self.assertEqual(pipeline.check_propagation(self.dir), [])

    def test_citation_guards(self) -> None:
        # A single guard for both id shapes is a bug this check already made:
        # excluding a trailing dot to separate §4.8 from §4.8.1 also excluded
        # "FR-001." at the end of an ordinary sentence.
        self.assertTrue(pipeline.mentions("Shape required by FR-001.", "FR-001"))
        self.assertFalse(pipeline.mentions("See FR-001a only.", "FR-001"))
        self.assertTrue(pipeline.mentions("Framing is defined in §4.1.", "§4.1"))
        self.assertFalse(pipeline.mentions("See §4.8.1 for detail.", "§4.8"))
        self.assertTrue(pipeline.mentions("Per §4.8 and A-COALESCE.", "A-COALESCE"))

    def test_a_prefix_is_not_a_citation(self) -> None:
        # FR-001 must not match inside FR-001a. This is the bug that would make
        # the check fire constantly and therefore be ignored.
        spec = self.SPEC + "- **FR-001a**: A refinement.\n"
        write(self.dir / "spec.md", spec)
        write(self.dir / "design.md", "Implements FR-001a only.\n")
        self.commit("initial")

        write(self.dir / "spec.md", spec.replace(
            "The thing must happen.", "Reworded entirely."))
        self.commit("amend FR-001 alone")

        self.assertEqual(pipeline.check_propagation(self.dir), [])

    def test_contracts_are_dependent_artifacts_too(self) -> None:
        write(self.dir / "spec.md", self.SPEC)
        write(self.dir / "contracts" / "wire.md", "Shape required by FR-001.\n")
        self.commit("initial")

        write(self.dir / "spec.md", self.SPEC.replace(
            "The thing must happen.", "The thing must happen differently."))
        self.commit("amend FR-001")

        problems = pipeline.check_propagation(self.dir)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("wire.md", problems[0])

    def test_spec_is_not_its_own_dependent(self) -> None:
        # Asserted on the artifact list directly. Asserting "no problems" would
        # have been vacuous: spec.md's write time always equals its own last
        # requirement change, so the arithmetic could never flag it and the
        # test would pass whether or not the exclusion existed. A surviving
        # mutant found that.
        write(self.dir / "spec.md", self.SPEC)
        write(self.dir / "design.md", "Implements FR-001.\n")
        self.commit("initial")
        names = [p.name for p in pipeline.dependent_artifacts(self.dir)]
        self.assertNotIn("spec.md", names)
        self.assertIn("design.md", names)

    def test_a_deleted_requirement_leaves_its_citations_stale(self) -> None:
        # A requirement can be removed as well as reworded, and a citation of
        # something that no longer exists is worse than a stale one. This needs
        # the diff's REMOVED lines, which a check reading only additions misses.
        write(self.dir / "spec.md", self.SPEC)
        write(self.dir / "design.md", "Implements FR-002 as described.\n")
        self.commit("initial")

        write(self.dir / "spec.md", "# Spec\n\n## Requirements\n\n"
              "- **FR-001**: The thing must happen.\n")
        self.commit("drop FR-002")

        problems = pipeline.check_propagation(self.dir)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("FR-002", problems[0])

    def test_an_uncommitted_artifact_is_being_worked_on_not_stale(self) -> None:
        write(self.dir / "spec.md", self.SPEC)
        write(self.dir / "design.md", "Implements FR-001.\n")
        self.commit("initial")
        write(self.dir / "spec.md", self.SPEC.replace("must happen.", "must happen now."))
        self.commit("amend FR-001")

        write(self.dir / "design.md", "Implements FR-001, mid-edit.\n")  # not committed
        self.assertEqual(pipeline.check_propagation(self.dir), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
