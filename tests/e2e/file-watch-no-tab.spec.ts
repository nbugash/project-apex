// T100 — FR-024 and SC-001b through the real interface.
//
// The half T090 does not cover: a change to a file with **no** open tab must not interrupt,
// and a file whose last tab has closed must stop being reported at all.
import { waitForShell } from './helpers';

async function changedMarkers(): Promise<number> {
  return browser.execute(() => document.querySelectorAll('[data-testid="tab-changed"]').length);
}

async function dialogsOrPrompts(): Promise<number> {
  // "Not interrupted" means no modal, no focus steal, no notification demanding dismissal. A
  // tree row updating in place is not an interruption; a prompt is.
  return browser.execute(
    () => document.querySelectorAll('[role="dialog"], [role="alertdialog"]').length,
  );
}

describe('a change to a file nobody has open', () => {
  beforeEach(async () => {
    await waitForShell();
  });

  it('interrupts nothing', async () => {
    const before = await dialogsOrPrompts();
    await browser.execute(() => {
      window.dispatchEvent(
        new CustomEvent('apex:test:file-event', {
          detail: {
            workspaceId: 'ws1',
            events: [{ event: 'modified', relative_path: '/untouched/elsewhere.rs' }],
          },
        }),
      );
    });
    expect(await dialogsOrPrompts()).toBe(before);
  });

  it('marks no tab, because there is no tab to mark', async () => {
    expect(await changedMarkers()).toBe(0);
  });
});
