# Research: Editor Integration

**Feature**: F006 `editor-integration` | **Date**: 2026-09-26 | **Plan**: [plan.md](./plan.md)

Each entry records a decision, why it was taken, and what was rejected. The rejected
alternatives are the load-bearing part: without them a later reader cannot tell a choice from an
accident.

---

## Writing is the first method that can destroy something

**Decision.** `workspace/writeFile` resolves its path through the **same** `resolve_request`
the read methods use, refuses anything outside the workspace root with `-32003`, writes to a
temporary file in the destination's own directory and renames it into place, and refuses content
above a stated bound before any of that.

**Rationale.** Every engine method until now has been a read. A malformed path on a read leaks
information; the same path on a write destroys a file, and the engine runs with the developer's
full filesystem rights (Principle VI's rationale says exactly this). Three properties follow:

*Containment is not re-implemented.* `ResolvedPath::resolve` already canonicalises against the
registered root and returns `PathRefusal::Refused` for an escape, and it is what
`readFile`, `stat` and `readDirectory` use. A second containment check written for the write
path would be a second thing to keep correct, and the one that is wrong is always the one
written last. Reusing it also means symlink escape is handled by construction, because
canonicalisation resolves the link before the comparison.

*Rename rather than truncate-and-write.* `write` onto the target truncates it first, so a
failure anywhere after that leaves a file shorter than either version — content that never
existed. Writing beside it and renaming makes the change one filesystem operation from the
reader's point of view, which is what FR-015 asks for: a failed write leaves the previous
content byte-for-byte intact.

*A size bound before the work.* A frame is capped at 1 MiB (§4.1), which bounds a single
`writeFile`, but the bound belongs in the use case too — an engine whose only protection is the
codec is one that breaks the moment something else calls it.

**Alternatives rejected.**

- *Write in place, truncating.* Simplest, and the failure mode is a truncated source file. No.
- *A fresh containment check for writes.* Tempting because writing feels like it deserves its
  own care. It deserves the same care, applied by the same code.
- *Copy-on-write backup before overwriting.* Real protection against a bad write, and it doubles
  the write cost for every save and leaves litter to reap. The base-hash check already prevents
  the destructive case this would mitigate.

---

## Telling our own write apart from someone else's

**Decision.** On `workspace/onFileEvent` for a file that is open, the client asks for the file's
current hash with `workspace/stat` and compares it with the buffer's base. Equal means nothing
diverged. Only a difference is treated as a change made on the host.

**Rationale.** The engine's own write trips its own watcher: the inotify mask includes `MODIFY`
and `CLOSE_WRITE`, so every successful save produces an event for the file just saved. An editor
that took events at face value would tell the developer their file had changed underneath them
on every single save.

The event cannot answer the question itself. `FileEvent` carries the kind, the path, and
optionally type, size and modified time — **no hash** (§4.8). Size is not sufficient: an edit
that changes a character changes no size. So the hash has to be asked for, and `workspace/stat`
already returns one.

The cost is one small round trip per event per open file, which is bounded by how many files are
open rather than by workspace size, and it is off the keystroke path entirely.

**Alternatives rejected.**

- *Suppress events for a path for a short window after writing it.* Free, and wrong in the one
  case that matters: a colleague's edit landing inside the window is silently discarded. The
  failure this whole feature exists to prevent, reintroduced as an optimisation.
- *Compare the size in the event.* Cheap, and blind to every edit that preserves length.
- *Have the engine tag events it caused.* Correct in principle and a protocol change, for a
  problem the client can answer with a method that already exists. If the round trip ever
  measures as a problem, this is the escalation.

---

## The editor's palette is five colours and a deferral

**Decision.** `ds-sync` extracts the five colours the prototype's editor actually distinguishes —
punctuation, function name, keyword, type, comment — as `--vk-code-*` tokens. Monaco's theme is
built from those plus the existing surface tokens. Every other role in Monaco's token set falls
back to the editor's foreground colour, and a complete syntax theme is logged as **owed to the
design system**.

**Rationale.** The prototype's `Editor` screen writes its code colours as raw hex — `#9397ab`
for punctuation and operators, `#e4e7f5` for names, `#b5abfc` for keywords, `#d2cefd` for types,
`#75798c` for comments. None is a design-system token. Under Principle I those hexes are the
specification of appearance, so they bind; and under the same principle a component may not
write them, so they are extracted rather than transcribed.

Five is not a target, it is a count: it is what the prototype distinguishes. Inventing colours
for the other forty-odd roles Monaco knows about would be this feature deciding what the design
system looks like, which is the drift Principle I exists to prevent. Falling back to the
foreground is honest — unstyled, not mis-styled.

This is A-TERMPALETTE's shape, one surface over. That record mapped three hues and deferred
eleven for the terminal, on the same grounds, and it is the precedent rather than a coincidence.

**Alternatives rejected.**

- *Use one of Monaco's built-in themes.* Immediate, complete, and a different visual language
  from the rest of the application — the definition of drift.
- *Invent tokens for the missing roles.* Produces a prettier editor and an unapproved design.
- *Ship no syntax colour at all until the design system defines one.* Defensible, and worse than
  the prototype: the prototype does colour code, so plain text would be a regression from a
  signed-off artifact.

---

## Where the base hash lives, and what "valid" means

**Decision.** The buffer holds the base hash. The cache holds content and its hash, as F003
already stores them. A buffer's base is set when the content is read and replaced by the hash a
successful write returns. Nothing else writes it.

**Rationale.** A base is a property of an editing session, not of a cache entry: two sessions of
the same file would have the same cached content and may have different bases. Keeping it on the
buffer also makes FR-023's one-buffer-per-file rule the thing that keeps bases consistent —
there is only one, so there is nothing to reconcile.

FR-001 says the editor reads from the cache "when the cache holds valid content". Validity is
F003's existing notion — content whose hash matches what the last `stat` reported — and this
feature adds nothing to it.

**Alternatives rejected.**

- *Keep the base in the cache row.* Makes the cache the arbiter of an editing concern and gives
  a second writer to a column F003 owns.
- *Re-`stat` before every save to get a fresh base.* Defeats the purpose: it would adopt a
  colleague's change as the base and then overwrite it. The whole point is that the base is what
  the developer **started from**.

---

## Ranges, and what a partially loaded buffer is allowed to do

**Decision.** A file above the threshold loads the range covering the first viewport, and
further ranges as the viewport moves. A buffer with unloaded regions is **read-only**: it may be
scrolled and read, not edited, until it is whole. Editing triggers loading the remainder.

**Rationale.** The alternative is an editor that lets a developer type into a file it has only
partly seen and then writes the whole thing back — which would replace the unloaded regions with
nothing. A write is whole-file (§4.8 carries `content`, not a patch), so a partial buffer cannot
be written safely, and the safest interlock is the simplest: do not accept edits until the file
is whole.

In practice a developer who opens a 40 MiB log is reading it; one who intends to edit waits for
a load they can see. Making that visible is better than making the failure silent.

**Alternatives rejected.**

- *Allow editing and fetch the rest before writing.* Works, and makes a save unbounded in time
  at the moment the developer least expects it.
- *Send a patch instead of the whole file.* Correct in the abstract and a protocol change §4.8
  does not have. A future decision, not this one.
- *Refuse to open large files.* Throws away the ranged read F003 already built.

---

## Why Monaco's own workers are off

**Decision.** Monaco is configured with no language web workers.

**Rationale.** Its workers implement completion, hover and diagnostics against the text in the
browser. Every one of those answers would be computed from a single file with no knowledge of
the workspace, while §8.1 requires them to come from the engine's language servers. A local
suggestion that looks like a real one is worse than none, because the developer cannot tell them
apart — and F007 has not shipped, so there is nothing to compare against.

Syntax highlighting stays, because it is lexical, local by design (§8.1) and required to survive
a dropped connection.

**Alternatives rejected.**

- *Leave the workers on until F007 replaces them.* Ships an editor that lies about a remote
  workspace, and trains the developer to trust an answer that is about to change meaning.
- *Disable highlighting too.* Confuses "local analysis of a remote workspace", which is wrong,
  with "local rendering of local text", which §8.1 explicitly wants.

---

## Saving is explicit, with autosave available and off

**Decision.** Recorded in spec.md's *Clarifications*. Explicit save always; autosave available,
off on a profile that has never set it, preference stored in the session store A-STATE defines.

**Rationale.** Every save is a chance for a conflict, because every save carries a base hash.
Making saves deliberate keeps refusals rare and meaningful. Autosave exists because a developer
who wants it has a real reason — no crash-recovery buffer exists until F012 — and off by default
because a refusal in the middle of typing is a worse experience than a forgotten save.

**Alternatives rejected.** Autosave-only, and explicit-only, both recorded with their costs in
the clarification session.

**Consequence.** A preference needs somewhere to be set, and no settings surface exists. The
editor carries one control; a settings screen belongs to whichever feature eventually owns
preferences. Building the mechanism with no way to reach it would be dead code.
