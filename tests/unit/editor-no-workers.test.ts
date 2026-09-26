/// FR-004: nothing in the editor answers a question about the workspace from the browser.
///
/// This is a requirement satisfied entirely by *which module was imported*, which makes it the
/// kind a later change removes without anybody noticing — somebody adds
/// `monaco-editor/esm/vs/language/typescript` because completion would be nice, and the
/// application starts answering questions about a remote workspace from a single file held
/// locally. A suggestion that looks real but is not is worse than none, and no other test in
/// this suite would fail.
///
/// Asserted against the **package on disk**, not against a list written here. A list would
/// agree with itself; `node_modules` is where the worker references actually are.
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const MONACO = join(process.cwd(), 'node_modules/monaco-editor/esm/vs');
const PANEL = join(process.cwd(), 'client/ui/lib/editor/EditorPanel.svelte');

/// Every `import ... from '...'` and `import('...')` specifier in a file.
function importsOf(source: string): string[] {
  const out: string[] = [];
  for (const m of source.matchAll(/import\s*\(\s*['"]([^'"]+)['"]\s*\)/g)) out.push(m[1]!);
  for (const m of source.matchAll(/from\s+['"]([^'"]+)['"]/g)) out.push(m[1]!);
  return out;
}

describe('the editor starts no language workers', () => {
  it('imports no worker-backed language service', () => {
    const imports = importsOf(readFileSync(PANEL, 'utf8'));
    const offending = imports.filter((i) => /monaco-editor\/(esm\/vs\/)?language\//.test(i));
    expect(
      offending,
      'the modules under `language/` spawn web workers and answer completion, hover and ' +
        'diagnostics from the single file in the browser — about a workspace they cannot see',
    ).toEqual([]);
  });

  it('imports the Monarch contributions, which are the ones that do not', () => {
    // The positive half. Without it this file would pass for an editor with no syntax colour at
    // all, which is FR-005 broken in exchange for FR-004 satisfied.
    const imports = importsOf(readFileSync(PANEL, 'utf8'));
    expect(imports).toContain('monaco-editor/basic-languages/monaco.contribution');
  });

  it('confirms against the package that the contributions reference no worker', () => {
    // The claim the import rests on, checked where it is true or false rather than in a comment.
    const entry = join(MONACO, 'basic-languages/monaco.contribution.js');
    if (!existsSync(entry)) {
      throw new Error(`monaco-editor is not laid out as expected: ${entry} is missing`);
    }
    expect(readFileSync(entry, 'utf8')).not.toMatch(/worker/i);
  });

  it('confirms the language services it avoids really do reference workers', () => {
    // Otherwise the first assertion proves nothing: a check that forbids imports which would
    // have been harmless is a check that has stopped tracking the hazard it was written for.
    const languages = join(MONACO, 'language');
    const dirs = readdirSync(languages, { withFileTypes: true }).filter((d) => d.isDirectory());
    expect(dirs.length).toBeGreaterThan(0);

    const withWorkers = dirs.filter((d) => {
      const files = readdirSync(join(languages, d.name)).filter((f) => f.endsWith('.js'));
      return files.some((f) => /worker/i.test(readFileSync(join(languages, d.name, f), 'utf8')));
    });
    expect(withWorkers.length).toBe(dirs.length);
  });
});
