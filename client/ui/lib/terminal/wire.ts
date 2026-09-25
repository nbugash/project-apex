/**
 * Bytes across the panel's boundary, in both directions.
 *
 * A task's output is bytes, not text. A compiler emits them in whatever encoding it pleases, and
 * a program under test emits whatever it was asked to. §4.8 carries them base64 for exactly that
 * reason, and this module is the only place the panel converts between the wire's form and the
 * terminal's.
 *
 * **Nothing here produces a `String` from output bytes.** `TextDecoder` without `{ stream: true }`
 * turns a chunk boundary falling mid-character into U+FFFD, and a substitution made here is
 * invisible at the engine -- the bytes left it intact. That is SC-003's failure mode, and keeping
 * output as `Uint8Array` all the way into the terminal library, which parses bytes itself, removes
 * the opportunity rather than guarding against it.
 */

/** Wire form to bytes. */
export function decodeBase64(encoded: string): Uint8Array {
  // `atob` yields one character per byte, each in 0..255. Reading `charCodeAt` back out is the
  // byte-exact inverse; anything that goes via a text encoding is not.
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

/** Bytes to wire form. */
export function encodeBase64(bytes: Uint8Array): string {
  let binary = '';
  // Built a character at a time rather than with `String.fromCharCode(...bytes)`, which spreads
  // the whole array onto the call stack and throws on a large keystroke paste.
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** A keystroke as the terminal library reports it, as the bytes the wire carries. */
export function encodeInput(data: string): string {
  return encodeBase64(new TextEncoder().encode(data));
}
