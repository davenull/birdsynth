#!/usr/bin/env node
// The README's screenshots: birdsynth in headless Chrome, driven over the
// DevTools protocol at the faceplate's own size (1280×800) and 2× pixels.
// Each shot starts from a fresh profile, sets the synth up in the page (a
// preset, a page, held notes, a panel), waits for the displays to move, and
// captures the whole faceplate or the union of the parts it names.
//
//   node tools/screenshots.mjs [--url <site>] [shot …]
//
// The site defaults to the public one (`npm run preview` serves the local
// build on http://localhost:4173). Writes docs/screenshot.png and
// docs/screenshots/<shot>.png. CHROME overrides the browser's path.

import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const CHROME = process.env.CHROME ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const W = 1280;
const H = 800;

// every setup runs after this, in the page
const PRELUDE = `
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const s = window.__synth;
  const chord = (notes, v = 0.75) => notes.forEach((n) => s.noteOn(n, v));
`;

/**
 * name → where it goes, what to do first (JS in the page, may await), and
 * what to capture: CSS selectors (every match counts) whose bounding boxes
 * are joined, or none for the whole faceplate. `dpr` raises the pixel ratio
 * for small crops, `maxHeight` (CSS px) cuts a tall one short; `companion` runs in a second browser (another computer,
 * as far as the page can tell) first. %CODE% becomes a group code of this run.
 */
const SHOTS = {
  hero: {
    file: 'docs/screenshot.png',
    setup: `await s.loadPreset('Choir Pad'); s.page('osc'); await wait(600); chord([48, 55, 63], 0.65); await wait(2000);`,
  },
  oscillators: {
    // one of each kind: a wavetable in 3D, a granular recording, a spectral analysis
    setup: `await s.loadPreset('Choir Pad'); s.page('osc');
      s.setPlain('osc.b.type', 3); await s.recording.hits(1, [0, 0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75], 2);
      s.setPlain('osc.c.enable', 1); s.setPlain('osc.c.type', 4); await s.recording.tone(2, 220, 2);
      await wait(1500); chord([48, 55, 60], 0.65); await wait(1800);`,
    crop: ['[aria-label="Sub oscillator"]', '[aria-label="Noise oscillator"]', '[aria-label="Osc A"]', '[aria-label="Osc B"]', '[aria-label="Osc C"]'],
  },
  filters: {
    // side by side: osc A into Filter 1, osc B into Filter 2 with a share back, the noise direct
    setup: `await s.loadPreset('Reese'); s.page('mix');
      s.setPlain('mix.filter_routing', 1);
      s.setPlain('filter.2.enable', 1); s.setPlain('filter.2.type', 2); s.setPlain('filter.2.cutoff', 1800);
      s.setPlain('osc.b.route', 1); s.setPlain('osc.b.balance', 0.3);
      s.setPlain('noise.enable', 1); s.setPlain('noise.level', 0.2); s.setPlain('noise.route', 3);
      await wait(500); chord([36, 43], 0.8); await wait(1800);`,
    crop: ['[aria-label="Signal routing"]', '.mix-page .strip .row'],
  },
  modulation: {
    setup: `await s.loadPreset('Evolving Vowels'); s.page('matrix');
      const n = s.matrix().length;
      s.mod(n, 'Env 2', 'filter.1.cutoff', 0.45); s.mod(n + 1, 'Velocity', 'osc.a.level', 0.3);
      s.mod(n + 2, 'Macro 1', 'filter.1.res', 0.6); s.mod(n + 3, 'Mod Wheel', 'filter.1.drive', 0.5);
      await wait(300); chord([48, 55, 62], 0.8); await wait(1800);`,
    crop: ['[aria-label="Modulation matrix"] > header', '[aria-label="Modulation matrix"] .tr'],
  },
  effects: {
    setup: `await s.loadPreset('BOC Olson'); s.page('fx'); await wait(400);
      document.querySelector('[aria-label="Edit Reverb 1"]').click();
      await wait(300); chord([52, 56, 59], 0.6); await wait(1800);`,
    // the rack and the open effect's controls (the rest of the page is room to grow)
    crop: ['main.page'],
    maxHeight: 270,
  },
  sequencing: {
    setup: `await s.loadPreset('BOC Olson'); s.page('clip'); await wait(500); s.seq.play(true); await wait(4500);`,
    crop: ['main.page'],
  },
  editor: {
    setup: `await s.loadPreset('Evolving Vowels'); s.page('osc'); await wait(300); s.editor.open(0); await wait(600); s.editor.select(40); await wait(900);`,
    crop: ['section.editor'],
  },
  presets: {
    setup: `await s.loadPreset('Choir Pad'); await wait(300); document.querySelector('[aria-label="Browse presets"]').click(); await wait(1000);`,
    crop: ['section.browser'],
  },
  global: {
    setup: `s.page('global'); await wait(800);`,
    crop: ['main.page section.panel'],
  },
  explainer: {
    setup: `await s.loadPreset('Choir Pad'); s.page('osc'); await wait(500); chord([48, 55, 60], 0.65); await wait(800); s.explain.open('filter.1'); await wait(1500);`,
    crop: ['.callout', 'main.page [data-explain="filter.1"]'],
    dpr: 3,
  },
  flow: {
    // the signal-flow tour's patch uses every path; the tour's card and highlight stay out of the picture
    setup: `s.tour.start('flow'); await wait(2500);
      document.querySelectorAll('.ring, .card.explain-ui').forEach((e) => (e.style.display = 'none')); await wait(300);`,
    crop: ['[aria-label="Signal flow"]'],
  },
  link: {
    // two computers, as far as Link can tell: this browser and a second one, on a group code of their own
    companion: `s.link.name('Laptop'); s.link.code('%CODE%'); s.link.net(true); s.link.on(true);`,
    setup: `s.link.name('Studio Mac'); s.link.code('%CODE%'); s.link.net(true); s.link.on(true); s.page('global');
      for (let i = 0; i < 200 && !s.link.state().members.some((m) => m.via === 'direct' && m.rtt !== null); i++) await wait(100);
      await wait(1500);`,
    crop: ['[aria-label="Link"]'],
    dpr: 3,
  },
};

const args = process.argv.slice(2);
let url = 'https://birdsynth.abusing.technology';
const pick = [];
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--url') url = args[++i];
  else pick.push(args[i]);
}
for (const name of pick) if (!SHOTS[name]) throw new Error(`no shot ${name}; there are ${Object.keys(SHOTS).join(', ')}`);
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

/** A headless Chrome on its own profile, showing `url`, with the synth running. */
async function open(port, dpr) {
  const profile = mkdtempSync(path.join(tmpdir(), 'birdsynth-shot-'));
  const chrome = spawn(
    CHROME,
    ['--headless=new', `--user-data-dir=${profile}`, `--remote-debugging-port=${port}`, '--autoplay-policy=no-user-gesture-required', '--mute-audio', '--hide-scrollbars', '--no-first-run', `--window-size=${W},${H}`, 'about:blank'],
    { stdio: 'ignore' },
  );
  const close = async () => {
    chrome.kill();
    await wait(300);
    rmSync(profile, { recursive: true, force: true });
  };
  try {
    let target;
    for (let i = 0; i < 50 && !target; i++) {
      await wait(200);
      try {
        target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === 'page');
      } catch {
        // not up yet
      }
    }
    if (!target) throw new Error('Chrome did not start');
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((ok, fail) => ((ws.onopen = ok), (ws.onerror = fail)));
    let id = 0;
    const pending = new Map();
    const events = new Set();
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id && pending.has(msg.id)) {
        pending.get(msg.id)(msg);
        pending.delete(msg.id);
      } else if (msg.method) events.add(msg.method);
    };
    const send = (method, params = {}) =>
      new Promise((ok, fail) => {
        const i = ++id;
        pending.set(i, (msg) => (msg.error ? fail(new Error(`${method}: ${msg.error.message}`)) : ok(msg.result)));
        ws.send(JSON.stringify({ id: i, method, params }));
      });
    const run = async (expression) => {
      const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
      if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
      return r.result.value;
    };
    await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: dpr, mobile: false });
    await send('Page.enable');
    await send('Page.navigate', { url });
    for (let i = 0; i < 100 && !events.has('Page.loadEventFired'); i++) await wait(100);
    // the engine (and the first wavetable) arrive after the page does
    for (let i = 0; i < 150 && (await run(`window.__synth?.status?.() ?? 'none'`)) !== 'running'; i++) await wait(100);
    return { send, run, close: async () => (ws.close(), close()) };
  } catch (e) {
    await close();
    throw e;
  }
}

async function shoot(name, shot, port) {
  const code = `JAM${Math.random().toString(36).slice(2, 6).toUpperCase()}`;
  const js = (src) => `(async () => { ${PRELUDE} ${(src ?? '').replaceAll('%CODE%', code)} })()`;
  const dpr = shot.dpr ?? 2;
  let companion = null;
  let page = null;
  try {
    if (shot.companion) {
      companion = await open(port + 100, 1);
      await companion.run(js(shot.companion));
    }
    page = await open(port, dpr);
    await page.run(js(shot.setup));
    let clip;
    if (shot.crop) {
      const box = await page.run(`(() => {
        const rects = ${JSON.stringify(shot.crop)}.flatMap((sel) => [...document.querySelectorAll(sel)]).map((e) => e.getBoundingClientRect()).filter((r) => r.width && r.height);
        if (!rects.length) return null;
        const pad = 6;
        const x = Math.max(0, Math.min(...rects.map((r) => r.left)) - pad);
        const y = Math.max(0, Math.min(...rects.map((r) => r.top)) - pad);
        return { x, y, width: Math.min(innerWidth, Math.max(...rects.map((r) => r.right)) + pad) - x, height: Math.min(innerHeight, Math.max(...rects.map((r) => r.bottom)) + pad) - y };
      })()`);
      if (!box) throw new Error(`nothing on screen matches ${shot.crop.join(', ')}`);
      clip = { ...box, height: Math.min(box.height, shot.maxHeight ?? Infinity), scale: 1 };
    }
    const png = await page.send('Page.captureScreenshot', { format: 'png', ...(clip ? { clip } : {}) });
    const out = path.join(ROOT, shot.file ?? `docs/screenshots/${name}.png`);
    mkdirSync(path.dirname(out), { recursive: true });
    writeFileSync(out, Buffer.from(png.data, 'base64'));
    const px = clip ? `${Math.round(clip.width * dpr)}×${Math.round(clip.height * dpr)}` : `${W * dpr}×${H * dpr}`;
    console.log(`${name.padEnd(12)} ${px.padEnd(10)} ${path.relative(ROOT, out)}`);
  } catch (e) {
    throw new Error(`${name}: ${e.message}`);
  } finally {
    await page?.close();
    await companion?.close();
  }
}

let port = 9333;
for (const [name, shot] of Object.entries(SHOTS)) if (!pick.length || pick.includes(name)) await shoot(name, shot, port++);
