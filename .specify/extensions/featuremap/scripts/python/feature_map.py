#!/usr/bin/env python3
"""Feature map parsing and sequence-gate enforcement.

The feature map is an ordered backlog in Markdown. Feature identities are
immutable, file position conveys build order, checkboxes are evidence of
completed work, and a feature may only be specified once every dependency it
declares is checked.

The map is optional. When it is absent every read action reports `skipped` and
exits zero, so the mandatory before_specify hook never blocks a project that
does not keep one. Only `add` creates a map, because adding a feature is an
explicit request for a backlog; resolving one must never opt a project into
sequencing it did not ask for.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_MAP_RELATIVE = Path("specs/features-map.md")

FEATURE_RE = re.compile(r"^- \[([ xX])\] \*\*(F\d{3}) ([a-z0-9][a-z0-9-]*)\*\*(.*)$")
SUBFEATURE_RE = re.compile(r"^ {2}- \[([ xX])\] (.+)$")
SPEC_RE = re.compile(r"^ {2}- Spec: (.+)$")
DEPENDS_RE = re.compile(r"\(depends:([^)]*)\)")
FEATURE_ID_RE = re.compile(r"F\d{3}")
SLUG_RE = re.compile(r"[a-z0-9][a-z0-9-]*")

SPEC_UNSET = "not yet specified"

SKELETON = """# Feature Map

Ordered backlog. Work top to bottom: every feature's dependencies sit above it.

Identities such as `F000` are immutable and never renumbered or reused; file
position conveys build order. A checkbox is evidence that the work is done, not
an intention to do it.

## Features

"""


class FeatureMapError(Exception):
    """Raised when the map cannot be read or does not parse."""


@dataclass
class Feature:
    """One top-level feature row in the map."""

    identity: str
    slug: str
    done: bool
    line_no: int
    parallel: bool = False
    depends: list[str] = field(default_factory=list)
    spec_path: str | None = None
    spec_line_no: int | None = None
    subfeatures: list[tuple[bool, str]] = field(default_factory=list)

    @property
    def pending_subfeatures(self) -> list[str]:
        return [text for done, text in self.subfeatures if not done]

    def to_dict(self) -> dict:
        return {
            "FEATURE_ID": self.identity,
            "SLUG": self.slug,
            "DONE": self.done,
            "PARALLEL": self.parallel,
            "DEPENDS_ON": self.depends,
            "SPEC_PATH": self.spec_path,
            "SUBFEATURE_COUNT": len(self.subfeatures),
            "SUBFEATURES_PENDING": self.pending_subfeatures,
        }


def find_project_root(start_dir: Path | None = None) -> Path | None:
    """Walk upward looking for the .specify directory."""
    current = (start_dir or Path.cwd()).resolve()
    while True:
        if (current / ".specify").is_dir():
            return current
        parent = current.parent
        if parent == current:
            return None
        current = parent


def parse(text: str) -> list[Feature]:
    """Parse the map into features, preserving file order."""
    features: list[Feature] = []
    current: Feature | None = None

    for offset, line in enumerate(text.splitlines()):
        line_no = offset + 1

        match = FEATURE_RE.match(line)
        if match:
            checkbox, identity, slug, remainder = match.groups()
            depends: list[str] = []
            depends_match = DEPENDS_RE.search(remainder)
            if depends_match:
                depends = FEATURE_ID_RE.findall(depends_match.group(1))
            current = Feature(
                identity=identity,
                slug=slug,
                done=checkbox.lower() == "x",
                line_no=line_no,
                parallel="[P]" in remainder,
                depends=depends,
            )
            features.append(current)
            continue

        if current is None:
            continue

        match = SUBFEATURE_RE.match(line)
        if match:
            checkbox, description = match.groups()
            current.subfeatures.append((checkbox.lower() == "x", description.strip()))
            continue

        match = SPEC_RE.match(line)
        if match:
            value = match.group(1).strip()
            current.spec_path = None if value == SPEC_UNSET else value
            current.spec_line_no = line_no

    if not features:
        raise FeatureMapError(
            "no feature rows found; expected lines like '- [ ] **F000 slug**'"
        )
    return features


def load(path: Path) -> list[Feature]:
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as error:
        raise FeatureMapError(f"cannot read {path}: {error}") from error
    return parse(text)


def verify(features: list[Feature]) -> list[str]:
    """Return integrity problems that make the map untrustworthy."""
    problems: list[str] = []
    by_identity: dict[str, Feature] = {}
    position: dict[str, int] = {}

    for index, feature in enumerate(features):
        if feature.identity in by_identity:
            problems.append(
                f"{feature.identity} is declared twice "
                f"(lines {by_identity[feature.identity].line_no} and {feature.line_no}); "
                "identities are never reused"
            )
            continue
        by_identity[feature.identity] = feature
        position[feature.identity] = index

    for feature in features:
        if feature.done and feature.pending_subfeatures:
            problems.append(
                f"{feature.identity} is checked but has unchecked subfeatures: "
                + "; ".join(feature.pending_subfeatures)
            )
        for dependency in feature.depends:
            if dependency not in by_identity:
                problems.append(
                    f"{feature.identity} declares unknown dependency {dependency}"
                )
            elif position[dependency] > position[feature.identity]:
                problems.append(
                    f"{feature.identity} depends on {dependency}, which is positioned "
                    "below it; file position must convey build order"
                )

    return problems


def blocking_dependencies(feature: Feature, by_identity: dict[str, Feature]) -> list[str]:
    return [
        dependency
        for dependency in feature.depends
        if dependency in by_identity and not by_identity[dependency].done
    ]


def resolve(features: list[Feature], target: str | None) -> dict:
    """Resolve which feature may be specified, or explain why none may be."""
    by_identity = {feature.identity: feature for feature in features}

    if target:
        identity = target.strip().upper()
        feature = by_identity.get(identity)
        if feature is None:
            return {
                "STATUS": "error",
                "REASON": f"{identity} is not in the feature map",
                "AVAILABLE": [item.identity for item in features],
            }
        if feature.done:
            return {
                "STATUS": "blocked",
                "REASON": f"{identity} is already checked as complete",
                "FEATURE": feature.to_dict(),
            }
        blocked_by = blocking_dependencies(feature, by_identity)
        if blocked_by:
            return {
                "STATUS": "blocked",
                "REASON": (
                    f"{identity} declares dependencies that are not complete: "
                    + ", ".join(blocked_by)
                ),
                "BLOCKED_BY": [by_identity[item].to_dict() for item in blocked_by],
                "FEATURE": feature.to_dict(),
            }
        return {"STATUS": "ready", "FEATURE": feature.to_dict()}

    for feature in features:
        if feature.done:
            continue
        if blocking_dependencies(feature, by_identity):
            continue
        return {"STATUS": "ready", "FEATURE": feature.to_dict()}

    unchecked = [feature for feature in features if not feature.done]
    if not unchecked:
        return {"STATUS": "blocked", "REASON": "every feature in the map is complete"}
    return {
        "STATUS": "blocked",
        "REASON": "no feature is ready: every unchecked feature is waiting on a dependency",
        "WAITING": [
            {
                "FEATURE_ID": feature.identity,
                "BLOCKED_BY": blocking_dependencies(feature, by_identity),
            }
            for feature in unchecked
        ],
    }


def render_block(
    identity: str,
    slug: str,
    depends: list[str],
    parallel: bool,
    subfeatures: list[str],
) -> str:
    """Render a feature block in the exact shape the parser reads back."""
    remainder = ""
    if parallel:
        remainder += " [P]"
    if depends:
        remainder += f" (depends: {', '.join(depends)})"
    lines = [f"- [ ] **{identity} {slug}**{remainder}", f"  - Spec: {SPEC_UNSET}"]
    lines.extend(f"  - [ ] {item}" for item in subfeatures)
    return "\n".join(lines) + "\n"


def add_feature(
    path: Path,
    features: list[Feature],
    slug: str,
    after: str | None,
    depends: list[str],
    parallel: bool,
    subfeatures: list[str],
    dry_run: bool = False,
) -> dict:
    """Insert a new feature, assigning the next unused identity.

    Position is chosen by the caller through `after`; identities are never
    reused and never renumbered.
    """
    if not SLUG_RE.fullmatch(slug):
        raise FeatureMapError(f"slug {slug!r} must be lowercase letters, digits and hyphens")
    if not subfeatures:
        raise FeatureMapError("a new feature needs at least one subfeature")

    next_number = max((int(feature.identity[1:]) for feature in features), default=-1) + 1
    if next_number > 999:
        raise FeatureMapError("identity space F000-F999 is exhausted")
    identity = f"F{next_number:03d}"

    by_identity = {feature.identity: feature for feature in features}
    if after is None or not features:
        anchor_index = len(features) - 1
    else:
        anchor = by_identity.get(after.strip().upper())
        if anchor is None:
            raise FeatureMapError(f"{after} is not in the feature map")
        anchor_index = features.index(anchor)

    depends = [item.strip().upper() for item in depends if item.strip()]
    for dependency in depends:
        if dependency not in by_identity:
            raise FeatureMapError(f"dependency {dependency} is not in the feature map")
        if features.index(by_identity[dependency]) > anchor_index:
            raise FeatureMapError(
                f"dependency {dependency} is positioned below the insertion point; "
                "a feature cannot depend on work that comes after it"
            )

    block = render_block(identity, slug, depends, parallel, subfeatures)
    if dry_run:
        return {"STATUS": "preview", "FEATURE_ID": identity, "BLOCK": block}

    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    if not features:
        insert_at = len(lines)
    elif anchor_index + 1 < len(features):
        insert_at = features[anchor_index + 1].line_no - 1
    else:
        insert_at = len(lines)
        for index in range(features[anchor_index].line_no, len(lines)):
            if lines[index].startswith("## "):
                insert_at = index
                break
        while insert_at > 0 and not lines[insert_at - 1].strip():
            insert_at -= 1

    original = "".join(lines)
    lines.insert(insert_at, block)
    path.write_text("".join(lines), encoding="utf-8")

    problems = verify(parse(path.read_text(encoding="utf-8")))
    if problems:
        path.write_text(original, encoding="utf-8")
        raise FeatureMapError(
            "insertion would break map integrity, no change written: " + "; ".join(problems)
        )

    return {"STATUS": "added", "FEATURE_ID": identity, "BLOCK": block, "LINE": insert_at + 1}


def record_spec(path: Path, features: list[Feature], identity: str, spec_path: str) -> str:
    """Write a feature's spec directory back into the map."""
    identity = identity.strip().upper()
    feature = next((item for item in features if item.identity == identity), None)
    if feature is None:
        raise FeatureMapError(f"{identity} is not in the feature map")
    if feature.spec_line_no is None:
        raise FeatureMapError(
            f"{identity} has no 'Spec:' line to update; add one beneath the feature row"
        )

    lines = path.read_text(encoding="utf-8").splitlines(keepends=True)
    index = feature.spec_line_no - 1
    ending = "\n" if lines[index].endswith("\n") else ""
    lines[index] = f"  - Spec: {spec_path}{ending}"
    path.write_text("".join(lines), encoding="utf-8")
    return f"{identity} spec path recorded as {spec_path}"


def emit(payload: dict, as_json: bool) -> None:
    if as_json:
        print(json.dumps(payload, indent=2))
        return

    status = payload.get("STATUS")
    if status == "ready":
        feature = payload["FEATURE"]
        print(f"READY: {feature['FEATURE_ID']} {feature['SLUG']}")
        if feature["DEPENDS_ON"]:
            print(f"  dependencies satisfied: {', '.join(feature['DEPENDS_ON'])}")
        pending = len(feature["SUBFEATURES_PENDING"])
        if pending:
            print(f"  subfeatures pending: {pending} of {feature['SUBFEATURE_COUNT']}")
        return

    if status == "ok":
        complete = payload.get("COMPLETE", 0)
        total = payload.get("FEATURE_COUNT", 0)
        print(f"OK: feature map is consistent, {complete} of {total} features complete")
        return

    if status == "skipped":
        print(f"SKIPPED: {payload.get('REASON', 'no feature map')}")
        return

    print(f"{str(status).upper()}: {payload.get('REASON', 'unknown')}")
    for item in payload.get("WAITING", []):
        print(f"  {item['FEATURE_ID']} waiting on {', '.join(item['BLOCKED_BY'])}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "action",
        choices=("resolve", "verify", "record-spec", "add"),
        help="resolve the next feature, verify map integrity, record a spec path, or add a feature",
    )
    parser.add_argument("feature", nargs="?", help="feature identity such as F001")
    parser.add_argument("spec_path", nargs="?", help="spec directory, for record-spec")
    parser.add_argument("--json", action="store_true", help="emit machine-readable output")
    parser.add_argument("--map", dest="map_path", help="path to the feature map")
    parser.add_argument("--slug", help="kebab-case slug for a new feature")
    parser.add_argument("--after", help="identity the new feature is positioned after")
    parser.add_argument("--depends", default="", help="comma-separated dependencies")
    parser.add_argument("--parallel", action="store_true", help="mark the new feature [P]")
    parser.add_argument(
        "--subfeature",
        action="append",
        default=[],
        dest="subfeatures",
        help="one subfeature; repeat the flag for each",
    )
    parser.add_argument("--dry-run", action="store_true", help="preview without writing")
    args = parser.parse_args(argv)

    if args.map_path:
        path = Path(args.map_path)
    else:
        root = find_project_root()
        if root is None:
            print(
                "ERROR: not inside a Spec Kit project (no .specify directory found)",
                file=sys.stderr,
            )
            return 1
        path = root / DEFAULT_MAP_RELATIVE

    created = False
    if not path.is_file():
        if args.action != "add":
            emit(
                {
                    "STATUS": "skipped",
                    "REASON": f"no feature map at {path}; the sequence gate does not apply",
                },
                args.json,
            )
            return 0
        if args.dry_run:
            features = []
        else:
            try:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(SKELETON, encoding="utf-8")
            except OSError as error:
                print(f"ERROR: cannot create {path}: {error}", file=sys.stderr)
                return 1
            created = True
            features = []
    else:
        try:
            features = load(path)
        except FeatureMapError as error:
            print(f"ERROR: {error}", file=sys.stderr)
            return 1

    problems = verify(features)
    if problems and args.action != "verify":
        emit(
            {
                "STATUS": "error",
                "REASON": "feature map integrity failure",
                "PROBLEMS": problems,
            },
            args.json,
        )
        if not args.json:
            for problem in problems:
                print(f"  {problem}")
        return 2

    if args.action == "verify":
        payload = {
            "STATUS": "error" if problems else "ok",
            "PROBLEMS": problems,
            "FEATURE_COUNT": len(features),
            "COMPLETE": sum(1 for feature in features if feature.done),
        }
        emit(payload, args.json)
        if problems and not args.json:
            for problem in problems:
                print(f"  {problem}")
        return 2 if problems else 0

    if args.action == "add":
        if not args.slug:
            print("ERROR: add requires --slug", file=sys.stderr)
            return 1
        try:
            payload = add_feature(
                path,
                features,
                slug=args.slug,
                after=args.after,
                depends=args.depends.split(","),
                parallel=args.parallel,
                subfeatures=args.subfeatures,
                dry_run=args.dry_run,
            )
        except FeatureMapError as error:
            print(f"ERROR: {error}", file=sys.stderr)
            return 1
        payload["MAP_PATH"] = str(path)
        payload["MAP_CREATED"] = created
        if args.json:
            print(json.dumps(payload, indent=2))
        else:
            if created:
                print(f"CREATED: {path}")
            print(f"{'PREVIEW' if payload['STATUS'] == 'preview' else 'ADDED'}: {payload['FEATURE_ID']}")
            print(payload["BLOCK"], end="")
        return 0

    if args.action == "record-spec":
        if not args.feature or not args.spec_path:
            print("ERROR: record-spec requires a feature identity and a spec path", file=sys.stderr)
            return 1
        try:
            message = record_spec(path, features, args.feature, args.spec_path)
        except FeatureMapError as error:
            print(f"ERROR: {error}", file=sys.stderr)
            return 1
        print(message)
        return 0

    payload = resolve(features, args.feature)
    emit(payload, args.json)
    return 0 if payload["STATUS"] == "ready" else 2


if __name__ == "__main__":
    raise SystemExit(main())
