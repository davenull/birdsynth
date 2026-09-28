//! Multisamples: recordings spread across the keyboard and velocity, as an
//! SFZ file describes them (the host parses the SFZ and packs the zones).
//!
//! The asset starts with one 16-float header per zone, then every zone's
//! audio (planar channels):
//!
//! | # | field                          | # | field                       |
//! |---|--------------------------------|---|-----------------------------|
//! | 0 | audio offset (floats)          | 8 | highest velocity (0..127)   |
//! | 1 | frames                         | 9 | loop mode (as the Sample type) |
//! | 2 | channels (1 or 2)              | 10 | loop start (frames)        |
//! | 3 | sample rate                    | 11 | loop end (frames)          |
//! | 4 | root note (fractional: tune)   | 12 | gain (linear)              |
//! | 5 | lowest key                     | 13 | pan (-1..1)                |
//! | 6 | highest key                    | 14 | 1: plays to its end after release |
//! | 7 | lowest velocity                | 15 | (unused)                   |

use super::sample::{Audio, Region, hermite};
use crate::spec::protocol::MAX_ZONES;
use crate::tables::AssetBuf;

pub const ZONE_FLOATS: usize = 16;
/// Zones one note can layer.
pub const MAX_LAYERS: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Zone {
    pub offset: usize,
    pub frames: usize,
    pub channels: usize,
    pub rate: f32,
    pub root: f32,
    pub keys: (u8, u8),
    pub vels: (u8, u8),
    pub loop_mode: u8,
    pub loop_start: f64,
    pub loop_end: f64,
    pub gain: f32,
    pub pan: f32,
    pub one_shot: bool,
}

pub struct Multi {
    buf: AssetBuf,
    pub zones: [Zone; MAX_ZONES],
    pub count: usize,
    audio_at: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BadMulti;

impl Multi {
    pub fn new(buf: AssetBuf, zones: usize) -> Result<Multi, BadMulti> {
        if zones == 0 || zones > MAX_ZONES {
            return Err(BadMulti);
        }
        let head = zones * ZONE_FLOATS;
        let f = buf.as_f32();
        if f.len() < head {
            return Err(BadMulti);
        }
        let audio = f.len() - head;
        let mut out = [Zone::default(); MAX_ZONES];
        for (z, o) in out.iter_mut().enumerate().take(zones) {
            let h = &f[z * ZONE_FLOATS..(z + 1) * ZONE_FLOATS];
            let frames = h[1] as usize;
            let channels = h[2] as usize;
            let offset = h[0] as usize;
            // (a NaN rate fails `> 0.0` too)
            if frames < 4 || !(1..=2).contains(&channels) || offset + frames * channels > audio || h[3].is_nan() || h[3] <= 0.0 {
                return Err(BadMulti);
            }
            let n = frames as f64;
            let ls = (h[10] as f64).clamp(0.0, n - 2.0);
            *o = Zone {
                offset,
                frames,
                channels,
                rate: h[3],
                root: h[4],
                keys: (h[5].clamp(0.0, 127.0) as u8, h[6].clamp(0.0, 127.0) as u8),
                vels: (h[7].clamp(0.0, 127.0) as u8, h[8].clamp(0.0, 127.0) as u8),
                loop_mode: h[9] as u8,
                loop_start: ls,
                loop_end: (h[11] as f64).clamp(ls + 2.0, n),
                gain: h[12],
                pan: h[13].clamp(-1.0, 1.0),
                one_shot: h[14] >= 0.5,
            };
        }
        Ok(Multi { buf, zones: out, count: zones, audio_at: head })
    }

    /// The zones a note plays (up to MAX_LAYERS), by key and velocity (0..127).
    pub fn pick(&self, key: u8, vel: u8, out: &mut [usize; MAX_LAYERS]) -> usize {
        let mut n = 0;
        for (i, z) in self.zones[..self.count].iter().enumerate() {
            if key >= z.keys.0 && key <= z.keys.1 && vel >= z.vels.0 && vel <= z.vels.1 && n < MAX_LAYERS {
                out[n] = i;
                n += 1;
            }
        }
        // no zone covers the key: take the nearest one (by root) in velocity range
        if n == 0 && self.count > 0 {
            let best = (0..self.count)
                .filter(|&i| vel >= self.zones[i].vels.0 && vel <= self.zones[i].vels.1)
                .min_by(|&a, &b| (self.zones[a].root - key as f32).abs().total_cmp(&(self.zones[b].root - key as f32).abs()))
                .or_else(|| (0..self.count).min_by(|&a, &b| (self.zones[a].root - key as f32).abs().total_cmp(&(self.zones[b].root - key as f32).abs())));
            if let Some(b) = best {
                out[0] = b;
                n = 1;
            }
        }
        n
    }

    pub fn audio(&self, z: usize) -> ZoneAudio<'_> {
        let zone = &self.zones[z];
        let f = self.buf.as_f32();
        let a = self.audio_at + zone.offset;
        ZoneAudio { l: &f[a..a + zone.frames], r: &f[a + (zone.channels - 1) * zone.frames..a + zone.channels * zone.frames] }
    }

    /// Where zone `z` plays: all of it, with its own loop.
    pub fn region(&self, z: usize) -> Region {
        let zone = &self.zones[z];
        Region { start: 0.0, end: zone.frames as f64, loop_start: zone.loop_start, loop_end: zone.loop_end, mode: zone.loop_mode, xfade: 0.0 }
    }
}

pub struct ZoneAudio<'a> {
    l: &'a [f32],
    r: &'a [f32],
}

impl Audio for ZoneAudio<'_> {
    #[inline]
    fn read_at(&self, _step: f64, pos: f64) -> (f32, f32) {
        (hermite(self.l, pos), hermite(self.r, pos))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(zones: &[[f32; ZONE_FLOATS]], audio: usize) -> AssetBuf {
        let mut b = AssetBuf::alloc((zones.len() * ZONE_FLOATS + audio) * 4).unwrap();
        for (i, z) in zones.iter().enumerate() {
            b.as_f32_mut()[i * ZONE_FLOATS..(i + 1) * ZONE_FLOATS].copy_from_slice(z);
        }
        b
    }

    fn zone(offset: f32, root: f32, keys: (f32, f32), vels: (f32, f32)) -> [f32; ZONE_FLOATS] {
        [offset, 100.0, 1.0, 48_000.0, root, keys.0, keys.1, vels.0, vels.1, 0.0, 0.0, 100.0, 1.0, 0.0, 0.0, 0.0]
    }

    #[test]
    fn picks_zones_by_key_and_velocity() {
        let m = Multi::new(asset(&[zone(0.0, 48.0, (0.0, 54.0), (0.0, 127.0)), zone(100.0, 60.0, (55.0, 66.0), (0.0, 63.0)), zone(200.0, 60.0, (55.0, 66.0), (64.0, 127.0)), zone(300.0, 72.0, (67.0, 80.0), (0.0, 127.0))], 400), 4).unwrap();
        let mut out = [0; MAX_LAYERS];
        assert_eq!((m.pick(50, 100, &mut out), out[0]), (1, 0));
        assert_eq!((m.pick(60, 30, &mut out), out[0]), (1, 1));
        assert_eq!((m.pick(60, 100, &mut out), out[0]), (1, 2));
        assert_eq!((m.pick(100, 100, &mut out), out[0]), (1, 3), "past the top: the nearest root");
        assert!(Multi::new(asset(&[zone(390.0, 60.0, (0.0, 127.0), (0.0, 127.0))], 400), 1).is_err(), "audio past the end");
    }
}
