// Copyright (c) 2026 QubeTX - ES Development LLC. All rights reserved.

import { readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const source = path.resolve(process.env.SPEEDQX_CANONICAL ?? path.join(root, '../speedtest'));
const manifestPath = path.join(root, 'src/speedtest/canonical-v5.json');
const entries = [
  ['src/services/measurement-contract-v5.json', 'src/speedtest/measurement-contract-v5.json'],
  ['measurement-v5-fixtures.json', 'src/speedtest/measurement-v5-fixtures.json'],
  ['METHODOLOGY.md', 'METHODOLOGY.md'],
];
const canonical = text => text.replace(/\r\n/g, '\n');
const digest = text => createHash('sha256').update(canonical(text)).digest('hex');
if (process.argv.includes('--check')) {
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
  if (manifest.methodology !== '5.0' || !/^[a-f0-9]{40}$/.test(manifest.sourceRevision)) throw new Error('Invalid canonical pin');
  if (process.env.CI && manifest.sourceDirty) throw new Error('Commit the canonical source before CI/release');
  for (const [from, to] of entries) {
    const pin = manifest.files[from];
    if (!pin || pin.target !== to || digest(await readFile(path.join(root, to), 'utf8')) !== pin.sha256) throw new Error(`Contract drift: ${to}`);
    if (process.argv.includes('--source-check') && digest(await readFile(path.join(source, from), 'utf8')) !== pin.sha256) throw new Error(`Canonical drift: ${from}`);
  }
  console.log('Canonical v5 contract, fixtures and methodology verified.');
} else {
  const sourceRevision = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: source, encoding: 'utf8' }).trim();
  const changed = execFileSync('git', ['status', '--porcelain', '--', ...entries.map(([from]) => from)], { cwd: source, encoding: 'utf8' }).trim();
  const manifest = { methodology: '5.0', source: 'QubeTX/speedtest', sourceRevision, sourceDirty: !!changed, files: {} };
  for (const [from, to] of entries) {
    const contents = canonical(await readFile(path.join(source, from), 'utf8'));
    await writeFile(path.join(root, to), contents);
    manifest.files[from] = { target: to, sha256: digest(contents) };
  }
  await writeFile(manifestPath, JSON.stringify(manifest, null, 2) + '\n');
  console.log(`Pinned v5 contract from ${sourceRevision}${changed ? ' (working changes; re-pin after commit)' : ''}.`);
}
