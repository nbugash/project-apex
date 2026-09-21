# Feature Map Sequencing

An ordered feature backlog in Markdown, plus a gate that refuses to specify a feature whose
prerequisites are incomplete.

## Why

Spec Kit specifies one feature at a time, but nothing decides which feature is next or
notices when a feature is started before the work it depends on exists. This extension
makes that ordering explicit and enforceable:

- Feature identities (`F000`, `F001`, ...) are immutable. They are never renumbered and
  never reused, so specification directories, design documents, commit messages and review
  threads keep pointing at the same thing forever.
- File position conveys build order. A feature inserted in the middle takes the next unused
  identity and sits where it belongs, rather than forcing a renumber.
- Checkboxes are evidence of completed work, not intention. The gate reads them as fact.
- Dependencies are declared per feature, so features marked `[P]` stay parallel instead of
  being serialised by their position in the file.

## Install

```bash
specify extension add featuremap
```

The map lives at `specs/features-map.md` and is optional. With no map present the hook
reports `skipped` and the specify workflow behaves exactly as it does without this
extension.

The first `speckit.featuremap.add` creates the map and assigns `F000`. Only `add` creates
it: resolving never does, because the gate runs on every specification and must not opt a
project into sequencing it did not ask for.

## Map format

```markdown
- [x] **F000 engineering-baseline**
  - Spec: specs/001-engineering-baseline
  - [x] Import-boundary lint rule
  - [x] Budget harness

- [ ] **F001 plugin-host-runtime** (depends: F000)
  - Spec: not yet specified
  - [ ] Plugin manifest format
  - [ ] Extension point registry

- [ ] **F002 remote-session** [P] (depends: F000)
  - Spec: not yet specified
  - [ ] Connection lifecycle
```

The format is strict because it is parsed: a three-digit identity, a lowercase-kebab slug,
both inside `**`, two-space indentation on the `Spec:` and subfeature lines, and
dependencies spelled `(depends: F000, F001)`. `[P]` marks a feature independent of its
neighbours.

## Commands

| Command | Purpose |
| --- | --- |
| `speckit.featuremap.check` | Mandatory `before_specify` hook: resolves the target feature and blocks when its dependencies are incomplete. Also routes `ADD` mode. |
| `speckit.featuremap.add` | Adds a feature with the correct identity, position and dependencies. |

Because `check` runs as the specify hook, `/speckit-specify ADD <description>` is routed to
the add command instead of writing a specification.

## Script

`scripts/python/feature_map.py`, installed at
`.specify/extensions/featuremap/scripts/python/feature_map.py`.

| Action | Effect |
| --- | --- |
| `resolve [F0NN]` | Names the feature that may be specified, or explains what blocks it. With no identity, picks the first unchecked feature whose dependencies are all checked. |
| `verify` | Reports integrity problems: a checked feature with unchecked subfeatures, a duplicate identity, an unknown dependency, or a dependency positioned below its dependent. |
| `add --slug ... --after ... --depends ... --subfeature ...` | Inserts a feature, assigning the next unused identity. Re-verifies afterwards and rolls back if integrity would break. |
| `record-spec F0NN specs/NNN-slug` | Records which specification directory a feature produced. |

All actions accept `--json` for machine-readable output and `--map` to point at a map
outside the default `specs/features-map.md`. Exit codes: `0` ready or skipped, `2` blocked or
integrity failure, `1` usage or I/O error.

## Relationship to a project constitution

A project may define this sequencing as a governing principle, in which case the
constitution is the authority on what a checked box means and this extension is the
mechanism that enforces it. The extension works without one.
