//! "Edit" style frame effects. Everything works on packed RGB24 buffers, and every
//! effect is scaled by the drive signal (bass pulse when synced).

/// What the effects react to. `pulse` jumps on a beat and decays, `level` follows bass energy.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drive {
    pub pulse: f32,
    pub level: f32,
    pub beat: bool,
    pub strength: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fx {
    Zoom,
    Shake,
    Split,
    Blur,
    Red,
    Glitch,
    Trails,
    Invert,
    Vhs,
    Pixel,
    Mirror,
    Strobe,
}

impl Fx {
    pub const ALL: [Fx; 12] = [
        Fx::Zoom,
        Fx::Shake,
        Fx::Split,
        Fx::Blur,
        Fx::Red,
        Fx::Glitch,
        Fx::Trails,
        Fx::Invert,
        Fx::Vhs,
        Fx::Pixel,
        Fx::Mirror,
        Fx::Strobe,
    ];
    const KEYS: [char; 12] = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0', '-', '='];

    pub fn key(self) -> char {
        Self::KEYS[self as usize]
    }

    pub fn from_key(c: char) -> Option<Fx> {
        Self::KEYS
            .iter()
            .position(|&k| k == c)
            .map(|i| Self::ALL[i])
    }

    pub fn label(self) -> &'static str {
        match self {
            Fx::Zoom => "zoom",
            Fx::Shake => "shake",
            Fx::Split => "rgb",
            Fx::Blur => "blur",
            Fx::Red => "red",
            Fx::Glitch => "glitch",
            Fx::Trails => "trails",
            Fx::Invert => "invert",
            Fx::Vhs => "vhs",
            Fx::Pixel => "pixel",
            Fx::Mirror => "mirror",
            Fx::Strobe => "strobe",
        }
    }
}

pub struct Preset {
    pub name: &'static str,
    pub fx: &'static [Fx],
}

pub const PRESETS: &[Preset] = &[
    Preset {
        name: "edit",
        fx: &[Fx::Zoom, Fx::Shake, Fx::Split, Fx::Glitch, Fx::Vhs],
    },
    Preset {
        name: "phonk red",
        fx: &[
            Fx::Zoom,
            Fx::Shake,
            Fx::Red,
            Fx::Invert,
            Fx::Strobe,
            Fx::Blur,
        ],
    },
    Preset {
        name: "glitchcore",
        fx: &[
            Fx::Split,
            Fx::Glitch,
            Fx::Pixel,
            Fx::Trails,
            Fx::Vhs,
            Fx::Shake,
        ],
    },
    Preset {
        name: "dream",
        fx: &[Fx::Blur, Fx::Trails, Fx::Zoom, Fx::Mirror],
    },
    Preset {
        name: "clean",
        fx: &[],
    },
];

/// Tiny xorshift RNG; effects only need fast, cheap noise.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform in 0..1.
    pub fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in -1..1.
    pub fn signed(&mut self) -> f32 {
        self.f32() * 2.0 - 1.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

pub struct Effects {
    enabled: [bool; Fx::ALL.len()],
    pub intensity: f32,
    rng: Rng,
    time: f32,
    prev: Vec<u8>,
    tmp: Vec<u8>,
    glitch_left: f32,
    glitch_seed: u64,
    invert_left: f32,
    red_lut: [[u8; 3]; 256],
}

impl Effects {
    pub fn new(preset: &Preset) -> Self {
        let mut e = Effects {
            enabled: [false; Fx::ALL.len()],
            intensity: 1.0,
            rng: Rng::new(0x9e37_79b9_7f4a_7c15),
            time: 0.0,
            prev: Vec::new(),
            tmp: Vec::new(),
            glitch_left: 0.0,
            glitch_seed: 1,
            invert_left: 0.0,
            red_lut: red_lut(),
        };
        e.apply_preset(preset);
        e
    }

    pub fn apply_preset(&mut self, preset: &Preset) {
        self.enabled = [false; Fx::ALL.len()];
        for &fx in preset.fx {
            self.enabled[fx as usize] = true;
        }
    }

    pub fn is_on(&self, fx: Fx) -> bool {
        self.enabled[fx as usize]
    }

    pub fn toggle(&mut self, fx: Fx) {
        self.enabled[fx as usize] ^= true;
    }

    /// Forget per-video state (trail history, running glitches).
    pub fn reset(&mut self) {
        self.prev.clear();
        self.glitch_left = 0.0;
        self.invert_left = 0.0;
    }

    pub fn render(&mut self, src: &[u8], w: usize, h: usize, d: Drive, dt: f32, out: &mut Vec<u8>) {
        let n = w * h * 3;
        if w == 0 || h == 0 || src.len() < n {
            return;
        }
        out.resize(n, 0);
        self.tmp.resize(n, 0);
        if self.prev.len() != n {
            self.prev.clear();
            self.prev.extend_from_slice(&src[..n]);
        }
        self.time += dt;

        let on = self.enabled;
        let on = |fx: Fx| on[fx as usize];
        let i = self.intensity;
        let p = (d.pulse * i).min(2.0);
        // Effects were tuned at 200 px wide; distances in pixels scale with the resolution.
        let px = (w as f32 / 200.0).max(1.0);

        if d.beat {
            if on(Fx::Glitch) && self.rng.f32() < 0.4 + 0.6 * d.strength {
                self.glitch_left = 0.08 + 0.18 * self.rng.f32();
                self.glitch_seed = self.rng.next();
            }
            if on(Fx::Invert) && d.strength > 0.5 {
                self.invert_left = 0.07;
            }
        }
        self.glitch_left -= dt;
        self.invert_left -= dt;

        // Geometry: zoom punch, camera shake and mirror in one resampling pass.
        let zoom = if on(Fx::Zoom) {
            1.0 + 0.2 * p + 0.04 * d.level * i
        } else {
            1.0
        };
        let (dx, dy) = if on(Fx::Shake) {
            (
                self.rng.signed() * w as f32 * 0.035 * p,
                self.rng.signed() * h as f32 * 0.035 * p,
            )
        } else {
            (0.0, 0.0)
        };
        geometry(&src[..n], out, w, h, zoom, dx, dy, on(Fx::Mirror));

        if on(Fx::Trails) {
            // Smear into the previous output, but let beats cut through sharply.
            let k = (0.55 * i).min(0.85) * (1.0 - 0.6 * d.pulse.min(1.0));
            blend(out, &self.prev, k);
        }
        self.prev.copy_from_slice(out);

        if on(Fx::Blur) {
            let radius = ((0.6 + 4.0 * p) * px).round() as usize;
            box_blur(out, &mut self.tmp, w, h, radius.min((10.0 * px) as usize));
        }
        if on(Fx::Split) {
            let off = ((0.5 + 6.0 * p) * px).round() as isize;
            rgb_split(out, &mut self.tmp, w, h, off);
        }
        if on(Fx::Red) {
            let amount = ((0.55 + 0.45 * d.pulse) * i.min(1.0)).clamp(0.0, 1.0);
            red_grade(out, w, h, &self.red_lut, amount);
        }
        if on(Fx::Glitch) && self.glitch_left > 0.0 {
            glitch(
                out,
                &mut self.tmp,
                w,
                h,
                self.glitch_seed,
                &mut self.rng,
                i.min(1.5),
            );
        }
        if on(Fx::Pixel) {
            let block = ((2.0 + 6.0 * p) * px).round() as usize;
            pixelate(out, w, h, block);
        }
        if on(Fx::Vhs) {
            vhs(out, &mut self.tmp, w, h, self.time, p, i, px, &mut self.rng);
        }
        if on(Fx::Strobe) {
            let gain = 1.0 + 1.4 * p;
            for v in out.iter_mut() {
                *v = (*v as f32 * gain).min(255.0) as u8;
            }
        }
        if on(Fx::Invert) && self.invert_left > 0.0 {
            for v in out.iter_mut() {
                *v = 255 - *v;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn geometry(
    src: &[u8],
    out: &mut [u8],
    w: usize,
    h: usize,
    zoom: f32,
    dx: f32,
    dy: f32,
    mirror: bool,
) {
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let inv = 1.0 / zoom;
    let (wmax, hmax) = ((w - 1) as f32, (h - 1) as f32);
    for y in 0..h {
        let sy = ((y as f32 - cy) * inv + cy + dy).clamp(0.0, hmax) as usize;
        for x in 0..w {
            let xm = if mirror && x >= w / 2 { w - 1 - x } else { x };
            let sx = ((xm as f32 - cx) * inv + cx + dx).clamp(0.0, wmax) as usize;
            let s = (sy * w + sx) * 3;
            let o = (y * w + x) * 3;
            out[o..o + 3].copy_from_slice(&src[s..s + 3]);
        }
    }
}

fn blend(out: &mut [u8], prev: &[u8], k: f32) {
    let k = (k.clamp(0.0, 1.0) * 256.0) as u32;
    for (o, &p) in out.iter_mut().zip(prev) {
        *o = ((*o as u32 * (256 - k) + p as u32 * k) >> 8) as u8;
    }
}

/// Separable box blur: horizontal pass into `tmp`, vertical pass back into `buf`.
fn box_blur(buf: &mut [u8], tmp: &mut [u8], w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let div = (2 * r + 1) as u32;
    let ri = r as isize;

    for y in 0..h {
        let row = y * w;
        let at = |x: isize| (row + x.clamp(0, w as isize - 1) as usize) * 3;
        for c in 0..3 {
            let mut sum: u32 = (-ri..=ri).map(|k| buf[at(k) + c] as u32).sum();
            for x in 0..w as isize {
                tmp[at(x) + c] = (sum / div) as u8;
                sum += buf[at(x + ri + 1) + c] as u32;
                sum -= buf[at(x - ri) + c] as u32;
            }
        }
    }
    for x in 0..w {
        let at = |y: isize| (y.clamp(0, h as isize - 1) as usize * w + x) * 3;
        for c in 0..3 {
            let mut sum: u32 = (-ri..=ri).map(|k| tmp[at(k) + c] as u32).sum();
            for y in 0..h as isize {
                buf[at(y) + c] = (sum / div) as u8;
                sum += tmp[at(y + ri + 1) + c] as u32;
                sum -= tmp[at(y - ri) + c] as u32;
            }
        }
    }
}

/// Chromatic aberration: red shifted one way, blue the other.
fn rgb_split(out: &mut [u8], tmp: &mut [u8], w: usize, h: usize, off: isize) {
    tmp.copy_from_slice(out);
    let voff = off / 3;
    let wi = w as isize - 1;
    let hi = h as isize - 1;
    for y in 0..h {
        let yr = (y as isize + voff).clamp(0, hi) as usize;
        let yb = (y as isize - voff).clamp(0, hi) as usize;
        for x in 0..w {
            let xr = (x as isize + off).clamp(0, wi) as usize;
            let xb = (x as isize - off).clamp(0, wi) as usize;
            let o = (y * w + x) * 3;
            out[o] = tmp[(yr * w + xr) * 3];
            out[o + 2] = tmp[(yb * w + xb) * 3 + 2];
        }
    }
}

/// Luminance -> black/blood-red/white duotone, plus a vignette.
fn red_lut() -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    for (i, e) in lut.iter_mut().enumerate() {
        let l = i as f32 / 255.0;
        let r = (l.powf(0.65) * 1.1).min(1.0);
        let g = l.powf(2.4) * 0.55;
        let b = l.powf(2.6) * 0.45;
        *e = [(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8];
    }
    lut
}

fn red_grade(out: &mut [u8], w: usize, h: usize, lut: &[[u8; 3]; 256], amount: f32) {
    let mix = (amount * 256.0) as u32;
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    for y in 0..h {
        let ny = (y as f32 - cy) / cy;
        for x in 0..w {
            let nx = (x as f32 - cx) / cx;
            let vignette = (1.0 - 0.55 * (nx * nx + ny * ny)).max(0.0);
            let o = (y * w + x) * 3;
            let px = &mut out[o..o + 3];
            let l = (px[0] as u32 * 77 + px[1] as u32 * 150 + px[2] as u32 * 29) >> 8;
            let t = lut[l as usize];
            for c in 0..3 {
                let v = (px[c] as u32 * (256 - mix) + t[c] as u32 * mix) >> 8;
                px[c] = (v as f32 * vignette) as u8;
            }
        }
    }
}

/// Datamosh-ish corruption: displaced slices with swapped channels, moved blocks, noise rows.
/// Layout comes from `seed` so a glitch holds for a few frames; `jitter` adds flicker.
fn glitch(
    out: &mut [u8],
    tmp: &mut [u8],
    w: usize,
    h: usize,
    seed: u64,
    jitter: &mut Rng,
    amount: f32,
) {
    tmp.copy_from_slice(out);
    let mut g = Rng::new(seed);

    for _ in 0..3 + g.below(6) {
        let y0 = g.below(h);
        let height = 1 + g.below(h / 5 + 1);
        let shift = ((g.signed() * 0.3 + jitter.signed() * 0.03) * w as f32 * amount) as isize;
        let perm = g.below(3);
        for y in y0..(y0 + height).min(h) {
            for x in 0..w {
                let sx = (x as isize - shift).rem_euclid(w as isize) as usize;
                let s = (y * w + sx) * 3;
                let (r, gg, b) = (tmp[s], tmp[s + 1], tmp[s + 2]);
                let px = match perm {
                    0 => [r, gg, b],
                    1 => [gg, b, r],
                    _ => [b, r, gg],
                };
                let o = (y * w + x) * 3;
                out[o..o + 3].copy_from_slice(&px);
            }
        }
    }

    for _ in 0..g.below(4) {
        let bw = (w / 10).max(1) + g.below(w / 5 + 1);
        let bh = (h / 12).max(1) + g.below(h / 6 + 1);
        let (sx, sy) = (g.below(w), g.below(h));
        let (dx, dy) = (g.below(w), g.below(h));
        for yy in 0..bh {
            for xx in 0..bw {
                let (x0, y0, x1, y1) = (sx + xx, sy + yy, dx + xx, dy + yy);
                if x0 < w && y0 < h && x1 < w && y1 < h {
                    let s = (y0 * w + x0) * 3;
                    let o = (y1 * w + x1) * 3;
                    out[o..o + 3].copy_from_slice(&tmp[s..s + 3]);
                }
            }
        }
    }

    let thick = (w / 200).max(1);
    for _ in 0..g.below(3) * thick {
        let y = jitter.below(h);
        for x in 0..w {
            let v = jitter.below(256) as u8;
            let o = (y * w + x) * 3;
            out[o..o + 3].copy_from_slice(&[v, v / 4, v / 2]);
        }
    }
}

fn pixelate(out: &mut [u8], w: usize, h: usize, block: usize) {
    if block < 2 {
        return;
    }
    for by in (0..h).step_by(block) {
        for bx in (0..w).step_by(block) {
            let cy = (by + block / 2).min(h - 1);
            let cx = (bx + block / 2).min(w - 1);
            let s = (cy * w + cx) * 3;
            let px = [out[s], out[s + 1], out[s + 2]];
            for y in by..(by + block).min(h) {
                for x in bx..(bx + block).min(w) {
                    let o = (y * w + x) * 3;
                    out[o..o + 3].copy_from_slice(&px);
                }
            }
        }
    }
}

/// Tape look: wobbling rows, a rolling tracking band, scanlines and grain.
#[allow(clippy::too_many_arguments)]
fn vhs(
    out: &mut [u8],
    tmp: &mut [u8],
    w: usize,
    h: usize,
    time: f32,
    p: f32,
    intensity: f32,
    px: f32,
    rng: &mut Rng,
) {
    tmp.copy_from_slice(out);
    let band = ((time * 0.23).fract() * h as f32) as isize;
    let band_half = (3.0 * px) as isize;
    let line = (px * 0.5).round().max(1.0) as usize;
    let grain = 10.0 * intensity;
    for y in 0..h {
        let mut off = (time * 9.0 + y as f32 * 0.21 / px).sin() * (0.4 + 1.5 * p) * px;
        let in_band = (y as isize - band).abs() < band_half;
        if in_band {
            off += rng.signed() * 6.0 * intensity * px;
        }
        let off = off.round() as isize;
        let scan = if (y / line) % 2 == 1 { 0.82 } else { 1.0 };
        let lift = if in_band { 25.0 } else { 0.0 };
        for x in 0..w {
            let sx = (x as isize - off).clamp(0, w as isize - 1) as usize;
            let s = (y * w + sx) * 3;
            let o = (y * w + x) * 3;
            let noise = rng.signed() * grain + lift;
            for c in 0..3 {
                out[o + c] = (tmp[s + c] as f32 * scan + noise).clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_effects_survive_odd_sizes_and_extreme_drive() {
        let mut fx = Effects::new(&PRESETS[0]);
        for f in Fx::ALL {
            if !fx.is_on(f) {
                fx.toggle(f);
            }
        }
        fx.intensity = 2.0;
        let mut out = Vec::new();
        for &(w, h) in &[(2, 2), (3, 2), (17, 5), (101, 57), (200, 112)] {
            let src: Vec<u8> = (0..w * h * 3).map(|i| (i * 37 % 256) as u8).collect();
            for step in 0..40 {
                let d = Drive {
                    pulse: (step % 5) as f32 * 0.5,
                    level: 1.0,
                    beat: step % 3 == 0,
                    strength: 1.0,
                };
                fx.render(&src, w, h, d, 1.0 / 30.0, &mut out);
                assert_eq!(out.len(), w * h * 3);
            }
        }
    }

    #[test]
    fn clean_preset_is_identity() {
        let mut fx = Effects::new(PRESETS.iter().find(|p| p.fx.is_empty()).unwrap());
        let src: Vec<u8> = (0..40 * 20 * 3).map(|i| (i % 251) as u8).collect();
        let mut out = Vec::new();
        fx.render(
            &src,
            40,
            20,
            Drive {
                pulse: 1.0,
                level: 1.0,
                beat: true,
                strength: 1.0,
            },
            0.03,
            &mut out,
        );
        assert_eq!(out, src);
    }
}

#[cfg(test)]
mod preview {
    use super::*;

    /// Writes one PPM per preset, mid-beat: `YTFX_FRAME=frame.rgb YTFX_OUT=dir cargo test preview -- --ignored`
    /// (frame.rgb is a raw 200x112 RGB24 frame).
    #[test]
    #[ignore]
    fn preview() {
        let src = std::fs::read(std::env::var("YTFX_FRAME").unwrap()).unwrap();
        let dir = std::env::var("YTFX_OUT").unwrap();
        let (w, h) = (200, 112);
        // Pulse as the analyzer produces it: a hit, then exponential decay.
        let timeline = [(true, 1.0), (false, 0.5), (false, 0.06)];
        for preset in PRESETS {
            let mut fx = Effects::new(preset);
            let mut out = Vec::new();
            for (i, &(beat, pulse)) in timeline.iter().enumerate() {
                let d = Drive {
                    pulse,
                    level: pulse.max(0.3),
                    beat,
                    strength: 0.6,
                };
                fx.render(&src, w, h, d, if i == 2 { 0.3 } else { 0.1 }, &mut out);
                let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
                ppm.extend_from_slice(&out);
                std::fs::write(
                    format!("{dir}/{}_{i}.ppm", preset.name.replace(' ', "_")),
                    ppm,
                )
                .unwrap();
            }
        }
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use std::time::Instant;

    /// `cargo test --release bench_all_effects -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_all_effects() {
        for &(w, h) in &[
            (200, 112),
            (640, 360),
            (854, 480),
            (1280, 720),
            (1920, 1080),
        ] {
            let mut fx = Effects::new(&PRESETS[0]);
            for f in Fx::ALL {
                if !fx.is_on(f) {
                    fx.toggle(f);
                }
            }
            let src: Vec<u8> = (0..w * h * 3).map(|i| (i * 37 % 256) as u8).collect();
            let mut out = Vec::new();
            let n = 30;
            let t = Instant::now();
            for k in 0..n {
                let d = Drive {
                    pulse: 0.8,
                    level: 0.8,
                    beat: k % 4 == 0,
                    strength: 0.8,
                };
                fx.render(&src, w, h, d, 1.0 / 30.0, &mut out);
            }
            println!(
                "{w}x{h}: {:.1} ms/frame (all 12 effects)",
                t.elapsed().as_secs_f64() * 1000.0 / n as f64
            );
        }
    }
}
