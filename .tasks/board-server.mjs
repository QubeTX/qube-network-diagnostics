#!/usr/bin/env node
// board-server.mjs — zero-dependency live server for the SHAUGHV task board.
//
// Ships inside `.tasks/` (copied there by /tasks-start). Uses ONLY Node built-ins —
// no npm install, no build step. Serves dashboard.html on localhost, reads/writes
// TASKS.md and memory files server-side, and live-syncs the browser via SSE.
//
// Subcommands:
//   node .tasks/board-server.mjs serve [--open] [--port N]   start in foreground
//   node .tasks/board-server.mjs ensure [--open]             start detached if not already running
//   node .tasks/board-server.mjs hook <EVENT>                ensure + print a board-maintenance nudge
//   node .tasks/board-server.mjs stop                        stop a running server
//   node .tasks/board-server.mjs status                      print running state (json)
//
// Paths resolve from THIS file's directory (the `.tasks/` folder), so it works no
// matter what cwd node is invoked from (e.g. a hook fired from the repo root).

import http from 'node:http';
import https from 'node:https';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import net from 'node:net';
import os from 'node:os';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const TASKS_DIR = path.dirname(fileURLToPath(import.meta.url));
const TASKS_MD = path.join(TASKS_DIR, 'TASKS.md');
const MILESTONES_MD = path.join(TASKS_DIR, 'MILESTONES.md');
const CLAUDE_MD = path.join(TASKS_DIR, 'CLAUDE.md');
const MEMORY_DIR = path.join(TASKS_DIR, 'memory');
const TASK_DETAIL_DIR = path.join(TASKS_DIR, 'tasks'); // per-task detail files: tasks/<id>.md
const MILESTONE_DETAIL_DIR = path.join(TASKS_DIR, 'milestones'); // per-milestone detail files: milestones/<id>.md
const SECURE_DIR_NAME = 'secure'; // .tasks/secure — never served, never watched (secrets; see tasks-memory)
const CONFIG_FILE = path.join(TASKS_DIR, 'config.json'); // setup-owned; served read-only at /api/config
const DASHBOARD = path.join(TASKS_DIR, 'dashboard.html');
const STATE_FILE = path.join(TASKS_DIR, '.board-server.json');     // {port, pid, startedAt} — written by the server
const NUDGE_FILE = path.join(TASKS_DIR, '.board-nudge.json');      // {<event>: epochMs} — written by hook, separate to avoid contention
const LOG_FILE = path.join(TASKS_DIR, '.board-server.log');

const DEFAULT_PORT = 4317;
const PING_TOKEN = 'shaughv-task-board';
const NUDGE_COOLDOWN_MS = 30_000; // de-dupe bursty events (e.g. parallel subagent fan-out)

// --- tiered dependency system (see references/board-server.md "Tiered dependencies") ---
const VENDOR_DIR = path.join(TASKS_DIR, 'vendor');                       // provisioned offline assets, served at /vendor/*
const NODE_MODULES_DIR = path.join(TASKS_DIR, 'node_modules');           // transient: npm stage installs here, then we prune
const PACKAGE_JSON = path.join(TASKS_DIR, 'package.json');
const PACKAGE_LOCK = path.join(TASKS_DIR, 'package-lock.json');
const MANIFEST_FILE = path.join(TASKS_DIR, '.install-manifest.json');    // exhaustive record of what install did — read by /tasks-remove

// Makira (the SHAUGHV body face) is a COMMERCIAL license — never bundled/mirrored. It loads
// from the CDN when reachable and otherwise falls back to the system font stack; the board's
// Slot Roll + FLIP motion is glyph-agnostic, so behaviour is identical either way.
const MAKIRA_NOTE = 'Makira (the SHAUGHV body typeface) is under a commercial license and is never bundled or mirrored — it loads from the CDN when reachable and otherwise falls back to the system font stack. Behaviour is identical regardless.';

// Every vendored asset, keyed by its path relative to vendor/. Each declares the sources it
// can come from, in tier order: `npm` (full) → `cdn` (vendor) → shipped plugin copy (shipped).
// `sha256` is verified on every fetched/copied byte (integrity + version-pin); a `null`/absent
// sha means "presence is enough" (generated text like fonts.css). The pins are byte-exact to the
// files shipped under skills/tasks-start/assets/vendor/ and to the CDN/npm artefacts they mirror.
const PINNED = {
  'anime.min.js': {
    sha256: 'b5ce1be3c3f530f192e0f2571d1942846096d66119cbada34bfdc912c4873f35',
    npm: { pkg: 'animejs', version: '3.2.2', file: path.join('animejs', 'lib', 'anime.min.js') },
    cdn: 'https://cdn.jsdelivr.net/npm/animejs@3.2.2/lib/anime.min.js',
  },
  'animated-brand-mark.js': {
    sha256: 'f56628c010793a65e618f05b142fd54a7b66b6217c999fe88a2302e160755eb6',
    cdn: 'https://cdn.shaughv.com/js/animated-brand-mark.js',
  },
  'fonts/ibm-plex-mono/IBMPlexMono-Regular.woff2': {
    sha256: '0af5656d2fffe95cd621959a684dcfe69e14d851b79b5980340bd012fb075c79',
    cdn: 'https://cdn.shaughv.com/fonts/ibm-plex-mono/woff2/IBMPlexMono-Regular.woff2',
  },
  'fonts/ibm-plex-mono/IBMPlexMono-Medium.woff2': {
    sha256: 'ad59ae21754cc7405f7e73838c8e21f253d96191ea7f7b6297a88b2086b037f1',
    cdn: 'https://cdn.shaughv.com/fonts/ibm-plex-mono/woff2/IBMPlexMono-Medium.woff2',
  },
  'fonts/ibm-plex-mono/IBMPlexMono-SemiBold.woff2': {
    sha256: 'a7cc7bc1d6e178820edf6374e84edc10271ccca981961ab49ae6a47fc761e8e5',
    cdn: 'https://cdn.shaughv.com/fonts/ibm-plex-mono/woff2/IBMPlexMono-SemiBold.woff2',
  },
  'fonts/unbounded/Unbounded-Regular.woff2': {
    sha256: '0b07919a70db342cbeaf0e8f6d788600e597f44541c9ad7ea8715a1c75e89d00',
    cdn: 'https://cdn.shaughv.com/fonts/unbounded/woff2/Unbounded-Regular.woff2',
  },
  'fonts/unbounded/Unbounded-Bold.woff2': {
    sha256: '160dc6b33e738a7480c13f5f06a549c560ff2dd2a4eebc48639d650ba5c05fb9',
    cdn: 'https://cdn.shaughv.com/fonts/unbounded/woff2/Unbounded-Bold.woff2',
  },
  // Generated stylesheet — no remote source, no sha pin (its bytes reference the local /vendor
  // URLs above with a CDN fallback). Always copied from the shipped plugin bundle when present.
  'fonts.css': { shippedOnly: true },
};

const STATIC_MIME = {
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.woff2': 'font/woff2',
  '.woff': 'font/woff',
  '.svg': 'image/svg+xml',
  '.json': 'application/json; charset=utf-8',
  '.map': 'application/json; charset=utf-8',
};

// ---------------------------------------------------------------------------
// small helpers
// ---------------------------------------------------------------------------

function readJsonSafe(file) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return null; }
}

function writeJsonSafe(file, obj) {
  try { fs.writeFileSync(file, JSON.stringify(obj, null, 2)); } catch { /* ignore */ }
}

async function fileMtimeMs(file) {
  try { return Math.floor((await fsp.stat(file)).mtimeMs); } catch { return 0; }
}

function pidAlive(pid) {
  if (!pid) return false;
  try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; }
}

function portFree(port) {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once('error', () => resolve(false));
    srv.once('listening', () => srv.close(() => resolve(true)));
    srv.listen(port, '127.0.0.1');
  });
}

async function findFreePort(start) {
  for (let p = start; p < start + 50; p++) {
    if (await portFree(p)) return p;
  }
  return start; // give up gracefully; bind will surface the error
}

// The board's identity: which .tasks/ folder this server serves. Ports are shared machine-wide —
// after a restart, another repo's board may hold the port our state file remembers — so liveness
// alone is NEVER identity (see the tasks-boards skill). Realpath + case normalization keeps the
// comparison stable across symlinks and Windows casing.
function boardRoot() {
  try { return fs.realpathSync(TASKS_DIR); } catch { return TASKS_DIR; }
}
function sameRoot(a, b) {
  if (!a || !b) return false;
  const norm = (p) => (process.platform === 'win32' ? String(p).toLowerCase() : String(p));
  return norm(a) === norm(b);
}

// Resolve "is OUR server already up?" — resolves with the state only when the responder on the
// recorded port is a shaughv board serving THIS .tasks/ folder. A matching token with a different
// root means another repo's board holds the port (our state file is stale): that is NOT ours, and
// callers start fresh on a free port. Legacy (pre-0.2.0) servers answer a bare token with no root;
// accept those only when the state file's pid is still alive (this repo's own older server).
function probeRunning() {
  const state = readJsonSafe(STATE_FILE);
  if (!state || !state.port) return Promise.resolve(null);
  return new Promise((resolve) => {
    const req = http.get(
      { host: '127.0.0.1', port: state.port, path: '/api/ping', timeout: 800 },
      (res) => {
        let body = '';
        res.on('data', (c) => (body += c));
        res.on('end', () => {
          const text = body.trim();
          if (text === PING_TOKEN) return resolve(pidAlive(state.pid) ? state : null); // legacy
          try {
            const info = JSON.parse(text);
            if (info && info.ok === PING_TOKEN && sameRoot(info.root, boardRoot())) {
              return resolve({ ...state, root: info.root });
            }
          } catch { /* not a board */ }
          resolve(null); // foreign board or non-board on our remembered port
        });
      }
    );
    req.on('error', () => resolve(null));
    req.on('timeout', () => { req.destroy(); resolve(null); });
  });
}

function openInBrowser(url) {
  try {
    const platform = process.platform;
    if (platform === 'win32') {
      // `start` is a cmd builtin; the empty "" is the window title arg.
      spawn('cmd', ['/c', 'start', '', url], { detached: true, stdio: 'ignore' }).unref();
    } else if (platform === 'darwin') {
      spawn('open', [url], { detached: true, stdio: 'ignore' }).unref();
    } else {
      spawn('xdg-open', [url], { detached: true, stdio: 'ignore' }).unref();
    }
    return true;
  } catch {
    return false;
  }
}

// Path guard for the memory API: only allow CLAUDE.md or *.md files under memory/.
// Rejects traversal (`../`), absolute / Windows-drive paths, null bytes, non-.md files,
// and symlinks whose real parent escapes the .tasks/ tree.
function resolveMemoryPath(relPath) {
  if (!relPath || relPath.includes('\0')) return null;
  const resolved = path.resolve(TASKS_DIR, relPath);
  if (!resolved.toLowerCase().endsWith('.md')) return null;
  const isClaude = resolved === CLAUDE_MD;
  const inMemory = resolved === MEMORY_DIR || resolved.startsWith(MEMORY_DIR + path.sep);
  if (!isClaude && !inMemory) return null;
  try {
    const realRoot = fs.realpathSync(TASKS_DIR);
    const realParent = fs.realpathSync(path.dirname(resolved)); // throws if parent missing — fine on first write
    if (realParent !== realRoot && !realParent.startsWith(realRoot + path.sep)) return null;
  } catch { /* parent doesn't exist yet — path is already lexically constrained above */ }
  return resolved;
}

// ---------------------------------------------------------------------------
// serve
// ---------------------------------------------------------------------------

async function serve({ open = false, port: requested } = {}) {
  const port = await findFreePort(requested || DEFAULT_PORT);
  const sseClients = new Set();
  let lastSelfWrite = 0; // suppress echo of our own writes

  function broadcast(kind) {
    const payload = `event: change\ndata: ${JSON.stringify({ kind, at: Date.now() })}\n\n`;
    for (const res of sseClients) { try { res.write(payload); } catch { /* dropped */ } }
  }

  function send(res, status, type, body, extra = {}) {
    res.writeHead(status, { 'Content-Type': type, 'Cache-Control': 'no-store', ...extra });
    res.end(body);
  }

  async function readBody(req) {
    const chunks = [];
    for await (const c of req) chunks.push(c);
    return Buffer.concat(chunks).toString('utf8');
  }

  async function listMemory() {
    const out = { claudeMd: fs.existsSync(CLAUDE_MD), files: [], dirs: {} };
    if (fs.existsSync(MEMORY_DIR)) {
      for (const entry of await fsp.readdir(MEMORY_DIR, { withFileTypes: true })) {
        if (entry.isFile() && entry.name.endsWith('.md')) {
          out.files.push(entry.name);
        } else if (entry.isDirectory()) {
          const sub = (await fsp.readdir(path.join(MEMORY_DIR, entry.name)))
            .filter((n) => n.endsWith('.md'));
          out.dirs[entry.name] = sub;
        }
      }
    }
    return out;
  }

  const server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url, `http://127.0.0.1:${port}`);
      const pathname = url.pathname;

      if (pathname === '/api/ping') {
        // Identity, not just liveness: multiple boards can share one machine, and a port can be
        // inherited by another repo's board. Clients must match `root` before trusting this server.
        return send(res, 200, 'application/json', JSON.stringify({
          ok: PING_TOKEN, root: boardRoot(), pid: process.pid, port,
        }));
      }

      if (pathname === '/' || pathname === '/index.html' || pathname === '/dashboard.html') {
        const html = await fsp.readFile(DASHBOARD, 'utf8').catch(() => '<h1>dashboard.html missing</h1>');
        return send(res, 200, 'text/html; charset=utf-8', html);
      }

      if (pathname === '/api/tasks') {
        if (req.method === 'GET') {
          const md = await fsp.readFile(TASKS_MD, 'utf8').catch(() => '# Tasks\n');
          const mtime = await fileMtimeMs(TASKS_MD);
          return send(res, 200, 'text/markdown; charset=utf-8', md, { 'X-Board-Mtime': String(mtime) });
        }
        if (req.method === 'POST') {
          const body = await readBody(req);
          // Optimistic concurrency: when the client tells us which version it edited
          // (X-Base-Mtime), reject with 409 if the file changed underneath it — so an
          // agent's write is never silently stomped by a stale browser save. The 409
          // body carries the latest content + mtime so the client can reconcile.
          const base = req.headers['x-base-mtime'];
          if (base !== undefined && base !== '') {
            const current = await fileMtimeMs(TASKS_MD);
            if (String(current) !== String(base)) {
              const latest = await fsp.readFile(TASKS_MD, 'utf8').catch(() => '# Tasks\n');
              return send(res, 409, 'text/markdown; charset=utf-8', latest, { 'X-Board-Mtime': String(current) });
            }
          }
          const tmp = TASKS_MD + '.tmp';
          await fsp.writeFile(tmp, body, 'utf8');
          await fsp.rename(tmp, TASKS_MD);
          lastSelfWrite = Date.now();
          const mtime = await fileMtimeMs(TASKS_MD);
          return send(res, 200, 'application/json', JSON.stringify({ ok: true, mtime }), { 'X-Board-Mtime': String(mtime) });
        }
      }

      // MILESTONES.md — same byte-pipe + optimistic-concurrency semantics as /api/tasks.
      // GET serves the skeleton when the file doesn't exist yet; it's only created on first write.
      if (pathname === '/api/milestones') {
        if (req.method === 'GET') {
          const md = await fsp.readFile(MILESTONES_MD, 'utf8').catch(() => '# Milestones\n');
          const mtime = await fileMtimeMs(MILESTONES_MD);
          return send(res, 200, 'text/markdown; charset=utf-8', md, { 'X-Board-Mtime': String(mtime) });
        }
        if (req.method === 'POST') {
          const body = await readBody(req);
          const base = req.headers['x-base-mtime'];
          if (base !== undefined && base !== '') {
            const current = await fileMtimeMs(MILESTONES_MD);
            if (String(current) !== String(base)) {
              const latest = await fsp.readFile(MILESTONES_MD, 'utf8').catch(() => '# Milestones\n');
              return send(res, 409, 'text/markdown; charset=utf-8', latest, { 'X-Board-Mtime': String(current) });
            }
          }
          const tmp = MILESTONES_MD + '.tmp';
          await fsp.writeFile(tmp, body, 'utf8');
          await fsp.rename(tmp, MILESTONES_MD);
          lastSelfWrite = Date.now();
          const mtime = await fileMtimeMs(MILESTONES_MD);
          return send(res, 200, 'application/json', JSON.stringify({ ok: true, mtime }), { 'X-Board-Mtime': String(mtime) });
        }
      }

      // Setup-owned config (git tracking, hooks target). Served verbatim, GET-only — the server
      // never parses, acts on, or writes it; /tasks-start owns it. secure/ has no route at all.
      if (pathname === '/api/config' && req.method === 'GET') {
        const body = await fsp.readFile(CONFIG_FILE, 'utf8').catch(() => '{}');
        return send(res, 200, 'application/json', body);
      }

      if (pathname === '/api/memory/tree' && req.method === 'GET') {
        return send(res, 200, 'application/json', JSON.stringify(await listMemory()));
      }

      if (pathname === '/api/memory/file') {
        const rel = url.searchParams.get('path') || '';
        const target = resolveMemoryPath(rel);
        if (!target) return send(res, 400, 'application/json', JSON.stringify({ error: 'bad path' }));
        if (req.method === 'GET') {
          const content = await fsp.readFile(target, 'utf8').catch(() => '');
          return send(res, 200, 'text/markdown; charset=utf-8', content);
        }
        if (req.method === 'POST') {
          const body = await readBody(req);
          await fsp.mkdir(path.dirname(target), { recursive: true });
          await fsp.writeFile(target, body, 'utf8');
          lastSelfWrite = Date.now();
          return send(res, 200, 'application/json', JSON.stringify({ ok: true }));
        }
      }

      // Per-task detail file (rich description + activity log): .tasks/tasks/<id>.md
      if (pathname === '/api/task') {
        const id = (url.searchParams.get('id') || '').toLowerCase();
        if (!/^[0-9a-z]{2,8}$/.test(id)) return send(res, 400, 'application/json', JSON.stringify({ error: 'bad id' }));
        const target = path.join(TASK_DETAIL_DIR, id + '.md');
        if (req.method === 'GET') {
          const content = await fsp.readFile(target, 'utf8').catch(() => '');
          return send(res, 200, 'text/markdown; charset=utf-8', content);
        }
        if (req.method === 'POST') {
          const body = await readBody(req);
          await fsp.mkdir(TASK_DETAIL_DIR, { recursive: true });
          const tmp = target + '.tmp';
          await fsp.writeFile(tmp, body, 'utf8');
          await fsp.rename(tmp, target);
          lastSelfWrite = Date.now();
          return send(res, 200, 'application/json', JSON.stringify({ ok: true }));
        }
        if (req.method === 'DELETE') {
          // Called when a task is deleted, so its detail file can't outlive it (and a
          // future task that happens to reuse the id never inherits stale content).
          await fsp.unlink(target).catch(() => {});
          lastSelfWrite = Date.now();
          return send(res, 200, 'application/json', JSON.stringify({ ok: true }));
        }
      }

      // Per-milestone detail file (rich description + Completed archive + activity log):
      // .tasks/milestones/<id>.md — same semantics as /api/task. The id regex + fixed directory
      // confine both detail routes inside .tasks/; secure/ is structurally unreachable from here.
      if (pathname === '/api/milestone') {
        const id = (url.searchParams.get('id') || '').toLowerCase();
        if (!/^[0-9a-z]{2,8}$/.test(id)) return send(res, 400, 'application/json', JSON.stringify({ error: 'bad id' }));
        const target = path.join(MILESTONE_DETAIL_DIR, id + '.md');
        if (req.method === 'GET') {
          const content = await fsp.readFile(target, 'utf8').catch(() => '');
          return send(res, 200, 'text/markdown; charset=utf-8', content);
        }
        if (req.method === 'POST') {
          const body = await readBody(req);
          await fsp.mkdir(MILESTONE_DETAIL_DIR, { recursive: true });
          const tmp = target + '.tmp';
          await fsp.writeFile(tmp, body, 'utf8');
          await fsp.rename(tmp, target);
          lastSelfWrite = Date.now();
          return send(res, 200, 'application/json', JSON.stringify({ ok: true }));
        }
        if (req.method === 'DELETE') {
          await fsp.unlink(target).catch(() => {});
          lastSelfWrite = Date.now();
          return send(res, 200, 'application/json', JSON.stringify({ ok: true }));
        }
      }

      if (pathname === '/api/events' && req.method === 'GET') {
        res.writeHead(200, {
          'Content-Type': 'text/event-stream',
          'Cache-Control': 'no-cache',
          Connection: 'keep-alive',
        });
        res.write('retry: 2000\n\n');
        sseClients.add(res);
        const keepalive = setInterval(() => { try { res.write(': ping\n\n'); } catch { /* */ } }, 25_000);
        req.on('close', () => { clearInterval(keepalive); sseClients.delete(res); });
        return;
      }

      // Vendored assets (anime.js / fonts / brandmark / fonts.css) provisioned by `install`,
      // served from .tasks/vendor/. Same confinement as the memory API: resolve under the root
      // and reject anything that escapes it (traversal, NUL, absolute/drive paths). Missing files
      // 404 so the dashboard's runtime loader degrades to its CDN/inline fallback automatically.
      if ((req.method === 'GET' || req.method === 'HEAD') && pathname.startsWith('/vendor/')) {
        const rel = decodeURIComponent(pathname.slice('/vendor/'.length));
        if (!rel || rel.includes('\0')) return send(res, 400, 'text/plain', 'bad path');
        const resolved = path.resolve(VENDOR_DIR, rel);
        if (resolved !== VENDOR_DIR && !resolved.startsWith(VENDOR_DIR + path.sep)) {
          return send(res, 403, 'text/plain', 'forbidden');
        }
        let buf;
        try {
          const st = await fsp.stat(resolved);
          if (!st.isFile()) return send(res, 404, 'text/plain', 'not found');
          buf = await fsp.readFile(resolved);
        } catch { return send(res, 404, 'text/plain', 'not found'); }
        const type = STATIC_MIME[path.extname(resolved).toLowerCase()] || 'application/octet-stream';
        res.writeHead(200, { 'Content-Type': type, 'Cache-Control': 'public, max-age=3600' });
        return res.end(req.method === 'HEAD' ? undefined : buf); // Buffer — binary-safe
      }

      return send(res, 404, 'text/plain', 'not found');
    } catch (err) {
      try { send(res, 500, 'text/plain', String(err && err.message || err)); } catch { /* */ }
    }
  });

  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', resolve);
  });

  writeJsonSafe(STATE_FILE, { port, pid: process.pid, startedAt: new Date().toISOString(), root: boardRoot() });

  // Watch for external edits (e.g. an agent editing TASKS.md) and push to the browser.
  // Debounced; suppress the echo of our own POST writes.
  let debounce = null;
  const onChange = (kind) => {
    if (Date.now() - lastSelfWrite < 800) return; // our own write — browser already has it
    clearTimeout(debounce);
    debounce = setTimeout(() => broadcast(kind), 150);
  };
  try { fs.watchFile(TASKS_MD, { interval: 600 }, () => onChange('tasks')); } catch { /* */ }
  try { fs.watchFile(MILESTONES_MD, { interval: 600 }, () => onChange('milestones')); } catch { /* */ }
  try { fs.watch(TASKS_DIR, { recursive: true }, (_e, name) => {
    if (!name) return onChange('memory');
    const n = String(name);
    // Ignore our own state files, anything the installer touches (vendor assets, the transient
    // npm scaffolding, the manifest, tmp files), the scoped .gitignore, and EVERYTHING under
    // secure/ — secrets must never surface over SSE, not even as write timing.
    if (n.startsWith('.board-') || n.startsWith('.install-manifest') ||
        n === 'package.json' || n === 'package-lock.json' || n.endsWith('.tmp') ||
        n === '.gitignore' ||
        n === SECURE_DIR_NAME || n.startsWith(SECURE_DIR_NAME + path.sep) ||
        n === 'vendor' || n.startsWith('vendor' + path.sep) ||
        n === 'node_modules' || n.startsWith('node_modules' + path.sep)) return;
    if (n === 'TASKS.md') return onChange('tasks');
    if (n === 'MILESTONES.md' || n === 'milestones' || n.startsWith('milestones' + path.sep)) return onChange('milestones');
    if (n === 'tasks' || n.startsWith('tasks' + path.sep)) return onChange('detail');
    if (n === 'config.json') return onChange('config');
    onChange('memory');
  }); } catch { /* recursive watch unsupported on some Linux — watchFile above still covers TASKS.md + MILESTONES.md */ }

  const url = `http://127.0.0.1:${port}/`;
  process.stdout.write(`SHAUGHV task board live at ${url}\n`);
  if (open) openInBrowser(url);

  const shutdown = () => { try { fs.unlinkSync(STATE_FILE); } catch { /* */ } process.exit(0); };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
  return url;
}

// ---------------------------------------------------------------------------
// ensure — start a detached server if one isn't already running
// ---------------------------------------------------------------------------

async function ensure({ open = false } = {}) {
  const running = await probeRunning();
  if (running) {
    const url = `http://127.0.0.1:${running.port}/`;
    if (open) openInBrowser(url);
    return url;
  }
  const port = await findFreePort(DEFAULT_PORT);
  let out = 'ignore', err = 'ignore';
  try { const fd = fs.openSync(LOG_FILE, 'a'); out = fd; err = fd; } catch { /* */ }
  const child = spawn(process.execPath, [fileURLToPath(import.meta.url), 'serve', '--port', String(port)], {
    detached: true,
    stdio: ['ignore', out, err],
  });
  child.unref();
  // Wait briefly for it to come up so callers learn the real port and can open it.
  const url = `http://127.0.0.1:${port}/`;
  for (let i = 0; i < 30; i++) {
    const r = await probeRunning();
    if (r) { if (open) openInBrowser(`http://127.0.0.1:${r.port}/`); return `http://127.0.0.1:${r.port}/`; }
    await new Promise((r) => setTimeout(r, 100));
  }
  if (open) openInBrowser(url);
  return url;
}

// ---------------------------------------------------------------------------
// hook — board-maintenance nudges (see references/board-server.md for the wiring)
// ---------------------------------------------------------------------------

// Returns { key, text } for an event (key is the SEMANTIC nudge type used for
// per-type cooldown — so a `git commit` nudge never suppresses a later `git push`),
// or null when this event shouldn't nudge (e.g. a Bash command that isn't commit/push).
function reminderFor(event, hookInput, url) {
  const tail = ` — the live board at ${url} (.tasks/TASKS.md).`;
  switch (event) {
    case 'SessionStart':
      return { key: 'session', text: `[task board] This repo uses a live SHAUGHV task board${tail} Keep it current so the operator has full visibility: as you start, finish, or discover work — and around commits, pushes, and subagents — update .tasks/TASKS.md (move items between sections, check off completed work, add new ones). Use proper subtasks for small board-visible steps: indented checkbox rows under the parent task, with optional indented detail lines; do not bury those steps as plain text in the parent description or call them sub-items. The board auto-syncs; you just edit the file. For each task, keep a rich, self-contained description in .tasks/tasks/<id>.md (lead with a plain-English TT;DR, then exhaustive context — goal, plan, files, decisions, what's done vs. left) so ANY agent that picks the task up later, at any stage, has everything it needs to continue. Log meaningful changes under its ## Activity section.` };
    case 'PostToolUse': {
      const cmd = (hookInput?.tool_input?.command || '').toString();
      if (/\bgit\s+push\b/.test(cmd)) return { key: 'push', text: `[task board] You just pushed. Make sure .tasks/TASKS.md reflects what landed — check off completed items and add any follow-ups — so the operator's board stays accurate.` };
      if (/\bgit\s+commit\b/.test(cmd)) return { key: 'commit', text: `[task board] You just committed. Update .tasks/TASKS.md to match (mark finished work done, add newly-surfaced tasks) so the operator can see progress.` };
      return null;
    }
    case 'ExitPlanMode':
      return { key: 'plan', text: `[task board] A plan was just approved. Mirror its steps into .tasks/TASKS.md as Active items so the operator can track execution against the board.` };
    case 'SubagentStart':
      return { key: 'subagent-start', text: `[task board] A subagent is starting. If it changes the plan or completes work, make sure .tasks/TASKS.md reflects it so the operator keeps visibility.` };
    case 'SubagentStop':
      return { key: 'subagent-stop', text: `[task board] A subagent just finished. Reflect any completed or newly-discovered work in .tasks/TASKS.md.` };
    default:
      return { key: 'generic', text: `[task board] Keep .tasks/TASKS.md current so the operator has full visibility.` };
  }
}

async function readStdin() {
  if (process.stdin.isTTY) return null;
  try {
    const chunks = [];
    const timer = setTimeout(() => process.stdin.destroy(), 500);
    for await (const c of process.stdin) chunks.push(c);
    clearTimeout(timer);
    const txt = Buffer.concat(chunks).toString('utf8').trim();
    return txt ? JSON.parse(txt) : null;
  } catch { return null; }
}

function nudgeAllowed(key) {
  // Session start always nudges; other nudge types are cooled down PER TYPE so a
  // commit nudge can't swallow a later push nudge, and a subagent fan-out can't spam.
  if (key === 'session') return true;
  const state = readJsonSafe(NUDGE_FILE) || {};
  const last = state[key] || 0;
  if (Date.now() - last < NUDGE_COOLDOWN_MS) return false;
  state[key] = Date.now();
  writeJsonSafe(NUDGE_FILE, state);
  return true;
}

async function hook(event) {
  // Gate: only act when this is really a task-board repo (defensive — the relative
  // path in the hook command already scopes us, but a subagent could run elsewhere).
  if (!fs.existsSync(DASHBOARD)) { process.exit(0); }

  const input = await readStdin();

  // A PostToolUse on the ExitPlanMode tool is the "plan approved" moment.
  let ev = event;
  if (event === 'PostToolUse' && input?.tool_name === 'ExitPlanMode') ev = 'ExitPlanMode';

  // Decide the reminder FIRST (using a placeholder URL) — a non-commit/push Bash
  // command returns null and we exit without even touching the server or cooldown.
  const placeholder = `http://127.0.0.1:${DEFAULT_PORT}/`;
  const reminder = reminderFor(ev, input, placeholder);
  if (!reminder) process.exit(0);

  // We have a nudge: make sure the live board is up (silently — hooks never --open),
  // then point the reminder at the real URL (the port may differ from the default).
  let url = placeholder;
  try { url = await ensure({ open: false }); } catch { /* board optional; still nudge */ }
  const text = reminder.text.split(placeholder).join(url);

  if (!nudgeAllowed(reminder.key)) process.exit(0);

  if (event === 'SessionStart') {
    // SessionStart: plain stdout is injected as context.
    process.stdout.write(text + '\n');
  } else {
    // PostToolUse / SubagentStart / SubagentStop: agent-visible additionalContext.
    process.stdout.write(JSON.stringify({
      hookSpecificOutput: { hookEventName: event, additionalContext: text },
    }));
  }
  process.exit(0);
}

// ---------------------------------------------------------------------------
// stop / status
// ---------------------------------------------------------------------------

function stop() {
  const state = readJsonSafe(STATE_FILE);
  if (state?.pid && pidAlive(state.pid)) {
    try { process.kill(state.pid); } catch { /* */ }
  }
  try { fs.unlinkSync(STATE_FILE); } catch { /* */ }
  try { fs.unlinkSync(NUDGE_FILE); } catch { /* */ }
  process.stdout.write('stopped\n');
}

async function status() {
  const running = await probeRunning();
  process.stdout.write(JSON.stringify(running ? { ...running, root: running.root || boardRoot() } : { running: false }) + '\n');
}

// ---------------------------------------------------------------------------
// install — provision the tiered dependency assets (NOT user-invocable; run by
// /tasks-start after copying assets). Try-everything chain, first success wins,
// offline floor cannot fail. Writes an exhaustive .install-manifest.json so
// /tasks-remove can do a complete, residue-free uninstall. See references/board-server.md.
// ---------------------------------------------------------------------------

const TIER_RANK = { offline: 0, shipped: 1, vendor: 2, full: 3 };
const SOURCE_TIER = { shipped: 'shipped', cdn: 'vendor', npm: 'full' }; // achieved-tier name for a source

function sha256Buf(buf) { return crypto.createHash('sha256').update(buf).digest('hex'); }
function sha256File(file) { try { return sha256Buf(fs.readFileSync(file)); } catch { return null; } }
function statSize(file) { try { return fs.statSync(file).size; } catch { return null; } }
function relTasks(abs) { return path.relative(TASKS_DIR, abs).split(path.sep).join('/'); }

// A vendored file is "already good" when it exists and (for sha-pinned assets) matches the pin.
function alreadyGood(dest, sha) {
  if (!fs.existsSync(dest)) return false;
  if (!sha) return true;            // presence-only asset (fonts.css)
  return sha256File(dest) === sha;
}

function writeFileAtomic(dest, buf) {
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  const tmp = dest + '.tmp-' + process.pid;
  fs.writeFileSync(tmp, buf);
  fs.renameSync(tmp, dest);
}

function readPluginVersion() {
  const local = readJsonSafe(path.join(TASKS_DIR, '.board-version.json'));
  if (local?.pluginVersion) return local.pluginVersion;
  if (process.env.SHAUGHV_TASKS_PLUGIN_VERSION) return process.env.SHAUGHV_TASKS_PLUGIN_VERSION;
  const sourceMarker = process.env.SHAUGHV_TASKS_ASSETS_DIR
    ? readJsonSafe(path.join(process.env.SHAUGHV_TASKS_ASSETS_DIR, 'board-version.json'))
    : null;
  if (sourceMarker?.pluginVersion) return sourceMarker.pluginVersion;
  for (const root of [process.env.CLAUDE_PLUGIN_ROOT, process.env.CODEX_PLUGIN_ROOT].filter(Boolean)) {
    for (const rel of [path.join('.claude-plugin', 'plugin.json'), path.join('.codex-plugin', 'plugin.json')]) {
      const j = readJsonSafe(path.join(root, rel));
      if (j?.version) return j.version;
    }
  }
  return 'unknown';
}

function shippedAssetsDir() {
  if (process.env.SHAUGHV_TASKS_ASSETS_DIR) return path.resolve(process.env.SHAUGHV_TASKS_ASSETS_DIR);
  for (const root of [process.env.CLAUDE_PLUGIN_ROOT, process.env.CODEX_PLUGIN_ROOT].filter(Boolean)) {
    const candidate = path.join(root, 'skills', 'tasks-start', 'assets');
    if (fs.existsSync(candidate)) return candidate;
  }
  return null;
}

function writeManifest(m) {
  m.updatedAt = new Date().toISOString();
  const tmp = MANIFEST_FILE + '.tmp';
  try { fs.writeFileSync(tmp, JSON.stringify(m, null, 2)); fs.renameSync(tmp, MANIFEST_FILE); } catch { /* best effort */ }
}

function trackCreatedDir(m, abs) {
  const r = relTasks(abs);
  if (r && !r.startsWith('..') && !m.created.dirs.includes(r)) m.created.dirs.push(r);
}
function trackCreatedFile(m, abs, sha, bytes, source) {
  const r = relTasks(abs);
  m.created.files = m.created.files.filter((f) => f.path !== r);
  m.created.files.push({ path: r, sha256: sha || null, bytes: bytes ?? null, source });
  trackCreatedDir(m, path.dirname(abs));
}

// HTTPS GET → Buffer, following redirects (CDNs/jsdelivr redirect). Rejects on non-200/timeout.
function httpsGet(url, redirects = 5) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { timeout: 15_000, headers: { 'user-agent': 'shaughv-task-board-installer' } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location && redirects > 0) {
        res.resume();
        try { return resolve(httpsGet(new URL(res.headers.location, url).toString(), redirects - 1)); }
        catch (e) { return reject(e); }
      }
      if (res.statusCode !== 200) { res.resume(); return reject(new Error('HTTP ' + res.statusCode)); }
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve(Buffer.concat(chunks)));
    });
    req.on('error', reject);
    req.on('timeout', () => req.destroy(new Error('timeout')));
  });
}

// Run a child process to completion; never throws — returns {code,out,err}. Negative codes:
// -1 spawn/error, -2 timeout-killed. Used for real executables (winget/sudo/brew/apt/node/where)
// with shell:false — no arg concatenation, no injection surface.
function run(cmd, args, opts = {}) {
  return new Promise((resolve) => {
    let out = '', err = '', done = false, timer = null;
    const finish = (code) => { if (done) return; done = true; if (timer) clearTimeout(timer); resolve({ code, out, err }); };
    let child;
    try {
      child = spawn(cmd, args, { windowsHide: true, ...opts });
    } catch (e) { return resolve({ code: -1, out: '', err: String((e && e.message) || e) }); }
    if (opts.timeout) timer = setTimeout(() => { try { child.kill(); } catch { /* */ } finish(-2); }, opts.timeout);
    if (child.stdout) child.stdout.on('data', (d) => { out += d; });
    if (child.stderr) child.stderr.on('data', (d) => { err += d; });
    child.on('error', (e) => { err += String((e && e.message) || e); finish(-1); });
    child.on('close', (code) => finish(code));
  });
}

// Run a fixed command LINE through the platform shell. Needed only for npm, which is a `.cmd`
// shim on Windows that Node won't spawn without a shell. Passing one pre-built string (rather than
// an args array with shell:true) sidesteps DEP0190; every interpolated value here is a hard-coded
// literal or an absolute path we control, so there is no injection surface.
function runLine(line, opts = {}) {
  return run(line, [], { shell: true, ...opts });
}

async function which(bin) {
  const r = await run(process.platform === 'win32' ? 'where' : 'which', [bin]);
  return r.code === 0;
}
async function probeNpm() {
  const r = await runLine('npm --version');
  return r.code === 0;
}
async function nodeVersionString() {
  const r = await run('node', ['--version']);
  return ((r.out || '').trim()) || process.version;
}

// npm stage: install the pinned package into the TRANSIENT .tasks/node_modules, with scripts
// disabled and nothing written to package.json/lock. Returns true on a clean exit.
async function npmInstall(npmSpec) {
  const r = await runLine(
    `npm install --prefix "${TASKS_DIR}" --no-audit --no-fund --ignore-scripts --no-save --no-package-lock ${npmSpec.pkg}@${npmSpec.version}`,
    { cwd: TASKS_DIR, timeout: 120_000 });
  return r.code === 0;
}

// Remove the transient npm scaffolding — node_modules is only ever a staging area: once the
// verified artefact is copied into vendor/, nothing under .tasks/ should reference npm. This is
// what keeps uninstall trivial (only vendor/ + manifest live under .tasks/, both wiped wholesale).
async function pruneNpmScaffolding() {
  for (const p of [NODE_MODULES_DIR, PACKAGE_JSON, PACKAGE_LOCK, path.join(TASKS_DIR, '.package-lock.json')]) {
    try { await fsp.rm(p, { recursive: true, force: true }); } catch { /* */ }
  }
}

// Best-effort global Node bootstrap so the full (npm) tier becomes reachable when npm is absent.
// Per-OS, non-interactive, gated on the manager existing; any non-zero/UAC/timeout is treated as
// failure → we fall through to a lower tier (the board still works). EVERY attempt — success or
// not — is recorded in manifest.global so /tasks-remove can offer to reverse it. Dormant in the
// common case (node/npm already present), since install runs *under* node to begin with.
async function bootstrapNodeGlobally(manifest) {
  const plat = process.platform;
  let attempt = null;
  if (plat === 'win32') {
    if (!(await which('winget'))) return false;
    attempt = { manager: 'winget', id: 'OpenJS.NodeJS.LTS',
      cmd: 'winget', args: ['install', '--id', 'OpenJS.NodeJS.LTS', '-e', '--silent', '--accept-package-agreements', '--accept-source-agreements'],
      reverse: 'winget uninstall --id OpenJS.NodeJS.LTS -e' };
  } else if (plat === 'darwin') {
    if (!(await which('brew'))) return false;
    attempt = { manager: 'brew', id: 'node', cmd: 'brew', args: ['install', 'node'], reverse: 'brew uninstall node' };
  } else {
    const sudo = await run('sudo', ['-n', 'true']);
    if (sudo.code !== 0) return false; // no passwordless sudo, and nvm is a shell fn we can't spawn — give up cleanly
    attempt = { manager: 'apt', id: 'nodejs', cmd: 'sudo', args: ['-n', 'apt-get', 'install', '-y', 'nodejs', 'npm'], reverse: 'sudo apt-get remove -y nodejs npm' };
  }
  const r = await run(attempt.cmd, attempt.args, { timeout: 180_000 });
  const ok = r.code === 0;
  manifest.global.push({
    kind: 'node', manager: attempt.manager, id: attempt.id,
    version: ok ? await nodeVersionString() : null,
    wasPreexisting: false, attempted: true, succeeded: ok,
    reverseCommand: attempt.reverse, reverseRisk: 'high',
    note: ok
      ? `Installed Node globally via ${attempt.manager} so the task board could use the full (npm) dependency tier. Reverse ONLY if nothing else on this machine relies on this Node install.`
      : `Attempted to install Node globally via ${attempt.manager} (exit ${r.code}); it did not complete, so no global change persisted and the board fell back to a lower tier.`,
  });
  writeManifest(manifest);
  return ok;
}

// Record a global Node install that /tasks-start performed BEFORE it could run this script
// (node was wholly absent). Spec: "manager:id" or "manager:id:reverse command".
function recordNodeBootstrap(manifest, spec) {
  const parts = String(spec).split(':');
  const manager = parts[0] || 'unknown';
  const id = parts[1] || '';
  const reverse = parts.length > 2 ? parts.slice(2).join(':') : defaultNodeReverse(manager, id);
  manifest.global.push({
    kind: 'node', manager, id, version: process.version,
    wasPreexisting: false, attempted: true, succeeded: true, source: 'tasks-start',
    reverseCommand: reverse, reverseRisk: 'high',
    note: 'Node was installed globally by /tasks-start because it was absent and the board requires it. Reverse ONLY if nothing else on this machine relies on this Node install.',
  });
  writeManifest(manifest);
}
function defaultNodeReverse(manager, id) {
  if (manager === 'winget') return `winget uninstall --id ${id || 'OpenJS.NodeJS.LTS'} -e`;
  if (manager === 'brew') return `brew uninstall ${id || 'node'}`;
  if (manager === 'apt') return `sudo apt-get remove -y ${id || 'nodejs npm'}`;
  if (manager === 'nvm') return `nvm uninstall ${id || 'lts/*'}`;
  return `# manually uninstall the globally-installed Node (${manager} ${id})`;
}

// Fetch one asset's bytes from a given source. Returns a Buffer or null (try next source).
async function fetchAssetFrom(source, rel, spec) {
  if (source === 'cdn') {
    if (!spec.cdn) return null;
    return await httpsGet(spec.cdn);
  }
  if (source === 'shipped') {
    const assets = shippedAssetsDir();
    if (!assets) return null; // can't locate the plugin bundle without an explicit or host-provided path
    try { return await fsp.readFile(path.join(assets, 'vendor', ...rel.split('/'))); }
    catch { return null; }
  }
  if (source === 'npm') {
    if (!spec.npm) return null;
    const ok = await npmInstall(spec.npm);
    let buf = null;
    if (ok) { try { buf = await fsp.readFile(path.join(NODE_MODULES_DIR, spec.npm.file)); } catch { /* */ } }
    await pruneNpmScaffolding(); // node_modules is transient regardless of outcome
    return buf;
  }
  return null;
}

// Provision a single asset by walking its source chain (highest allowed tier first). sha256 is
// enforced on every candidate, so version drift / corruption / a MITM'd CDN is rejected and we
// fall through. Returns a manifest asset record.
async function provisionAsset(rel, spec, dest, ctx) {
  const { cap, npmUsable, offline, priorSource } = ctx;
  if (alreadyGood(dest, spec.sha256)) {
    const prev = priorSource(rel);
    const src = (prev && SOURCE_TIER[prev]) ? prev : 'shipped'; // best-known provenance, else local≈shipped
    return { path: rel, source: src, sha256: spec.sha256 || sha256File(dest), bytes: statSize(dest), ok: true, reused: true };
  }
  const candidates = [];
  if (spec.npm && cap >= TIER_RANK.full && npmUsable) candidates.push('npm');
  if (spec.cdn && cap >= TIER_RANK.vendor && !offline) candidates.push('cdn');
  if (cap >= TIER_RANK.shipped) candidates.push('shipped');
  for (const source of candidates) {
    try {
      const buf = await fetchAssetFrom(source, rel, spec);
      if (!buf || buf.length === 0) continue;
      if (spec.sha256 && sha256Buf(buf) !== spec.sha256) continue; // integrity / pin mismatch
      writeFileAtomic(dest, buf);
      return { path: rel, source, sha256: spec.sha256 || sha256Buf(buf), bytes: buf.length, ok: true };
    } catch { /* try next source */ }
  }
  return { path: rel, source: 'absent', ok: false };
}

async function install(opts = {}) {
  let requested = opts.tier && TIER_RANK[opts.tier] !== undefined ? opts.tier : 'full';
  // --offline means "no network, no npm" → cap at shipped (use the bundled copies, which are the
  // whole point of the offline tier). To force the true floor (provision nothing) use --tier offline.
  if (opts.offline && TIER_RANK[requested] > TIER_RANK.shipped) requested = 'shipped';
  const cap = TIER_RANK[requested];

  const prior = readJsonSafe(MANIFEST_FILE);
  const priorSource = (rel) => prior?.assets?.find((a) => a.path === rel)?.source;

  const manifest = {
    schemaVersion: 1,
    pluginVersion: readPluginVersion(),
    tool: 'board-server.mjs install',
    createdAt: prior?.createdAt || new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    status: 'in-progress',
    requestedTier: requested,
    tier: 'offline',
    options: { offline: !!opts.offline, noGlobal: !!opts.noGlobal },
    node: { version: process.version, execPath: process.execPath, platform: process.platform },
    assets: [],
    created: { dirs: [], files: [] },
    npm: [],                 // intentionally empty: node_modules is transient (see pruneNpmScaffolding)
    global: prior?.global?.filter((g) => g.succeeded) || [], // carry forward prior real global changes
    notes: [MAKIRA_NOTE],
  };
  writeManifest(manifest); // eager — a crash mid-install still leaves a valid, exhaustive record

  if (opts.nodeBootstrap) recordNodeBootstrap(manifest, opts.nodeBootstrap);

  if (cap >= TIER_RANK.shipped) { fs.mkdirSync(VENDOR_DIR, { recursive: true }); trackCreatedDir(manifest, VENDOR_DIR); }

  // Decide whether the full (npm) tier is usable; optionally bootstrap node to make it so.
  let npmUsable = false;
  if (cap >= TIER_RANK.full) {
    npmUsable = await probeNpm();
    if (!npmUsable && !opts.noGlobal && !opts.offline) {
      if (await bootstrapNodeGlobally(manifest)) npmUsable = await probeNpm();
    }
  }

  for (const [rel, spec] of Object.entries(PINNED)) {
    const dest = path.join(VENDOR_DIR, ...rel.split('/'));
    const rec = await provisionAsset(rel, spec, dest, { cap, npmUsable, offline: !!opts.offline, priorSource });
    manifest.assets.push(rec);
    if (rec.source !== 'absent') trackCreatedFile(manifest, dest, rec.sha256, rec.bytes, rec.source);
    writeManifest(manifest); // incremental — survives a crash on any single asset
  }

  // Achieved tier tracks anime.min.js (the marquee enhancement); fonts/brandmark degrade
  // independently and are recorded per-asset. No anime → offline floor.
  const anime = manifest.assets.find((a) => a.path === 'anime.min.js');
  manifest.tier = anime && anime.source !== 'absent' ? (SOURCE_TIER[anime.source] || 'shipped') : 'offline';
  manifest.status = 'complete'; // install never fails — the offline floor is a valid outcome
  writeManifest(manifest);

  const provisioned = manifest.assets.filter((a) => a.source !== 'absent').length;
  const summary = {
    ok: true, tier: manifest.tier, requestedTier: requested,
    provisioned, total: manifest.assets.length,
    assets: manifest.assets.map((a) => ({ path: a.path, source: a.source })),
    global: manifest.global.map((g) => ({ kind: g.kind, succeeded: g.succeeded })),
  };
  if (opts.json) {
    process.stdout.write(JSON.stringify(summary) + '\n');
  } else {
    const g = manifest.global.filter((x) => x.succeeded);
    process.stdout.write(`task board install: tier=${manifest.tier} (requested ${requested}); ${provisioned}/${manifest.assets.length} assets provisioned${g.length ? `; global changes recorded: ${g.map((x) => x.kind).join(', ')}` : ''}.\n`);
  }
  return summary;
}

// ---------------------------------------------------------------------------
// cli
// ---------------------------------------------------------------------------

const argv = process.argv.slice(2);
const cmd = argv[0] || 'serve';
const open = argv.includes('--open');
const flagVal = (name) => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : undefined; };
const portArg = argv.includes('--port') ? Number(flagVal('--port')) : undefined;

switch (cmd) {
  case 'serve':
    serve({ open, port: portArg }).catch((e) => { console.error(e); process.exit(1); });
    break;
  case 'ensure':
    ensure({ open }).then((u) => { process.stdout.write(u + '\n'); process.exit(0); })
      .catch((e) => { console.error(e); process.exit(1); });
    break;
  case 'install':
    // NOT user-invocable — run by /tasks-start after copying assets. Always exits 0 (the offline
    // floor is a valid outcome); a hard crash leaves the eagerly-written manifest as the record.
    install({
      tier: flagVal('--tier'),
      offline: argv.includes('--offline'),
      noGlobal: argv.includes('--no-global'),
      json: argv.includes('--json'),
      nodeBootstrap: flagVal('--node-bootstrap'),
    }).then(() => process.exit(0)).catch((e) => { console.error(e); process.exit(0); });
    break;
  case 'hook':
    hook(argv[1] || 'SessionStart');
    break;
  case 'stop':
    stop();
    break;
  case 'status':
    status();
    break;
  default:
    process.stderr.write(`unknown command: ${cmd}\n`);
    process.exit(2);
}
