// Normalized <-> plain mapping and display formatting for parameters.
// toPlain mirrors ParamInfo::to_plain in crates/engine/src/params.rs; a test
// checks the two agree.

import type { ParamInfo } from '../gen/params';

const clamp01 = (n: number) => (n < 0 ? 0 : n > 1 ? 1 : n);

export function toPlain(p: ParamInfo, norm: number): number {
  const n = Number.isNaN(norm) ? p.def : clamp01(norm);
  const span = p.max - p.min;
  const c = p.curve;
  switch (c.kind) {
    case 'lin':
      return p.min + span * n;
    case 'exp':
      return p.min * Math.pow(p.max / p.min, n);
    case 'pow':
      return p.min + span * Math.pow(n, c.k);
    case 'db':
      return n <= 0 ? -Infinity : p.min + span * n;
    case 'int':
      return Math.floor(p.min + span * n + 0.5);
    case 'bool':
      return n >= 0.5 ? 1 : 0;
    case 'enum':
      return Math.floor(n * (c.options.length - 1) + 0.5);
  }
}

export function toNorm(p: ParamInfo, plain: number): number {
  const span = p.max - p.min;
  const c = p.curve;
  switch (c.kind) {
    case 'lin':
    case 'int':
      return clamp01((plain - p.min) / span);
    case 'db':
      return plain === -Infinity ? 0 : clamp01((plain - p.min) / span);
    case 'exp':
      return clamp01(Math.log(plain / p.min) / Math.log(p.max / p.min));
    case 'pow':
      return clamp01(Math.pow(Math.max(0, (plain - p.min) / span), 1 / c.k));
    case 'bool':
      return plain ? 1 : 0;
    case 'enum':
      return clamp01(plain / (c.options.length - 1));
  }
}

/** Number of discrete steps, or 0 for a continuous parameter. */
export function steps(p: ParamInfo): number {
  switch (p.curve.kind) {
    case 'int':
      return Math.round(p.max - p.min);
    case 'bool':
      return 1;
    case 'enum':
      return p.curve.options.length - 1;
    default:
      return 0;
  }
}

/** Snap a normalized value onto the parameter's steps (continuous ones pass through). */
export function snap(p: ParamInfo, norm: number): number {
  const s = steps(p);
  const n = clamp01(norm);
  return s ? Math.round(n * s) / s : n;
}

/** Fixed digits with an explicit sign; zero shows unsigned. */
function signed(v: number, digits: number): string {
  const s = v.toFixed(digits);
  if (Number(s) === 0) return (0).toFixed(digits);
  return v > 0 ? `+${s}` : s;
}

function time(ms: number): string {
  if (ms < 10) return `${ms.toFixed(2)} ms`;
  if (ms < 1000) return `${ms.toFixed(1)} ms`;
  return `${(ms / 1000).toFixed(2)} s`;
}

/** Human-readable value, as shown on a knob. */
export function format(p: ParamInfo, norm: number): string {
  const v = toPlain(p, norm);
  const c = p.curve;
  if (c.kind === 'bool') return v ? 'On' : 'Off';
  if (c.kind === 'enum') return c.options[v] ?? String(v);
  switch (p.unit) {
    case 'dB':
      return v === -Infinity ? '-inf dB' : `${signed(v, 1)} dB`;
    case '%':
      return `${Math.round(v * 100)} %`;
    case 'ms':
      return time(v);
    case 'pan':
      return Math.abs(v) < 0.005 ? 'C' : v < 0 ? `L${Math.round(-v * 100)}` : `R${Math.round(v * 100)}`;
    case 'ct':
      return `${signed(v, 0)} ct`;
    case 'st':
      return c.kind === 'int' ? `${signed(v, 0)} st` : `${signed(v, 2)} st`;
    case 'oct':
      return `${signed(v, 0)} oct`;
    case '':
      return c.kind === 'int' ? String(v) : v.toFixed(2);
    default:
      return c.kind === 'int' ? `${v} ${p.unit}` : `${v.toFixed(2)} ${p.unit}`;
  }
}

/** Parse a typed value (as entered on a knob) back to normalized, or null. */
export function parse(p: ParamInfo, text: string): number | null {
  const t = text.trim().toLowerCase();
  const c = p.curve;
  if (c.kind === 'bool') {
    if (['on', '1', 'true', 'yes'].includes(t)) return 1;
    if (['off', '0', 'false', 'no'].includes(t)) return 0;
    return null;
  }
  if (c.kind === 'enum') {
    const i = c.options.findIndex((o) => o.toLowerCase() === t);
    return i >= 0 ? toNorm(p, i) : null;
  }
  if (p.unit === 'dB' && (t === '-inf' || t === '-inf db')) return 0;
  if (p.unit === 'pan') {
    if (t === 'c') return toNorm(p, 0);
    const m = /^([lr])\s*(\d+(?:\.\d+)?)$/.exec(t);
    if (m) return toNorm(p, (m[1] === 'l' ? -1 : 1) * Number(m[2]) / 100);
  }
  const m = /^([+-]?\d*\.?\d+)\s*([a-z%]*)$/.exec(t);
  if (!m) return null;
  let v = Number(m[1]);
  const unit = m[2];
  if (p.unit === '%') v /= 100;
  if (p.unit === 'ms' && unit === 's') v *= 1000;
  if (p.unit === 'pan') v /= 100;
  return snap(p, toNorm(p, v));
}
