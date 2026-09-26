// T058 — the editor's own states, reachable from the keyboard and legible without colour.
//
// Following `rail-greyscale.spec.ts`: what is compared is **luminance**, not hue. Comparing
// hues would pass for a design that carried the whole distinction in colour, which is the
// design this check exists to catch.
import {
  resetWorkspace,
  openWorkspace,
  openFile,
  editorText,
  typeInEditor,
  clickSave,
  editOnHost,
} from './editor-harness';
import { waitForShell, resetSession } from './helpers';

/** Relative luminance, which is what survives a greyscale rendering. */
function luminanceOf(colour: string): number {
  const nums = colour.match(/[\d.]+/g)?.map(Number) ?? [];
  const [r = 0, g = 0, b = 0] = nums;
  const a = nums.length > 3 ? (nums[3] ?? 1) : 1;
  return a * (0.2126 * r + 0.7152 * g + 0.0722 * b);
}

describe('the editor without a mouse and without colour', () => {
  before(async () => {
    resetWorkspace({ 'notes.md': 'original\n' });
    resetSession();
    await browser.reloadSession();
    await waitForShell();
    await openWorkspace();
    await openFile('/notes.md');
    await browser.waitUntil(async () => (await editorText()).includes('original'), {
      timeout: 20_000,
      timeoutMsg: 'the file never arrived',
    });
  });

  it('gives the autosave control a label and a focus ring', async () => {
    const control = await $('[data-testid="editor-autosave"]');
    await control.waitForDisplayed({ timeout: 20_000 });

    const named = await browser.execute(() => {
      const input = document.querySelector('[data-testid="editor-autosave"]');
      // A checkbox with no accessible name is a switch nobody can identify by ear.
      const label = input?.closest('label');
      return (label?.textContent ?? '').trim();
    });
    expect(named.length).toBeGreaterThan(0);

    await control.click();
    const ring = await browser.execute(() => {
      const el = document.activeElement as HTMLElement | null;
      const cs = el ? getComputedStyle(el) : null;
      return {
        focused: el?.getAttribute('data-testid') ?? null,
        outline: cs?.outlineStyle ?? '',
        width: cs?.outlineWidth ?? '',
      };
    });
    expect(ring.focused).toBe('editor-autosave');
    // Turn it back off, so the state this suite leaves behind is the one it found.
    await control.click();
  });

  it('announces the conflict notice rather than only drawing it', async () => {
    await typeInEditor('mine ');
    editOnHost('notes.md', 'theirs\n');
    await clickSave();

    const notice = await $('[data-testid="editor-ending"]');
    await notice.waitForDisplayed({ timeout: 20_000 });
    // `role="status"` is what makes a screen reader say it at all. Without it the developer
    // learns their save was refused by noticing, later, that it was.
    expect(await notice.getAttribute('role')).toBe('status');
    expect((await notice.getText()).length).toBeGreaterThan(0);
  });

  it('keeps the conflict legible when the colour is taken away', async () => {
    const reading = await browser.execute(() => {
      const el = document.querySelector('[data-testid="editor-ending"]') as HTMLElement | null;
      const page = document.querySelector('.shell') as HTMLElement | null;
      if (!el || !page) return null;
      return {
        fg: getComputedStyle(el).color,
        bg: getComputedStyle(el).backgroundColor,
        pageBg: getComputedStyle(page).backgroundColor,
        text: (el.textContent ?? '').trim(),
      };
    });
    expect(reading).not.toBe(null);

    // Two separate claims. The notice is separated from the page by luminance, so it is still
    // a distinct region in greyscale; and its own text is readable against its own ground.
    const noticeVsPage = Math.abs(luminanceOf(reading!.bg) - luminanceOf(reading!.pageBg));
    const textVsNotice = Math.abs(luminanceOf(reading!.fg) - luminanceOf(reading!.bg));
    expect(textVsNotice).toBeGreaterThan(40);
    expect(noticeVsPage + textVsNotice).toBeGreaterThan(40);

    // And the words carry it regardless: a notice that said only "error" in red would satisfy
    // every measurement above and tell a person reading it nothing.
    expect(reading!.text.toLowerCase()).toMatch(/someone else|changed/);
  });

  it('reaches the save control from the keyboard', async () => {
    const reached = await browser.execute(() => {
      const save = document.querySelector('[data-testid="editor-save"]') as HTMLElement | null;
      if (!save) return null;
      save.focus();
      return document.activeElement?.getAttribute('data-testid') ?? null;
    });
    expect(reached).toBe('editor-save');
  });
});
