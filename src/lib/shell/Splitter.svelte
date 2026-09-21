<script lang="ts">
  import { clampExtent, deltaFor } from './extent';

  /** Pointer-driven divider. Hand-rolled rather than adopting a docking framework: the
      specification excludes floating panels, and a framework ships a stylesheet that
      Principle I forbids us from overriding. */
  interface Props {
    orientation: 'vertical' | 'horizontal';
    extent: number;
    min: number;
    max?: number;
    label: string;
    onresize: (extent: number) => void;
  }
  let {
    orientation,
    extent,
    min,
    max = Number.POSITIVE_INFINITY,
    label,
    onresize,
  }: Props = $props();

  const KEYBOARD_STEP = 16;
  let dragging = $state(false);

  const clamp = (v: number) => clampExtent(v, min, max);

  function start(event: PointerEvent) {
    dragging = true;
    (event.target as HTMLElement).setPointerCapture(event.pointerId);
  }

  function move(event: PointerEvent) {
    if (!dragging) return;
    onresize(clamp(extent + deltaFor(orientation, event.movementX, event.movementY)));
  }

  function end(event: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    (event.target as HTMLElement).releasePointerCapture(event.pointerId);
  }

  // FR-018: reachable and operable without a pointer.
  function key(event: KeyboardEvent) {
    const grow = orientation === 'vertical' ? 'ArrowRight' : 'ArrowUp';
    const shrink = orientation === 'vertical' ? 'ArrowLeft' : 'ArrowDown';
    if (event.key === grow) onresize(clamp(extent + KEYBOARD_STEP));
    else if (event.key === shrink) onresize(clamp(extent - KEYBOARD_STEP));
    else return;
    event.preventDefault();
  }
</script>

<div
  class="splitter {orientation}"
  class:dragging
  role="separator"
  tabindex="0"
  aria-label={label}
  aria-orientation={orientation}
  aria-valuenow={extent}
  aria-valuemin={min}
  onpointerdown={start}
  onpointermove={move}
  onpointerup={end}
  onpointercancel={end}
  onkeydown={key}
></div>

<style>
  .splitter {
    background: var(--color-divider);
    flex: 0 0 auto;
    transition: background 120ms ease;
  }
  .splitter.vertical {
    inline-size: var(--space-1);
    cursor: col-resize;
  }
  .splitter.horizontal {
    block-size: var(--space-1);
    cursor: row-resize;
  }
  .splitter:hover,
  .splitter.dragging {
    background: var(--color-accent-700);
  }
  /* Themed focus, never the platform default (FR-021). */
  .splitter:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }
</style>
