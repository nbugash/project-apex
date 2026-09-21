// T034 — US1. The window always opens somewhere reachable (FR-009, SC-009).
//
// A detached monitor cannot be simulated in CI, so this asserts the observable invariant:
// whatever is persisted, the window opens within the attached display. The detach case
// itself is covered exhaustively by src-tauri/tests/display_geometry.rs, where display
// topology is an input rather than hardware.
import { relaunch, waitForShell, writeSessionRaw } from './helpers';

const offScreen = JSON.stringify({
  schema_version: 1,
  workspace: null,
  window: { x: 9000, y: 9000, width: 1000, height: 700, maximized: false },
  layout: {
    navigation: { visible: true, extent: 260 },
    output: { visible: true, extent: 200 },
    document_area: { visible: true, extent: 0 },
  },
  documents: [],
  focused_document_id: null,
});

describe('window recovery onto an attached display', () => {
  before(waitForShell);

  it('opens on-screen even when the saved position is far outside every display', async () => {
    writeSessionRaw(offScreen);
    await relaunch();

    const { width, height } = await browser.execute(() => ({
      width: window.screen.availWidth,
      height: window.screen.availHeight,
    }));
    const { x, y } = await browser.execute(() => ({ x: window.screenX, y: window.screenY }));

    expect(x).toBeLessThan(width);
    expect(y).toBeLessThan(height);
    expect(await $('.shell').isDisplayed()).toBe(true);
  });
});
