//! The oscillator types that play recordings (Sample, Multisample,
//! Granular, Spectral), per voice: their state, and the render that fills
//! an oscillator's buffer the way a wavetable would, so routing, filters
//! and the amp treat every type alike.

use wt_dsp::math;

use crate::filter::MAX_N;
use crate::osc::granular::{self, GrainPool, GrainSettings, GranularVoice};
use crate::osc::multi::{MAX_LAYERS, Multi};
use crate::osc::sample::{Head, LOOP_OFF, Recording, Region};
use crate::osc::spectral::{Analysis, BINS, SpecScratch, SpecSettings, SpectralVoice};
use crate::osc::unison::{Layout, MAX_LANES};
use crate::spec::params as p;
use crate::spec::protocol::{OSC_COUNT, VOICE_SLOTS};

pub const TYPE_WAVETABLE: u8 = 0;
pub const TYPE_SAMPLE: u8 = 1;
pub const TYPE_MULTI: u8 = 2;
pub const TYPE_GRANULAR: u8 = 3;
pub const TYPE_SPECTRAL: u8 = 4;

type Stereo = [[f32; MAX_N]; 2];

/// What the engine keeps for the recording types: the assets, and state every voice shares.
pub struct OscAssets {
    pub recs: [Option<Recording>; OSC_COUNT],
    pub multis: [Option<Multi>; OSC_COUNT],
    pub analyses: [Option<Analysis>; OSC_COUNT],
    pub spec_filter: [[f32; BINS]; OSC_COUNT],
}

impl Default for OscAssets {
    fn default() -> Self {
        OscAssets { recs: [None, None, None], multis: [None, None, None], analyses: [None, None, None], spec_filter: [[1.0; BINS]; OSC_COUNT] }
    }
}

pub struct OscShared {
    pub pool: GrainPool,
    /// One spectral voice per voice slot and oscillator (heap, made once).
    pub spec: Vec<SpectralVoice>,
    pub scratch: SpecScratch,
}

impl Default for OscShared {
    fn default() -> Self {
        OscShared { pool: GrainPool::default(), spec: (0..VOICE_SLOTS * OSC_COUNT).map(|_| SpectralVoice::default()).collect(), scratch: SpecScratch::default() }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Layer {
    zone: usize,
    head: Head,
    region: Region,
    gl: f32,
    gr: f32,
}

/// One voice's state for one oscillator's recording types.
#[derive(Clone, Copy, Debug, Default)]
pub struct SrcState {
    /// Set up on the next render (the note just started).
    pub fresh: bool,
    heads: [Head; MAX_LANES],
    region: Region,
    /// The settings the region came from (it's resolved again when they move).
    region_key: [f32; 7],
    slice: Option<(f64, f64)>,
    layers: [Layer; MAX_LAYERS],
    nlayers: usize,
    gran: GranularVoice,
    /// Where it is in its recording, 0..1, for the display (-1: nothing playing).
    pub play: f32,
}

/// Everything a recording type needs from the voice this sub-block.
pub struct SrcIn<'a> {
    pub kind: u8,
    pub osc: usize,
    pub slot: usize,
    pub note: u8,
    pub velocity: f32,
    pub held: bool,
    /// The note's pitch now (glide, bend, tuning), and the oscillator's own tuning in semitones.
    pub pitch: f32,
    pub tuning: f32,
    pub bend: f32,
    pub sr: f32,
    pub layout: &'a Layout,
    pub res: &'a dyn Fn(u16) -> f32,
}

impl SrcState {
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, x: &SrcIn, assets: &OscAssets, shared: &mut OscShared, start: usize, len: usize, out: &mut Stereo) {
        let res = x.res;
        let o = x.osc;
        let fresh = std::mem::take(&mut self.fresh);
        let clear = |out: &mut Stereo| {
            out[0][start..len].fill(0.0);
            out[1][start..len].fill(0.0);
        };
        let root = res(p::OSC_ROOT[o]);
        let key = res(p::OSC_KEYTRACK[o]) >= 0.5;
        // semitones from the recording's own pitch
        let semis = x.tuning + if key { x.pitch - root } else { x.bend };
        let ratio = math::semis_to_ratio(semis);
        match x.kind {
            TYPE_SAMPLE => {
                let Some(rec) = &assets.recs[o] else {
                    self.play = -1.0;
                    return clear(out);
                };
                let slicing = res(p::OSC_SLICE[o]) >= 0.5 && !rec.slices().is_empty();
                if fresh {
                    self.slice = None;
                    if slicing {
                        let s = rec.slices();
                        let i = ((x.note as i32 - root as i32).rem_euclid(s.len() as i32)) as usize;
                        let a = s[i] as f64;
                        let b = if res(p::OSC_TAIL[o]) >= 0.5 || i + 1 >= s.len() { rec.frames as f64 } else { s[i + 1] as f64 };
                        self.slice = Some((a, b.max(a + 2.0)));
                    }
                    self.region_key = [f32::NAN; 7];
                }
                let k = [res(p::OSC_SMP_START[o]), res(p::OSC_SMP_END[o]), res(p::OSC_LOOP_START[o]), res(p::OSC_LOOP_END[o]), res(p::OSC_LOOP_MODE[o]), res(p::OSC_XFADE[o]), res(p::OSC_SNAP[o])];
                if k != self.region_key {
                    self.region_key = k;
                    self.region = match self.slice {
                        Some((a, b)) => Region { start: a, end: b, loop_start: a, loop_end: b, mode: LOOP_OFF, xfade: 0.0 },
                        None => Region::resolve(rec, k[0], k[1], k[2], k[3], k[4] as u8, k[5], k[6] >= 0.5),
                    };
                }
                let lay = x.layout;
                if fresh {
                    for h in self.heads.iter_mut().take(lay.n) {
                        h.start(&self.region);
                    }
                }
                // slices play at the recording's pitch (with the osc's tuning)
                let r = if slicing { math::semis_to_ratio(x.tuning + x.bend) } else { ratio };
                let step = (r * res(p::OSC_RATE[o])) as f64 * rec.rate as f64 / x.sr as f64;
                clear(out);
                for l in 0..lay.n {
                    let h = &mut self.heads[l];
                    let st = step * lay.ratio[l] as f64;
                    let (gl, gr) = (lay.gl[l], lay.gr[l]);
                    for i in start..len {
                        let (a, b) = h.tick(rec, &self.region, st, x.held);
                        out[0][i] += a * gl;
                        out[1][i] += b * gr;
                    }
                }
                self.play = if self.heads[0].done { -1.0 } else { (self.heads[0].pos / rec.frames as f64) as f32 };
            }
            TYPE_MULTI => {
                let Some(m) = &assets.multis[o] else {
                    self.play = -1.0;
                    return clear(out);
                };
                if fresh {
                    let mut pick = [0usize; MAX_LAYERS];
                    self.nlayers = m.pick(x.note, (x.velocity * 127.0).round() as u8, &mut pick);
                    for (n, &z) in pick.iter().enumerate().take(self.nlayers) {
                        let zone = &m.zones[z];
                        let region = m.region(z);
                        let mut head = Head::default();
                        head.start(&region);
                        let (pl, pr) = math::balance((zone.pan + 1.0) * 0.5);
                        self.layers[n] = Layer { zone: z, head, region, gl: pl * zone.gain, gr: pr * zone.gain };
                    }
                }
                clear(out);
                let rate = res(p::OSC_RATE[o]);
                for layer in self.layers.iter_mut().take(self.nlayers) {
                    let zone = &m.zones[layer.zone];
                    let semis = x.tuning + x.pitch - zone.root;
                    let step = (math::semis_to_ratio(semis) * rate) as f64 * zone.rate as f64 / x.sr as f64;
                    let audio = m.audio(layer.zone);
                    let held = x.held || zone.one_shot;
                    for i in start..len {
                        let (a, b) = layer.head.tick(&audio, &layer.region, step, held);
                        out[0][i] += a * layer.gl;
                        out[1][i] += b * layer.gr;
                    }
                }
                self.play = if self.nlayers > 0 && !self.layers[0].head.done { (self.layers[0].head.pos / m.zones[self.layers[0].zone].frames as f64) as f32 } else { -1.0 };
            }
            TYPE_GRANULAR => {
                let Some(rec) = &assets.recs[o] else {
                    self.play = -1.0;
                    return clear(out);
                };
                if fresh {
                    shared.pool.free(granular::owner(x.slot, o));
                    self.gran.start((x.slot as u32 * 7919) ^ (x.note as u32 * 104_729) ^ 0x5bd1_e995);
                }
                let n = rec.frames as f64;
                let s = GrainSettings {
                    density: res(p::OSC_GR_DENSITY[o]),
                    length_ms: res(p::OSC_GR_LENGTH[o]),
                    position: res(p::OSC_GR_POSITION[o]),
                    scan: res(p::OSC_GR_SCAN[o]),
                    spray: res(p::OSC_GR_SPRAY[o]),
                    pitch_rand: res(p::OSC_GR_PITCH_RAND[o]),
                    pan: res(p::OSC_GR_PAN[o]),
                    window: res(p::OSC_GR_WINDOW[o]) as u8,
                    direction: res(p::OSC_GR_DIRECTION[o]) as u8,
                    sync: res(p::OSC_GR_SYNC[o]) >= 0.5,
                    timbre: res(p::OSC_GR_TIMBRE[o]),
                    looped: res(p::OSC_GR_LOOP[o]) >= 0.5,
                    step: (ratio * res(p::OSC_RATE[o])) as f64 * rec.rate as f64 / x.sr as f64,
                    hz: math::note_to_hz((x.pitch + x.tuning) as f64) as f32,
                    region: (res(p::OSC_SMP_START[o]) as f64 * n, (res(p::OSC_SMP_END[o]) as f64 * n).max(res(p::OSC_SMP_START[o]) as f64 * n + 2.0)),
                };
                self.gran.render(granular::owner(x.slot, o), &mut shared.pool, rec, &s, x.sr, start, len, out);
                self.play = (self.gran.pos / n) as f32;
            }
            TYPE_SPECTRAL => {
                let Some(an) = &assets.analyses[o] else {
                    self.play = -1.0;
                    return clear(out);
                };
                let v = &mut shared.spec[x.slot * OSC_COUNT + o];
                if fresh {
                    v.start();
                }
                let s = SpecSettings {
                    position: res(p::OSC_SP_POSITION[o]),
                    scan: res(p::OSC_SP_SCAN[o]),
                    ratio,
                    low: res(p::OSC_SP_LOW[o]),
                    high: res(p::OSC_SP_HIGH[o]),
                    warp: res(p::OSC_SP_WARP[o]) as u8,
                    amount: res(p::OSC_SP_WARP_AMT[o]),
                    looped: res(p::OSC_SP_LOOP[o]) >= 0.5,
                    hz: math::note_to_hz((x.pitch + x.tuning) as f64) as f32,
                    filter: res(p::OSC_SP_FILTER[o]) >= 0.5,
                };
                v.render(&mut shared.scratch, an, &assets.spec_filter[o], &s, x.sr, start, len, out);
                self.play = v.position(an);
            }
            _ => clear(out),
        }
    }

    /// The voice stopped: its grains stop with it.
    pub fn stop(&mut self, slot: usize, osc: usize, shared: &mut OscShared) {
        shared.pool.free(granular::owner(slot, osc));
        self.play = -1.0;
    }
}
