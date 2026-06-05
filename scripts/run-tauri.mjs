#!/usr/bin/env node

import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const binName = process.platform === 'win32' ? 'tauri.cmd' : 'tauri';
const tauriBin = path.join(repoRoot, 'frontend', 'node_modules', '.bin', binName);
const result = spawnSync(tauriBin, process.argv.slice(2), {
  cwd: repoRoot,
  stdio: 'inherit',
  shell: false,
});

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
