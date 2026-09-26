/**
 * Monaco's language *contributions* have no type declarations, and this says so deliberately
 * rather than by silence.
 *
 * `monaco-editor/basic-languages/monaco.contribution` is imported for its side effect only: it
 * registers every Monarch tokenizer with the editor API and exports nothing a caller uses. The
 * package ships it as plain JavaScript with no `.d.ts`, so `svelte-check` reports an implicit
 * `any` — a real report about a real gap, which is why it is answered here instead of being
 * suppressed at the import.
 *
 * Declared with no shape at all, because it has none worth naming. A hand-written interface
 * would be a claim about a module nobody calls, and the first version of Monaco to change it
 * would leave the claim standing and wrong.
 */
declare module 'monaco-editor/basic-languages/monaco.contribution';
