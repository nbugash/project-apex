# Research: Shell Chrome Fidelity

**Branch**: `002-shell-chrome-fidelity` | **Date**: 2026-09-21 | **Plan**: [plan.md](./plan.md)

Decisions taken during Phase 0. One is marked for promotion to Appendix A, because it changes
how every future change to the persisted session behaves.

---

## Extending the persisted session without discarding it

**Promote to Appendix A.** This sets the migration policy for every later schema change.

**Decision**: Raise the session schema version and migrate forward on load. A file at any
version up to the current one is accepted, its missing fields filled with defaults, and
rewritten at the current version. Only a version *newer* than the running application is
discarded.

**Rationale**: The store F000 built treats any `schema_version` that is not exactly current as
incoherent, and discards the file in full. That was right when there was one version: a
mismatch could only mean a file from the future, which this build cannot interpret.

It becomes wrong the moment a second version exists. Tool window state adds fields, which
raises the version, which under the current rule would discard every existing user's window
geometry, layout and open tabs on upgrade — a silent data loss caused purely by shipping a
feature. The requirement that invalid state must not prevent launch (FR-008 of the shell) was
never intended to mean that *valid older* state should be thrown away.

Discarding a *newer* file remains correct, and stays. A build cannot know what a future
version means, and guessing risks corrupting it on the next write.

**Alternatives considered**:

- *Add the fields as optional and leave the version at 1.* Avoids migration entirely, and the
  deserialiser would fill defaults. Rejected because it makes the version number meaningless:
  the first genuinely incompatible change would have no way to distinguish itself, and the
  contract's schema would no longer describe what is on disk.
- *Bump the version and accept the loss.* Honest but indefensible: the user loses their layout
  because we added a panel.
- *Write a separate file for tool window state.* Avoids touching the existing schema, at the
  cost of two files that can disagree about one session. The lifetimes are identical, so there
  is no reason to split them — unlike A-STATE, where the split was between a durable
  preference and a disposable cache.

**Reversal condition**: If a future change cannot be expressed as "fill defaults for missing
fields", migration needs a real transformation step per version rather than a single defaults
pass, and this decision should be revisited rather than stretched.

---

## Where the prototype's layout dimensions live

**Decision**: Extract the prototype's layout values at build time into a generated token file,
alongside the design system copy that `ds:sync` already produces. They are never hand-copied
into application code.

**Rationale**: FR-009 requires every dimension to be a token rather than a literal, but these
values are not in the design system. Confirmed by inspection: the design system stylesheet
defines colour, spacing, radius, shadow and font tokens and contains no `--vk-*` layout
values. They exist only as inline fallbacks inside the prototype's own markup — `var(--vk-tool,
276px)` and similar.

That leaves three places the value could come from, and two are forbidden or fragile. Editing
the design system to add them would modify a signed-off artifact, which Principle I prohibits.
Hand-authoring them in application code makes the application the source of truth for a value
the prototype owns, which is the drift the principle exists to prevent.

Extracting them is the same mechanism already used for the design system itself: derived on
every build, so the only way to change a value is to change the prototype, and that is a
visible, reviewable act.

**Alternatives considered**:

- *Add the values to the design system stylesheet.* Rejected: modifies a signed-off artifact.
- *Hand-author a project token layer.* Rejected: silent drift, and it is precisely what the
  design system is copied rather than transcribed to avoid.
- *Read them at runtime from the prototype.* Rejected for the same reason the design system is
  copied at build time: it couples the shipped bundle to a directory that exists for design
  review and would break packaging.

---

## How fidelity is measured

**Decision**: Two complementary checks in one command. A geometry check compares the measured
position and size of each named surface against a baseline, using the 2-device-pixel position
tolerance. A pixel check compares the rendered window against a baseline image, using the 0.5%
area tolerance. Both must pass.

**Rationale**: Neither alone is sufficient, and the reasons are different.

Geometry alone cannot see a wrong colour, a missing border, a substituted icon or a font that
failed to load — the surface is exactly where it should be and looks wrong. Pixels alone can
see all of that but reports "1.4% of pixels differ" without saying which surface moved, which
makes a failure expensive to diagnose and therefore likely to be ignored.

Together the geometry check says *what* moved and the pixel check says *that something looks
different*. A gate whose failures are cheap to read is a gate people keep.

**Alternatives considered**:

- *Pixel comparison only.* Simplest, and the usual approach. Rejected on diagnosability: the
  most common failure mode of a visual gate is being switched off after a run of failures
  nobody could act on.
- *Geometry only.* Cheap and precise, and blind to everything that is not a rectangle.
- *A third-party visual regression service.* Rejected: an external dependency and an account,
  for a comparison that is two commands against a golden file.

**Tooling**: ImageMagick, already required by the end-to-end harness for screenshot capture
and confirmed present, provides the pixel comparison. No new prerequisite.

---

## Capturing and updating the baseline

**Decision**: The baseline is a committed golden file — a reference image plus the measured
geometry. Updating it is a separate, explicit command. The comparison never writes it.

**Rationale**: A baseline that regenerates as a side effect of running the comparison compares
a fresh capture against a fresh capture and passes unconditionally. That is worse than having
no gate, because it reports success. The repository's own ignore rules already articulate this
for a reference file — the baseline is tracked precisely because it is the thing being judged
against, not an artifact of judging.

Making the update explicit also puts a design change in front of a reviewer: the diff shows
the approved appearance changing, which is exactly the moment someone should look.

**Alternatives considered**:

- *Regenerate on every run.* Rejected, as above; it is a gate that cannot fail.
- *Regenerate when the comparison fails.* Rejected for the same reason, with an extra step.
- *Store the baseline outside the repository.* Rejected: it would no longer appear in review,
  which is most of its value.

---

## Where the rail's destinations come from

**Decision**: A static list derived from the prototype, each entry carrying an availability
flag. Destinations whose tool window does not exist yet are present and visibly unavailable.

**Rationale**: FR-008 requires an unbuilt destination to be visibly unavailable rather than
present and inert, and the spec assumes destinations are shown rather than omitted so the
rail's proportions match the prototype from the outset. A registry that features add to would
give a rail that grows over months and never matches the prototype until the last feature
lands.

Most destinations map to features that do not exist yet — version control to F011, search to
F013, run configurations to F010. A static list is honest about that: the rail is complete and
most of it is not yet available.

**Alternatives considered**:

- *A registry each feature contributes to.* Rejected: the rail would not match the prototype
  until every feature is complete, and fidelity is the point of this feature.
- *Omit unavailable destinations.* Rejected: the rail's proportions would differ from the
  prototype, and the spec explicitly assumes otherwise.

---

## Collapse behaviour and remembered width

**Decision**: Selecting the active destination collapses the tool window; selecting it again
restores it at the width it had. The width is retained while collapsed rather than reset.

**Rationale**: This mirrors the behaviour the shell already has for hiding a region, where the
extent is kept so showing it again restores the previous size. Consistency inside one
application matters more than matching any particular external convention, and the alternative
— restoring at a default width — discards a deliberate adjustment for no benefit.

**Alternatives considered**:

- *A separate close control rather than toggling the active destination.* More discoverable,
  but the prototype shows no such control, and inventing one is a deviation requiring designer
  approval.
- *Reset to a default width on restore.* Rejected: throws away the user's sizing.
