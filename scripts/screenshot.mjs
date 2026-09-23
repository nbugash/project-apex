// Capture the running application to a PNG.
//
// Exists because this project is developed on a headless EC2 box where nobody can look at the
// GUI. The launch and capture logic is the fidelity gate's, imported rather than copied: it
// already knows the two things that are easy to get wrong — that a debug build renders blank
// without a server on devUrl, and that the X window carrying the process name is a 10x10
// placeholder rather than the window a user sees.

import { withShell, captureWindow } from '../tools/gate-fidelity/measure.mjs';
import { mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';

const out = resolve(process.argv[2] ?? 'reports/app.png');
await mkdir(dirname(out), { recursive: true });

await withShell(async () => {
  // A moment for the interface to paint. The gate waits on a readiness signal; here a short
  // settle is enough, and a blank capture is obvious to the person looking at it.
  await new Promise((r) => setTimeout(r, 1500));
  captureWindow(out);
});

console.log(`screenshot: ${out}`);
