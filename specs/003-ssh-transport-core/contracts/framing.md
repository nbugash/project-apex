# Contract: Wire framing

**Branch**: `feature/F001-ssh-transport-core` | **Date**: 2026-09-22

Conformance rules for the codec that reads and writes frames on the child process's stdio.

**The format itself is defined in §4.1 of the system specification and is not restated
here** — not the header spelling, not the separator, not the cap's value. This document says
what an implementation must _do_ about that format, which §4.1 does not cover.

Why the split: a wire format written down twice diverges on the first edit, and the copy
that disagrees is invariably the one someone reads. See research.md, "Where the protocol's
constants live".

---

## Reading

1. **Read the header, then exactly the declared number of bytes.** Not until a delimiter, not
   until the buffer looks like valid JSON. The length is authoritative.

2. **Refuse a declared length above the cap before allocating.** The cap is a defence against
   a hostile or broken peer, so checking it after allocating the buffer defeats it entirely.

3. **A refused frame must not desynchronise the stream.** After refusing, the reader is
   positioned at the next frame boundary or the connection is closed — never left midway
   through a body that will be read as a header.

   This is the single most important rule here. A reader that misaligns turns one bad frame
   into every subsequent frame being garbage, which presents as the engine having gone insane
   rather than as one malformed message.

4. **A body that is not valid JSON is a protocol error for that frame alone.** The frame is
   discarded, the error reported, and reading continues.

5. **A partial frame at EOF is a lost connection, not a protocol error.** The distinction
   matters: one triggers reconnection, the other does not.

---

## Writing

1. **Header and body are written as one unit.** A frame is never partially written and then
   abandoned; the length has already promised what follows.

2. **Writes are serialised.** Exactly one writer touches the child's stdin. Two concurrent
   writers would interleave bytes and corrupt both frames — and the corruption would appear
   as a length mismatch somewhere downstream, far from its cause.

3. **A frame over the cap is refused before any byte is written**, surfaced to the caller as
   a failure (contracts/transport.md, "Refuses").

---

## Correlation

1. **Register before writing** (FR-011). A reply cannot arrive before its receiver exists.

2. **A reply whose id matches nothing is discarded**, without disturbing anything in flight.
   This is normal: a request that timed out or was withdrawn may still be answered.

3. **A second reply for an already-resolved id is discarded.** Resolving twice is impossible
   by construction, not merely unlikely.

4. **A duplicate outbound id is a defect, refused at registration** rather than silently
   delivering one answer to two callers.

---

## Ordering

1. **Priority applies between frames, never within one** (research.md, "Head-of-line blocking
   within a frame"). A frame being written completes first.

2. **FIFO within a class.** Two `Interactive` requests are written in the order submitted.

---

## Conformance

Both implementations satisfy this: the OpenSSH-backed codec and the mock. The mock is the
only way to produce the hostile inputs below, which is why it belongs to this feature rather
than to a test file.

A conforming implementation survives each of these without desynchronising or exhausting
memory, and without affecting any request in flight:

| Input                     | Required behaviour                                    |
| ------------------------- | ----------------------------------------------------- |
| Length above the cap      | Refuse before allocating; stay aligned                |
| Length that never arrives | Treat EOF as connection loss                          |
| Body that is not JSON     | Discard that frame; keep reading                      |
| Reply id matching nothing | Discard silently                                      |
| Reply id already resolved | Discard silently                                      |
| Two replies, same id      | First resolves; second discarded                      |
| Header split across reads | Reassemble; frames are not aligned to read boundaries |
| Body split across reads   | Reassemble                                            |
| Two frames in one read    | Both delivered                                        |

The last three are not hostile input — they are what a pipe does normally, and an
implementation that assumes one read yields one frame will fail intermittently under load
rather than reliably in a test.
