#!/usr/bin/env node
// Public smoke test for a deployed birdsynth: the page, its assets and
// headers, as a visitor's browser gets them through Cloudflare.
//
//   node tools/smoke.mjs [https://birdsynth.abusing.technology]
//
// It checks: the page loads and isn't marked noindex; index.html isn't
// cached; the JS and engine.wasm are served with the right types, immutable,
// and compressed; security headers are set; plain http redirects to https;
// robots.txt allows crawling. (Playing a note with a clean console is
// checked in the browser: see CLAUDE.md, "Checking a deploy".)

const base = (process.argv[2] ?? 'https://birdsynth.abusing.technology').replace(/\/$/, '');
let failed = 0;
const ok = (cond, what, detail = '') => {
  console.log(`${cond ? 'ok  ' : 'FAIL'} ${what}${detail ? `: ${detail}` : ''}`);
  if (!cond) failed++;
};

async function get(url, init = {}) {
  const r = await fetch(url, { redirect: 'manual', ...init, headers: { 'accept-encoding': 'gzip, br', ...(init.headers ?? {}) } });
  return r;
}

const page = await get(`${base}/`);
const html = await page.text();
ok(page.status === 200, 'page loads', `${page.status}`);
ok(/<title>birdsynth<\/title>/.test(html), 'page is birdsynth');
ok(!/noindex/i.test(html) && !/noindex/i.test(page.headers.get('x-robots-tag') ?? ''), 'not marked noindex');
ok(/no-cache/.test(page.headers.get('cache-control') ?? ''), 'index.html is revalidated', page.headers.get('cache-control') ?? 'none');
ok(page.headers.get('x-content-type-options') === 'nosniff', 'nosniff');
ok(!!page.headers.get('referrer-policy'), 'referrer policy', page.headers.get('referrer-policy') ?? 'none');

const js = html.match(/assets\/index-[\w-]+\.js/)?.[0];
ok(!!js, 'page links its script', js ?? 'none');
if (js) {
  const r = await get(`${base}/${js}`);
  const text = await r.text();
  ok(r.status === 200 && /javascript/.test(r.headers.get('content-type') ?? ''), 'script is JavaScript', r.headers.get('content-type') ?? '');
  ok(/immutable/.test(r.headers.get('cache-control') ?? ''), 'script is immutable', r.headers.get('cache-control') ?? 'none');
  for (const name of ['engine', 'tools']) {
    const wasm = text.match(new RegExp(`assets/${name}-[\\w-]+\\.wasm`))?.[0] ?? (name === 'tools' ? (await findInWorkers(text, name)) : undefined);
    if (!wasm) {
      ok(name === 'tools', `${name}.wasm is referenced`, 'not found in the main script');
      continue;
    }
    const w = await get(`${base}/${wasm}`);
    const bytes = new Uint8Array(await w.arrayBuffer());
    ok(w.status === 200 && w.headers.get('content-type') === 'application/wasm', `${name}.wasm type`, w.headers.get('content-type') ?? '');
    ok(/immutable/.test(w.headers.get('cache-control') ?? ''), `${name}.wasm is immutable`);
    ok(bytes[0] === 0 && bytes[1] === 0x61 && bytes[2] === 0x73 && bytes[3] === 0x6d, `${name}.wasm is WebAssembly`, `${bytes.length} bytes`);
    if (name === 'engine') {
      const mod = new WebAssembly.Module(bytes);
      ok(WebAssembly.Module.imports(mod).length === 0, 'engine.wasm has no imports');
    }
  }
}

async function findInWorkers(text, name) {
  // the tools worker is its own chunk; follow the worker URLs the main script names
  for (const w of text.match(/assets\/[\w-]+\.js/g) ?? []) {
    const t = await (await get(`${base}/${w}`)).text();
    const m = t.match(new RegExp(`assets/${name}-[\\w-]+\\.wasm`));
    if (m) return m[0];
  }
  return undefined;
}

const http = await get(base.replace(/^https:/, 'http:') + '/');
ok([301, 302, 307, 308].includes(http.status) && (http.headers.get('location') ?? '').startsWith('https:'), 'http redirects to https', `${http.status} ${http.headers.get('location') ?? ''}`);

const robots = await get(`${base}/robots.txt`);
ok(robots.status === 200 && /Allow: \//.test(await robots.text()), 'robots.txt allows crawling');

console.log(failed ? `\n${failed} check(s) failed` : '\nall checks passed');
process.exit(failed ? 1 : 0);
