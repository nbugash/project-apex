// Design-system adherence rules, ported from the signed-off `_adherence.oxlintrc.json`.
//
// The source configuration targeted React via oxlint plugins. The rules it expresses are
// framework-agnostic — only the plugin and parser wiring were React-specific — so every rule
// is preserved verbatim here against the Svelte toolchain, per Constitution Principle I and
// the Design System Compliance section, which requires this lint to run in CI.
import svelteParser from 'svelte-eslint-parser';
import tsParser from '@typescript-eslint/parser';

const designSystemAdherence = [
  {
    selector: 'Literal[value=/#[0-9a-fA-F]{3,8}\\b/]',
    message: 'Raw hex colour — use a design-system colour token via var().',
  },
  {
    selector: 'Literal[value=/\\b\\d+px\\b/]',
    message: 'Raw px value — use a design-system spacing token via var().',
  },
  {
    selector: "Literal[value=/font-family\\s*:\\s*(?!['\"]?(?:Inter|JetBrains))/i]",
    message:
      'Hard-coded font family — use var(--font-heading), var(--font-body) or var(--font-mono).',
  },
];

const rules = { 'no-restricted-syntax': ['error', ...designSystemAdherence] };

export default [
  {
    ignores: [
      'dist/**',
      'node_modules/**',
      // The design system itself is the source of truth, not something we lint against.
      'src/lib/ds/**',
      'src-tauri/target/**',
    ],
  },
  {
    files: ['**/*.svelte'],
    languageOptions: {
      parser: svelteParser,
      parserOptions: { parser: tsParser, ecmaVersion: 2022, sourceType: 'module' },
    },
    rules,
  },
  {
    files: ['**/*.ts'],
    languageOptions: {
      parser: tsParser,
      parserOptions: { ecmaVersion: 2022, sourceType: 'module' },
    },
    rules,
  },
];
