#!/usr/bin/env node
// Generates the Rust and TypeScript codecs from params/**/*.toml and
// schema/protocol.toml, so the engine and the host share one spec.
//
//   node tools/gen-schema.mjs          write the generated files
//   node tools/gen-schema.mjs --check  exit 1 if a generated file is stale
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'smol-toml';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const rel = (...p) => path.join(ROOT, ...p);
const read = (p) => fs.readFileSync(p, 'utf8');
const f32 = Math.fround;

function fail(msg) {
  console.error(`gen-schema: ${msg}`);
  process.exit(2);
}

function listToml(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...listToml(p));
    else if (e.name.endsWith('.toml')) out.push(p);
  }
  return out;
}

const screaming = (s) => s.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replace(/[^A-Za-z0-9]+/g, '_').toUpperCase();
const snake = (s) => s.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replace(/[^A-Za-z0-9]+/g, '_').toLowerCase();
const camel = (s) => s.replace(/[-_.]+([a-z0-9])/g, (_, c) => c.toUpperCase()).replace(/^[A-Z]/, (c) => c.toLowerCase());

// Shortest Rust float literal that parses to the same f32 as v.
function rf(v) {
  const want = f32(v);
  let s = String(want);
  for (let digits = 1; digits <= 9; digits++) {
    const t = String(Number(want.toPrecision(digits)));
    if (f32(Number(t)) === want) {
      s = t;
      break;
    }
  }
  return /[.e]/.test(s) ? s : `${s}.0`;
}

// ------------------------------------------------------------------ params
const FLAG = { mod: 1, smooth: 2 };

function parseCurve(p, where) {
  const c = p.curve ?? 'lin';
  if (c === 'lin' || c === 'exp' || c === 'db' || c === 'int' || c === 'bool') return { kind: c };
  const pow = /^pow:(\d+(?:\.\d+)?)$/.exec(c);
  if (pow) return { kind: 'pow', k: Number(pow[1]) };
  if (c === 'enum') {
    if (!Array.isArray(p.options) || p.options.length < 2) fail(`${where}: enum needs at least two options`);
    return { kind: 'enum', options: p.options.map(String) };
  }
  fail(`${where}: unknown curve "${c}"`);
}

function toNorm(curve, min, max, v) {
  switch (curve.kind) {
    case 'lin':
    case 'int':
      return (v - min) / (max - min);
    case 'db':
      return v === -Infinity ? 0 : (v - min) / (max - min);
    case 'exp':
      return Math.log(v / min) / Math.log(max / min);
    case 'pow':
      return Math.pow((v - min) / (max - min), 1 / curve.k);
    case 'bool':
      return v ? 1 : 0;
    case 'enum':
      return v / (curve.options.length - 1);
  }
}

const params = [];
for (const file of listToml(rel('params'))) {
  const doc = parse(read(file));
  const where0 = path.relative(ROOT, file);
  const g = doc.group;
  if (!g || typeof g.key !== 'string' || typeof g.name !== 'string') fail(`${where0}: needs [group] with key and name`);
  const instances = g.instances ?? [null];
  const instNames = g.instance_names ?? instances.map(() => g.name);
  if (instNames.length !== instances.length) fail(`${where0}: instance_names must match instances`);
  const locals = new Set();
  const specs = (doc.param ?? []).map((p) => {
    const where = `${where0} param ${p.key}`;
    if (typeof p.key !== 'string' || typeof p.name !== 'string') fail(`${where}: needs key and name`);
    if (locals.has(p.key)) fail(`${where}: duplicate key`);
    locals.add(p.key);
    const curve = parseCurve(p, where);
    let min = p.min ?? 0;
    let max = p.max ?? 1;
    if (curve.kind === 'bool') [min, max] = [0, 1];
    if (curve.kind === 'enum') [min, max] = [0, curve.options.length - 1];
    if (!(max > min)) fail(`${where}: max must exceed min`);
    if (curve.kind === 'exp' && !(min > 0)) fail(`${where}: exp curves need min > 0`);
    const flags = (p.flags ?? []).reduce((acc, f) => {
      if (!(f in FLAG)) fail(`${where}: unknown flag "${f}"`);
      return acc | FLAG[f];
    }, 0);
    return { p, where, curve, min, max, flags };
  });
  // Instance-major order: env.1.* before env.2.*.
  instances.forEach((inst, i) => {
    for (const { p, where, curve, min, max, flags } of specs) {
      const plainDef = Array.isArray(p.default) ? p.default[i] : p.default;
      if (plainDef === undefined) fail(`${where}: missing default`);
      const def = toNorm(curve, min, max, plainDef);
      if (!(def >= 0 && def <= 1)) fail(`${where}: default ${plainDef} is outside ${min}..${max}`);
      params.push({
        key: [g.key, inst, p.key].filter((v) => v != null).join('.'),
        group: g.key,
        instance: inst,
        local: p.key,
        name: inst == null ? p.name : `${instNames[i]} ${p.name}`,
        short: p.short ?? p.name,
        curve,
        min,
        max,
        def,
        unit: p.unit ?? '',
        flags,
        explain: p.explain ?? '',
        instanced: inst != null,
      });
    }
  });
}
params.forEach((p, i) => (p.id = i));
const keys = new Set();
for (const p of params) {
  if (keys.has(p.key)) fail(`duplicate parameter key ${p.key}`);
  keys.add(p.key);
}

// ---------------------------------------------------------------- protocol
const proto = parse(read(rel('schema/protocol.toml')));
const SIZES = { u8: 1, u16: 2, u32: 4, i32: 4, f32: 4, f64: 8 };
const align = (n, a) => Math.ceil(n / a) * a;

const constants = proto.constants ?? {};
const commands = (proto.command ?? []).map((c) => {
  let off = 0;
  const fields = (c.fields ?? []).map((f) => {
    const size = SIZES[f.type];
    if (!size) fail(`command ${c.name}: unknown type ${f.type}`);
    off = align(off, size);
    const out = { name: f.name, type: f.type, offset: off, size };
    off += size;
    return out;
  });
  return { name: c.name, op: c.op, doc: c.doc ?? '', fields, bytes: align(off, 8) };
});
{
  const ops = new Set();
  for (const c of commands) {
    if (ops.has(c.op)) fail(`duplicate op ${c.op}`);
    ops.add(c.op);
  }
}
let telLen = 0;
const telemetry = (proto.telemetry ?? []).map((t) => {
  const out = { name: t.name, offset: telLen, count: t.count ?? 1 };
  telLen += out.count;
  return out;
});
const taps = (proto.tap ?? []).map((t, i) => ({ name: t.name, index: i }));
if (taps.length > 32) fail('at most 32 taps fit the u32 tap mask');
const debug = proto.debug ?? {};

// ------------------------------------------------------------------ hash
const canon = JSON.stringify({
  params: params.map((p) => [p.key, p.curve.kind, p.curve.k ?? null, p.curve.options ?? null, f32(p.min), f32(p.max), f32(p.def), p.flags]),
  constants,
  commands: commands.map((c) => [c.name, c.op, c.bytes, c.fields.map((f) => [f.name, f.type, f.offset])]),
  telemetry: telemetry.map((t) => [t.name, t.offset, t.count]),
  taps: taps.map((t) => t.name),
  debug,
});
let hash = 0x811c9dc5;
for (const b of Buffer.from(canon, 'utf8')) {
  hash ^= b;
  hash = Math.imul(hash, 0x01000193) >>> 0;
}
const HASH = `0x${hash.toString(16).padStart(8, '0')}`;

const BANNER = '@generated by tools/gen-schema.mjs from params/**/*.toml and schema/protocol.toml. Do not edit.';

// ------------------------------------------------------------ rust: params
function rustCurve(c) {
  switch (c.kind) {
    case 'lin': return 'Curve::Lin';
    case 'exp': return 'Curve::Exp';
    case 'db': return 'Curve::Db';
    case 'int': return 'Curve::Int';
    case 'bool': return 'Curve::Bool';
    case 'pow': return `Curve::Pow(${rf(c.k)})`;
    case 'enum': return `Curve::Enum(${c.options.length})`;
  }
}

function rustParams() {
  const L = [`// ${BANNER}`, '', 'use crate::params::{Curve, ParamInfo};', '', `pub const COUNT: usize = ${params.length};`, ''];
  const seen = new Set();
  for (const p of params) {
    const name = screaming(`${p.group}_${p.local}`);
    if (seen.has(name)) continue;
    seen.add(name);
    if (p.instanced) {
      const ids = params.filter((q) => q.group === p.group && q.local === p.local).map((q) => q.id);
      L.push(`pub const ${name}: [u16; ${ids.length}] = [${ids.join(', ')}];`);
    } else {
      L.push(`pub const ${name}: u16 = ${p.id};`);
    }
  }
  L.push('', 'pub static INFO: [ParamInfo; COUNT] = [');
  for (const p of params) {
    L.push(`    ParamInfo { key: "${p.key}", curve: ${rustCurve(p.curve)}, min: ${rf(p.min)}, max: ${rf(p.max)}, default: ${rf(p.def)}, flags: ${p.flags} },`);
  }
  L.push('];', '');
  return L.join('\n');
}

// ---------------------------------------------------------- rust: protocol
const RUST_TY = { u8: 'u8', u16: 'u16', u32: 'u32', i32: 'i32', f32: 'f32', f64: 'f64' };

function rustProtocol() {
  const L = [`// ${BANNER}`, '', `pub const ABI_HASH: u32 = ${HASH};`, 'pub const HEADER_BYTES: usize = 16;'];
  for (const [k, v] of Object.entries(constants)) L.push(`pub const ${screaming(k)}: usize = ${v};`);
  L.push('', 'pub mod op {');
  for (const c of commands) L.push(`    pub const ${screaming(c.name)}: u16 = ${c.op};`);
  L.push('}', '', 'pub mod bytes {');
  for (const c of commands) L.push(`    pub const ${screaming(c.name)}: usize = ${c.bytes};`);
  L.push('}', '', '#[derive(Clone, Copy, Debug, PartialEq)]', 'pub enum Command {');
  for (const c of commands) {
    if (c.doc) L.push(`    /// ${c.doc}`);
    if (!c.fields.length) L.push(`    ${c.name},`);
    else L.push(`    ${c.name} { ${c.fields.map((f) => `${snake(f.name)}: ${RUST_TY[f.type]}`).join(', ')} },`);
  }
  L.push('}', '', '/// Decode one payload. Returns None for unknown ops or short payloads.', 'pub fn decode(op: u16, p: &[u8]) -> Option<Command> {', '    match op {');
  for (const c of commands) {
    if (!c.fields.length) {
      L.push(`        op::${screaming(c.name)} => Some(Command::${c.name}),`);
      continue;
    }
    const need = c.fields.at(-1).offset + c.fields.at(-1).size;
    const init = c.fields.map((f) => `${snake(f.name)}: rd_${f.type}(p, ${f.offset})`).join(', ');
    L.push(`        op::${screaming(c.name)} if p.len() >= ${need} => Some(Command::${c.name} { ${init} }),`);
  }
  L.push('        _ => None,', '    }', '}', '');
  const used = new Set(commands.flatMap((c) => c.fields.map((f) => f.type)));
  for (const t of ['u8', 'u16', 'u32', 'i32', 'f32', 'f64']) {
    if (!used.has(t)) continue;
    const n = SIZES[t];
    if (n === 1) L.push(`#[inline]\nfn rd_${t}(p: &[u8], o: usize) -> ${t} {\n    p[o]${t === 'u8' ? '' : ` as ${t}`}\n}`);
    else L.push(`#[inline]\nfn rd_${t}(p: &[u8], o: usize) -> ${t} {\n    let mut b = [0u8; ${n}];\n    b.copy_from_slice(&p[o..o + ${n}]);\n    ${t}::from_le_bytes(b)\n}`);
  }
  L.push('', 'pub mod debug {');
  for (const [k, v] of Object.entries(debug)) L.push(`    pub const ${screaming(k)}: u32 = ${v};`);
  L.push('}', '', '/// Telemetry slot offsets (f32 each).', 'pub mod tel {');
  for (const t of telemetry) {
    L.push(`    pub const ${screaming(t.name)}: usize = ${t.offset};`);
    if (t.count > 1) L.push(`    pub const ${screaming(t.name)}_LEN: usize = ${t.count};`);
  }
  L.push(`    pub const LEN: usize = ${telLen};`, '}', '', 'pub mod tap {');
  for (const t of taps) L.push(`    pub const ${screaming(t.name)}: usize = ${t.index};`);
  L.push(`    pub const COUNT: usize = ${taps.length};`, `    pub const NAMES: [&str; COUNT] = [${taps.map((t) => `"${t.name}"`).join(', ')}];`, '}', '');
  return L.join('\n');
}

// -------------------------------------------------------------- ts: params
function tsCurve(c) {
  switch (c.kind) {
    case 'pow': return `{ kind: 'pow', k: ${c.k} }`;
    case 'enum': return `{ kind: 'enum', options: ${JSON.stringify(c.options)} }`;
    default: return `{ kind: '${c.kind}' }`;
  }
}

function tsParams() {
  const L = [
    `// ${BANNER}`,
    '',
    "export type Curve = { kind: 'lin' } | { kind: 'exp' } | { kind: 'db' } | { kind: 'int' } | { kind: 'bool' } | { kind: 'pow'; k: number } | { kind: 'enum'; options: readonly string[] };",
    '',
    'export interface ParamInfo {',
    '  readonly id: number;',
    '  readonly key: string;',
    '  readonly group: string;',
    '  readonly instance: string | null;',
    '  readonly local: string;',
    '  /** Full name, unique across the synth (used as the ARIA label). */',
    '  readonly name: string;',
    '  /** Name within its section, as printed under a knob. */',
    '  readonly short: string;',
    '  readonly curve: Curve;',
    '  readonly min: number;',
    '  readonly max: number;',
    '  /** Normalized default, 0..1 (rounded to f32 like the engine). */',
    '  readonly def: number;',
    '  readonly unit: string;',
    '  readonly flags: number;',
    '  readonly explain: string;',
    '}',
    '',
    'export const FLAG = { mod: 1, smooth: 2 } as const;',
    '',
    'export const PARAMS: readonly ParamInfo[] = [',
  ];
  for (const p of params) {
    L.push(
      `  { id: ${p.id}, key: ${JSON.stringify(p.key)}, group: ${JSON.stringify(p.group)}, instance: ${JSON.stringify(p.instance)}, local: ${JSON.stringify(p.local)}, name: ${JSON.stringify(p.name)}, short: ${JSON.stringify(p.short)}, curve: ${tsCurve(p.curve)}, min: ${f32(p.min)}, max: ${f32(p.max)}, def: ${f32(p.def)}, unit: ${JSON.stringify(p.unit)}, flags: ${p.flags}, explain: ${JSON.stringify(p.explain)} },`,
    );
  }
  L.push('];', '', 'export const PARAM_KEYS = [');
  for (const p of params) L.push(`  ${JSON.stringify(p.key)},`);
  L.push('] as const;', '', 'export type ParamKey = (typeof PARAM_KEYS)[number];', '', 'export const PARAM_ID: Readonly<Record<ParamKey, number>> = {');
  for (const p of params) L.push(`  ${JSON.stringify(p.key)}: ${p.id},`);
  L.push('};', '');
  return L.join('\n');
}

// ------------------------------------------------------------ ts: protocol
const DV = { u8: 'Uint8', u16: 'Uint16', u32: 'Uint32', i32: 'Int32', f32: 'Float32', f64: 'Float64' };

function tsProtocol() {
  const L = [`// ${BANNER}`, '', `export const ABI_HASH = ${HASH};`, 'export const HEADER_BYTES = 16;', '', 'export const CONST = {'];
  for (const [k, v] of Object.entries(constants)) L.push(`  ${camel(k)}: ${v},`);
  L.push('} as const;', '', 'export const OP = {');
  for (const c of commands) L.push(`  ${c.name}: ${c.op},`);
  L.push('} as const;', '', 'export const DEBUG = {');
  for (const [k, v] of Object.entries(debug)) L.push(`  ${k}: ${v},`);
  L.push('} as const;', '');
  L.push(
    '/** Appends binary commands; see schema/protocol.toml for the layout. */',
    'export class CmdWriter {',
    '  private buf: ArrayBuffer;',
    '  private dv: DataView;',
    '  private u8: Uint8Array;',
    '  private pos = 0;',
    '  /** Offsets where each command starts, so batches can be split between commands. */',
    '  private starts: number[] = [];',
    '',
    '  constructor(capacity = 1 << 16) {',
    '    this.buf = new ArrayBuffer(capacity);',
    '    this.dv = new DataView(this.buf);',
    '    this.u8 = new Uint8Array(this.buf);',
    '  }',
    '',
    '  get length(): number {',
    '    return this.pos;',
    '  }',
    '',
    '  private head(op: number, bytes: number, frame: number): number {',
    '    const need = this.pos + HEADER_BYTES + bytes;',
    '    if (need > this.buf.byteLength) this.grow(need);',
    '    const p = this.pos;',
    '    this.starts.push(p);',
    '    this.u8.fill(0, p, need);',
    '    this.dv.setUint16(p, op, true);',
    '    this.dv.setUint32(p + 4, bytes, true);',
    '    this.dv.setFloat64(p + 8, frame, true);',
    '    this.pos = need;',
    '    return p + HEADER_BYTES;',
    '  }',
    '',
    '  private grow(need: number): void {',
    '    let cap = this.buf.byteLength * 2;',
    '    while (cap < need) cap *= 2;',
    '    const next = new ArrayBuffer(cap);',
    '    new Uint8Array(next).set(this.u8.subarray(0, this.pos));',
    '    this.buf = next;',
    '    this.dv = new DataView(next);',
    '    this.u8 = new Uint8Array(next);',
    '  }',
    '',
    '  /** The bytes written so far, without copying. */',
    '  bytes(): Uint8Array {',
    '    return this.u8.subarray(0, this.pos);',
    '  }',
    '',
    '  /** Forget everything written. (The Reset command is reset(frame).) */',
    '  clear(): void {',
    '    this.pos = 0;',
    '    this.starts.length = 0;',
    '  }',
    '',
    '  /** Copies the batch out as ArrayBuffers of at most maxBytes each, split between commands, then resets. */',
    '  take(maxBytes: number = CONST.cmdCapacity): ArrayBuffer[] {',
    '    const out: ArrayBuffer[] = [];',
    '    let from = 0;',
    '    for (let i = 0; i < this.starts.length; i++) {',
    '      const end = i + 1 < this.starts.length ? this.starts[i + 1] : this.pos;',
    '      if (end - from > maxBytes && this.starts[i] > from) {',
    '        out.push(this.buf.slice(from, this.starts[i]));',
    '        from = this.starts[i];',
    '      }',
    '    }',
    '    if (this.pos > from) out.push(this.buf.slice(from, this.pos));',
    '    this.clear();',
    '    return out;',
    '  }',
  );
  for (const c of commands) {
    const args = ['frame: number', ...c.fields.map((f) => `${camel(f.name)}: number`)].join(', ');
    L.push('', `  /** ${c.doc || c.name} */`, `  ${camel(c.name)}(${args}): void {`);
    if (c.fields.length) L.push(`    const o = this.head(${c.op}, ${c.bytes}, frame);`);
    else L.push(`    this.head(${c.op}, ${c.bytes}, frame);`);
    for (const f of c.fields) {
      const le = f.size > 1 ? ', true' : '';
      L.push(`    this.dv.set${DV[f.type]}(o + ${f.offset}, ${camel(f.name)}${le});`);
    }
    L.push('  }');
  }
  L.push('}', '', '/** Telemetry slot offsets into the engine\'s f32 telemetry array. */', 'export const TEL = {');
  for (const t of telemetry) L.push(`  ${t.name}: ${t.offset},`);
  L.push(`  LEN: ${telLen},`, '} as const;', '', 'export const TEL_COUNT = {');
  for (const t of telemetry) L.push(`  ${t.name}: ${t.count},`);
  L.push('} as const;', '', `export const TAP_NAMES = [${taps.map((t) => JSON.stringify(t.name)).join(', ')}] as const;`, '', 'export type TapName = (typeof TAP_NAMES)[number];', '', 'export const TAP: Readonly<Record<TapName, number>> = {');
  for (const t of taps) L.push(`  ${JSON.stringify(t.name)}: ${t.index},`);
  L.push('};', '');
  return L.join('\n');
}

// ------------------------------------------------------------------ write
const outputs = [
  ['crates/engine/src/spec/params.rs', rustParams()],
  ['crates/engine/src/spec/protocol.rs', rustProtocol()],
  ['web/src/gen/params.ts', tsParams()],
  ['web/src/gen/protocol.ts', tsProtocol()],
];

const check = process.argv.includes('--check');
let stale = 0;
for (const [file, text] of outputs) {
  const full = rel(file);
  const old = fs.existsSync(full) ? read(full) : null;
  if (old === text) continue;
  if (check) {
    console.error(`stale: ${file}`);
    stale++;
  } else {
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, text);
    console.log(`wrote ${file}`);
  }
}
if (check && stale) {
  console.error('Run `npm run gen` to regenerate.');
  process.exit(1);
}
if (!check) console.log(`${params.length} params, ${commands.length} commands, ${telLen} telemetry slots, ${taps.length} taps, ABI ${HASH}`);
