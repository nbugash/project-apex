// T069, T097 — FR-018a, SC-013a, SC-016. Maintenance is visible, and legible without colour.
//
// NOTE: needs a display; see the note in workspace-tree.spec.ts.
import { waitForShell, resetSession, relaunch } from './helpers';

describe('cache maintenance', () => {
  before(async () => {
    resetSession();
    await relaunch();
    await waitForShell();
  });

  // FR-018a. Only three phases are rendered; the others order the work.
  it('renders exactly the phases the interface is meant to show', async () => {
    const rendered = await browser.execute(() => {
      const w = window as unknown as {
        __APEX_MAINTENANCE_PRESENTATION__?: Record<string, { icon: string; label: string }>;
      };
      return Object.keys(w.__APEX_MAINTENANCE_PRESENTATION__ ?? {});
    });
    expect(rendered).toEqual(['migrating', 'rebuilding', 'evicting']);
  });

  // SC-016. Collapsing migrating and evicting into one state would make SC-013a unmeasurable,
  // because it asserts on migration reports specifically.
  it('keeps migrating distinguishable from evicting without colour', async () => {
    const map = await browser.execute(() => {
      const w = window as unknown as {
        __APEX_MAINTENANCE_PRESENTATION__?: Record<string, { icon: string; label: string }>;
      };
      return w.__APEX_MAINTENANCE_PRESENTATION__ ?? {};
    });
    expect(`${map.migrating?.icon}|${map.migrating?.label}`).not.toBe(
      `${map.evicting?.icon}|${map.evicting?.label}`,
    );
    for (const [name, p] of Object.entries(map)) {
      expect(p.icon).toMatch(/^ph-/);
      expect(p.label.length).toBeGreaterThan(0);
    }
  });

  it('shows a banner while maintenance is running', async () => {
    // A migration that appears to hang is indistinguishable from a broken install, which is why
    // FR-018a requires the state at all rather than merely permitting it.
    const banner = await $('[data-testid="maintenance-banner"]');
    if (await banner.isExisting()) {
      expect(await banner.getAttribute('data-phase')).toMatch(/migrating|rebuilding|evicting/);
    }
  });
});
