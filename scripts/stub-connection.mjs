#!/usr/bin/env node
// Drives the connection stub during development (T057, quickstart scenario 5).
//
// Writes the desired state into the profile directory; the running shell picks it up.
// Deliberately file-based rather than a socket: the shell already owns a profile
// directory, and a second IPC channel existing only for development is a surface that
// outlives its usefulness.
import { mkdirSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

const STATES = ['unknown', 'connecting', 'connected', 'disconnected'];

const stateIndex = process.argv.indexOf('--state');
const state = stateIndex > -1 ? process.argv[stateIndex + 1] : undefined;

if (!state || !STATES.includes(state)) {
  console.error(`usage: npm run stub:connection -- --state <${STATES.join('|')}>`);
  process.exit(1);
}

const dataDir =
  process.env.APEX_DATA_DIR ??
  (process.platform === 'darwin'
    ? join(homedir(), 'Library/Application Support/dev.apex.shell')
    : join(process.env.XDG_DATA_HOME ?? join(homedir(), '.local/share'), 'dev.apex.shell'));

mkdirSync(dataDir, { recursive: true });
writeFileSync(join(dataDir, 'stub-connection'), state);
console.log(`stub connection state -> ${state} (${dataDir})`);
