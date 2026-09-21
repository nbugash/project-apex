#!/usr/bin/env python3
"""Setup implementation plan for a feature."""

from __future__ import annotations

import json
import sys
from pathlib import Path

try:
    from common import (
        TemplateResolutionError,
        get_feature_paths,
        resolve_template_content,
    )
except ImportError:  # pragma: no cover - direct execution from unusual cwd
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from common import (
        TemplateResolutionError,
        get_feature_paths,
        resolve_template_content,
    )


def _resolve_optional_template(name: str, repo_root: Path) -> str:
    """Resolve a Phase 2 design template, degrading to an empty string when absent.

    Mirrors the two-tier handling ``plan-template`` already receives: a template
    that is simply missing is tolerable, so a project with stale shared templates
    can still plan, while a template that exists but cannot be composed raises.
    The plan command treats an empty value as a blocking error rather than
    inventing a section structure.
    """
    content = resolve_template_content(name, repo_root)
    if content is None:
        print(f"Warning: {name} not found", file=sys.stderr)
        return ""
    return content


def _json_line(payload: object) -> str:
    return json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n"


def _help_text(argv0: str) -> str:
    return f"""Usage: {argv0} [--json]
  --json    Output results in JSON format
  --help    Show this help message
"""


def main(argv: list[str] | None = None) -> int:
    args = list(argv if argv is not None else sys.argv[1:])
    json_mode = False
    for arg in args:
        if arg == "--json":
            json_mode = True
        elif arg in {"--help", "-h"}:
            sys.stdout.write(_help_text(sys.argv[0]))
            return 0
        else:
            print(f"ERROR: Unknown option '{arg}'", file=sys.stderr)
            return 1

    try:
        paths = get_feature_paths(script_file=Path(__file__))
    except SystemExit as exc:
        if exc.code == 0:
            return 0
        print("ERROR: Failed to resolve feature paths", file=sys.stderr)
        return int(exc.code) if isinstance(exc.code, int) else 1

    paths.feature_dir.mkdir(parents=True, exist_ok=True)

    # Status messages go to stderr in JSON mode so stdout stays pure JSON.
    status_stream = sys.stderr if json_mode else sys.stdout
    if paths.impl_plan.is_file():
        print(
            f"Plan already exists at {paths.impl_plan}, skipping template copy",
            file=status_stream,
        )
    else:
        try:
            template_content = resolve_template_content("plan-template", paths.repo_root)
        except TemplateResolutionError as exc:
            print(f"Error: {exc}", file=sys.stderr)
            return 1
        if template_content is not None:
            paths.impl_plan.write_bytes(template_content.encode("utf-8"))
            print(f"Copied plan template to {paths.impl_plan}", file=status_stream)
        else:
            print("Warning: Plan template not found", file=status_stream)
            paths.impl_plan.touch()

    try:
        architecture_template_content = _resolve_optional_template(
            "architecture-template", paths.repo_root
        )
        design_template_content = _resolve_optional_template(
            "design-template", paths.repo_root
        )
    except TemplateResolutionError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1

    if json_mode:
        sys.stdout.write(
            _json_line(
                {
                    "FEATURE_SPEC": str(paths.feature_spec),
                    "IMPL_PLAN": str(paths.impl_plan),
                    "FEATURE_DIR": str(paths.feature_dir),
                    "BRANCH": paths.current_branch,
                    "ARCHITECTURE_TEMPLATE_CONTENT": architecture_template_content,
                    "DESIGN_TEMPLATE_CONTENT": design_template_content,
                }
            )
        )
    else:
        print(f"FEATURE_SPEC: {paths.feature_spec}")
        print(f"IMPL_PLAN: {paths.impl_plan}")
        print(f"FEATURE_DIR: {paths.feature_dir}")
        print(f"BRANCH: {paths.current_branch}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
