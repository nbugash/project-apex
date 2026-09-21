// T033 — US1. Every corruption shape still launches (FR-008, SC-003).
import { relaunch, waitForShell, writeSessionRaw } from './helpers';

const CORRUPTIONS: Array<[string, string]> = [
  ['malformed json', 'not json at all'],
  ['truncated', '{"schema_version": 1, "window": {'],
  ['empty file', ''],
  ['wrong shape', '{"unexpected": true}'],
  [
    'dangling focus reference',
    JSON.stringify({
      schema_version: 1,
      workspace: null,
      window: { x: 100, y: 100, width: 1200, height: 800, maximized: false },
      layout: {
        navigation: { visible: true, extent: 260 },
        output: { visible: true, extent: 200 },
        document_area: { visible: true, extent: 0 },
      },
      documents: [],
      focused_document_id: 'ghost',
    }),
  ],
  ['future schema version', '{"schema_version": 99}'],
];

describe('corrupt persisted state never prevents launch', () => {
  before(waitForShell);

  for (const [label, body] of CORRUPTIONS) {
    it(`launches with defaults after ${label}`, async () => {
      writeSessionRaw(body);
      await relaunch();

      // The window opens, the frame renders, and nothing reports an error: falling back
      // is a normal path, not a failure the user should be told about.
      expect(await $('.shell').isExisting()).toBe(true);
      expect(await $('[aria-label="Project navigation"]').isExisting()).toBe(true);
      expect(await $('[role="alert"]').isExisting()).toBe(false);
    });
  }
});
