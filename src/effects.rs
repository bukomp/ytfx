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

    pub fn from_label(label: &str) -> Option<Fx> {
        Self::ALL.into_iter().find(|fx| fx.label() == label)
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

/// Length of the VHS grain table (more than one 1080p row, so rows don't repeat visibly).
const NOISE_LEN: usize = 8192;

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
    /// Vignette strength per pixel (0..=256) for the red grade, cached per frame size.
    vignette: Vec<u16>,
    vignette_size: (usize, usize),
    /// Grain values for VHS, read at random offsets instead of a fresh random per pixel.
    noise: Vec<i16>,
    /// Running column sums for the blur's vertical pass.
    sums: Vec<u32>,
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
            vignette: Vec::new(),
            vignette_size: (0, 0),
            noise: {
                let mut r = Rng::new(0x51f1_5eed);
                (0..NOISE_LEN).map(|_| (r.signed() * 256.0) as i16).collect()
            },
            sums: Vec::new(),
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

    pub fn enabled(&self) -> Vec<Fx> {
        Fx::ALL.into_iter().filter(|&fx| self.is_on(fx)).collect()
    }

    pub fn set_enabled(&mut self, fxs: &[Fx]) {
        self.enabled = [false; Fx::ALL.len()];
        for &fx in fxs {
            self.enabled[fx as usize] = true;
        }
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
            let radius = radius.min((10.0 * px) as usize);
            box_blur(out, &mut self.tmp, &mut self.sums, w, h, radius);
        }
        if on(Fx::Split) {
            let off = ((0.5 + 6.0 * p) * px).round() as isize;
            rgb_split(out, &mut self.tmp, w, h, off);
        }
        if on(Fx::Red) {
            let amount = ((0.55 + 0.45 * d.pulse) * i.min(1.0)).clamp(0.0, 1.0);
            if self.vignette_size != (w, h) {
                self.vignette = vignette(w, h);
                self.vignette_size = (w, h);
            }
            red_grade(out, &self.vignette, &self.red_lut, amount);
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
            vhs(out, w, h, self.time, p, i, px, &self.noise, &mut self.rng);
        }
        if on(Fx::Strobe) {
            let gain = 1.0 + 1.4 * p;
            let lut: [u8; 256] = std::array::from_fn(|v| (v as f32 * gain).min(255.0) as u8);
            for v in out.iter_mut() {
                *v = lut[*v as usize];
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
    let identity_x = !mirror && (zoom - 1.0).abs() < 1e-4 && dx.round() == 0.0;
    // Source column of every output column (in bytes), shared by all rows.
    let cols: Vec<usize> = if identity_x {
        Vec::new()
    } else {
        (0..w)
            .map(|x| {
                let xm = if mirror && x >= w / 2 { w - 1 - x } else { x };
                ((xm as f32 - cx) * inv + cx + dx).clamp(0.0, wmax) as usize * 3
            })
            .collect()
    };
    let row = w * 3;
    for (y, out_row) in out.chunks_exact_mut(row).enumerate().take(h) {
        let sy = ((y as f32 - cy) * inv + cy + dy).clamp(0.0, hmax) as usize;
        let src_row = &src[sy * row..sy * row + row];
        if identity_x {
            out_row.copy_from_slice(src_row);
        } else {
            for (o, &s) in out_row.chunks_exact_mut(3).zip(&cols) {
                o.copy_from_slice(&src_row[s..s + 3]);
            }
        }
    }
}

fn blend(out: &mut [u8], prev: &[u8], k: f32) {
    let k = (k.clamp(0.0, 1.0) * 256.0) as u32;
    for (o, &p) in out.iter_mut().zip(prev) {
        *o = ((*o as u32 * (256 - k) + p as u32 * k) >> 8) as u8;
    }
}

/// Separable box blur with running sums. Both passes walk whole rows, which keeps memory
/// access sequential (a column-by-column vertical pass is several times slower).
fn box_blur(buf: &mut [u8], tmp: &mut [u8], sums: &mut Vec<u32>, w: usize, h: usize, r: usize) {
    if r == 0 {
        return;
    }
    let row = w * 3;
    let div = (2 * r + 1) as u32;
    // x / div as a multiply and shift.
    let recip = (1u32 << 16).div_ceil(div);

    // Horizontal: buf -> tmp, one row at a time, all three channels together.
    for (src, dst) in buf.chunks_exact(row).zip(tmp.chunks_exact_mut(row)).take(h) {
        let at = |x: isize| x.clamp(0, w as isize - 1) as usize * 3;
        let mut sum = [0u32; 3];
        for k in -(r as isize)..=r as isize {
            let i = at(k);
            for c in 0..3 {
                sum[c] += src[i + c] as u32;
            }
        }
        for x in 0..w {
            let o = x * 3;
            let (add, sub) = (at(x as isize + r as isize + 1), at(x as isize - r as isize));
            for c in 0..3 {
                dst[o + c] = ((sum[c] * recip) >> 16) as u8;
                sum[c] = sum[c] + src[add + c] as u32 - src[sub + c] as u32;
            }
        }
    }

    // Vertical: tmp -> buf, keeping a running sum per column.
    sums.clear();
    sums.resize(row, 0);
    let line = |y: isize| {
        let y = y.clamp(0, h as isize - 1) as usize;
        &tmp[y * row..y * row + row]
    };
    for k in -(r as isize)..=r as isize {
        for (s, &v) in sums.iter_mut().zip(line(k)) {
            *s += v as u32;
        }
    }
    for y in 0..h {
        let dst = &mut buf[y * row..y * row + row];
        for (d, &s) in dst.iter_mut().zip(sums.iter()) {
            *d = ((s * recip) >> 16) as u8;
        }
        let (add, sub) = (line(y as isize + r as isize + 1), line(y as isize - r as isize));
        for ((s, &a), &b) in sums.iter_mut().zip(add).zip(sub) {
            *s = *s + a as u32 - b as u32;
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

/// Per-pixel vignette factor, 256 = untouched.
fn vignette(w: usize, h: usize) -> Vec<u16> {
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    (0..h)
        .flat_map(|y| {
            let ny = (y as f32 - cy) / cy;
            (0..w).map(move |x| {
                let nx = (x as f32 - cx) / cx;
                ((1.0 - 0.55 * (nx * nx + ny * ny)).max(0.0) * 256.0) as u16
            })
        })
        .collect()
}

fn red_grade(out: &mut [u8], vignette: &[u16], lut: &[[u8; 3]; 256], amount: f32) {
    let mix = (amount * 256.0) as u32;
    for (px, &v) in out.chunks_exact_mut(3).zip(vignette) {
        let l = (px[0] as u32 * 77 + px[1] as u32 * 150 + px[2] as u32 * 29) >> 8;
        let t = lut[l as usize];
        for c in 0..3 {
            let graded = (px[c] as u32 * (256 - mix) + t[c] as u32 * mix) >> 8;
            px[c] = ((graded * v as u32) >> 8) as u8;
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
        // Mostly plain displacement; a channel swap only now and then.
        let perm = if g.below(8) == 0 { 1 + g.below(2) } else { 0 };
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

    // Small displaced blocks: about 3-11% of the width and 2-9% of the height each.
    for _ in 0..g.below(7) {
        let bw = (w / 32).max(1) + g.below(w / 12 + 1);
        let bh = (h / 48).max(1) + g.below(h / 14 + 1);
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
            out[o..o + 3].copy_from_slice(&[v, v, v]);
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
    w: usize,
    h: usize,
    time: f32,
    p: f32,
    intensity: f32,
    px: f32,
    noise: &[i16],
    rng: &mut Rng,
) {
    let band = ((time * 0.23).fract() * h as f32) as isize;
    let band_half = (3.0 * px) as isize;
    let line = (px * 0.5).round().max(1.0) as usize;
    // Grain amplitude in 1/256 steps (noise values are -256..256).
    let grain = (10.0 * intensity) as i32;
    let row = w * 3;
    for (y, dst) in out.chunks_exact_mut(row).enumerate().take(h) {
        let mut off = (time * 9.0 + y as f32 * 0.21 / px).sin() * (0.4 + 1.5 * p) * px;
        let in_band = (y as isize - band).abs() < band_half;
        if in_band {
            off += rng.signed() * 6.0 * intensity * px;
        }
        shift_row(dst, off.round() as isize);

        let scan: i32 = if (y / line) % 2 == 1 { 210 } else { 256 };
        let lift: i32 = if in_band { 25 } else { 0 };
        // A random window into the grain table for this row.
        let start = rng.below(noise.len() - w.min(noise.len() - 1));
        for (pxl, &n) in dst.chunks_exact_mut(3).zip(noise[start..].iter().cycle()) {
            let n = n as i32 * grain / 256 + lift;
            for v in pxl {
                *v = (((*v as i32 * scan) >> 8) + n).clamp(0, 255) as u8;
            }
        }
    }
}

/// Shift a row of RGB pixels right by `off` (left if negative), repeating the edge pixel.
fn shift_row(row: &mut [u8], off: isize) {
    let w = row.len() / 3;
    let k = off.unsigned_abs().min(w.saturating_sub(1));
    if k == 0 {
        return;
    }
    let b = k * 3;
    if off > 0 {
        row.copy_within(..row.len() - b, b);
        let edge = [row[b], row[b + 1], row[b + 2]];
        for px in row[..b].chunks_exact_mut(3) {
            px.copy_from_slice(&edge);
        }
    } else {
        row.copy_within(b.., 0);
        let n = row.len();
        let edge = [row[n - b - 3], row[n - b - 2], row[n - b - 1]];
        for px in row[n - b..].chunks_exact_mut(3) {
            px.copy_from_slice(&edge);
        }
    }
}

/// Fills the space around a video that doesn't cover its screen area (another aspect
/// ratio, or smaller) with a zoomed, blurred and darkened copy of the frame. The copy is
/// built at low resolution and scaled up smoothly, so it costs little.
#[derive(Default)]
pub struct Backdrop {
    small: Vec<u8>,
    tmp: Vec<u8>,
    sums: Vec<u32>,
}

/// Part of a frame as (x0, y0, x1, y1), end exclusive.
pub type Crop = (usize, usize, usize, usize);

/// The picture inside black bars baked into a video frame (letterboxing, pillarboxing).
/// Edges count as bars only when they are nearly black across the whole line and at
/// least 2% of the frame; a frame that is all dark (a fade) keeps its full size.
pub fn content_rect(frame: &[u8], w: usize, h: usize) -> Crop {
    const DARK: u8 = 24;
    let lit = |i: usize| {
        let p = &frame[i * 3..i * 3 + 3];
        p[0].max(p[1]).max(p[2]) > DARK
    };
    // A line is part of the picture if more than 1% of its (sampled) pixels are lit.
    let row_lit = |y: usize| (0..w).step_by(2).filter(|&x| lit(y * w + x)).count() * 2 > w / 100;
    let col_lit = |x: usize| (0..h).step_by(2).filter(|&y| lit(y * w + x)).count() * 2 > h / 100;
    let (Some(top), Some(bottom)) = ((0..h).find(|&y| row_lit(y)), (0..h).rfind(|&y| row_lit(y)))
    else {
        return (0, 0, w, h);
    };
    let left = (0..w).find(|&x| col_lit(x)).unwrap_or(0);
    let right = (0..w).rfind(|&x| col_lit(x)).map_or(w, |x| x + 1);
    let (min_x, min_y) = (w / 50, h / 50);
    let x0 = if left >= min_x { left } else { 0 };
    let x1 = if w - right >= min_x { right } else { w };
    let y0 = if top >= min_y { top } else { 0 };
    let y1 = if h - (bottom + 1) >= min_y { bottom + 1 } else { h };
    // A tiny picture is more likely a dark scene with one bright spot than real bars.
    if (x1 - x0) * 4 < w || (y1 - y0) * 4 < h {
        return (0, 0, w, h);
    }
    (x0, y0, x1, y1)
}

/// Backdrop resolution: 1/BACKDROP_SCALE of the canvas, but at least BACKDROP_MIN wide so
/// small canvases (the text-block renderer is ~120 px) still show shapes, not one color.
const BACKDROP_SCALE: usize = 6;
const BACKDROP_MIN: usize = 32;
const BACKDROP_ZOOM: f32 = 1.5;
/// Brightness of the backdrop, of 256.
const BACKDROP_LIGHT: u32 = 150;

impl Backdrop {
    /// Put `frame` (fw x fh) in the middle of a cw x ch canvas written to `out`. Only the
    /// `content` part of the frame (x0, y0, x1, y1; the rest is black bars baked into the
    /// video) is copied; everything else shows the backdrop.
    #[allow(clippy::too_many_arguments)]
    pub fn compose(
        &mut self,
        frame: &[u8],
        fw: usize,
        fh: usize,
        content: Crop,
        cw: usize,
        ch: usize,
        out: &mut Vec<u8>,
    ) {
        out.resize(cw * ch * 3, 0);
        let (x0, y0, x1, y1) = content;
        let (pw, ph) = ((x1 - x0) as f32, (y1 - y0) as f32);
        let (mx, my) = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
        let sw = (cw / BACKDROP_SCALE).max(BACKDROP_MIN).min(cw);
        let sh = (ch * sw / cw).max(1);
        // Sample the frame scaled to cover the whole canvas, zoomed in further so the
        // middle of the picture, not just its edges, shows around the video.
        let scale = (cw as f32 / pw).max(ch as f32 / ph) * BACKDROP_ZOOM;
        self.small.resize(sw * sh * 3, 0);
        for y in 0..sh {
            let cy = (y as f32 + 0.5) * ch as f32 / sh as f32;
            let fy = ((cy - ch as f32 / 2.0) / scale + my).clamp(y0 as f32, (y1 - 1) as f32);
            for x in 0..sw {
                let cx = (x as f32 + 0.5) * cw as f32 / sw as f32;
                let fx = ((cx - cw as f32 / 2.0) / scale + mx).clamp(x0 as f32, (x1 - 1) as f32);
                let s = (fy as usize * fw + fx as usize) * 3;
                let o = (y * sw + x) * 3;
                for c in 0..3 {
                    self.small[o + c] = ((frame[s + c] as u32 * BACKDROP_LIGHT) >> 8) as u8;
                }
            }
        }
        self.tmp.resize(self.small.len(), 0);
        // Blur relative to the backdrop's size: soft, but shapes and colors still read.
        let radius = (sw / 48).max(1);
        for _ in 0..2 {
            box_blur(&mut self.small, &mut self.tmp, &mut self.sums, sw, sh, radius);
        }

        // Bilinear lookup tables: per output column, the two backdrop columns and weight.
        let taps = |n: usize, m: usize| -> Vec<(usize, usize, u32)> {
            (0..n)
                .map(|i| {
                    let f = ((i as f32 + 0.5) * m as f32 / n as f32 - 0.5).clamp(0.0, (m - 1) as f32);
                    let a = f as usize;
                    (a, (a + 1).min(m - 1), ((f - a as f32) * 256.0) as u32)
                })
                .collect()
        };
        let (xs, ys) = (taps(cw, sw), taps(ch, sh));
        // Canvas position of the content.
        let (ox, oy) = ((cw - fw.min(cw)) / 2 + x0, (ch - fh.min(ch)) / 2 + y0);
        let (pw, ph) = (x1 - x0, y1 - y0);
        let row = cw * 3;
        for (y, dst) in out.chunks_exact_mut(row).enumerate() {
            let in_video = y >= oy && y < oy + ph;
            let (a, b, wy) = ys[y];
            let (ra, rb) = (&self.small[a * sw * 3..], &self.small[b * sw * 3..]);
            let mut fill = |from: usize, to: usize| {
                for x in from..to {
                    let (xa, xb, wx) = xs[x];
                    for c in 0..3 {
                        let top = ra[xa * 3 + c] as u32 * (256 - wx) + ra[xb * 3 + c] as u32 * wx;
                        let bot = rb[xa * 3 + c] as u32 * (256 - wx) + rb[xb * 3 + c] as u32 * wx;
                        dst[x * 3 + c] = ((top * (256 - wy) + bot * wy) >> 16) as u8;
                    }
                }
            };
            if in_video {
                fill(0, ox);
                fill(ox + pw, cw);
                let fy = y - oy + y0;
                let src = &frame[(fy * fw + x0) * 3..(fy * fw + x1) * 3];
                dst[ox * 3..(ox + pw) * 3].copy_from_slice(src);
            } else {
                fill(0, cw);
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
    fn shift_row_repeats_edges() {
        let mut row: Vec<u8> = (1..=12).collect();
        shift_row(&mut row, 1);
        assert_eq!(row, [1, 2, 3, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let mut row: Vec<u8> = (1..=12).collect();
        shift_row(&mut row, -2);
        assert_eq!(row, [7, 8, 9, 10, 11, 12, 10, 11, 12, 10, 11, 12]);
        let mut row: Vec<u8> = (1..=6).collect();
        shift_row(&mut row, 99);
        assert_eq!(row, [1, 2, 3, 1, 2, 3]);
    }

    #[test]
    fn backdrop_keeps_the_video_and_fills_the_rest() {
        let (fw, fh, cw, ch) = (40, 30, 100, 30);
        let frame: Vec<u8> = (0..fw * fh * 3).map(|i| (i % 200) as u8 + 50).collect();
        let mut out = Vec::new();
        let mut b = Backdrop::default();
        b.compose(&frame, fw, fh, (0, 0, fw, fh), cw, ch, &mut out);
        assert_eq!(out.len(), cw * ch * 3);
        let ox = (cw - fw) / 2;
        for y in 0..fh {
            assert_eq!(
                &out[(y * cw + ox) * 3..(y * cw + ox + fw) * 3],
                &frame[y * fw * 3..(y + 1) * fw * 3]
            );
        }
        // The bars are filled, and darker than the picture.
        let bar: u32 = out[..ox * 3].iter().map(|&v| v as u32).sum();
        assert!(bar > 0 && bar / (ox as u32 * 3) < 160);
        // Odd sizes and a canvas taller than the frame.
        b.compose(&frame, fw, fh, (0, 0, fw, fh), 41, 77, &mut out);
        b.compose(&frame[..3], 1, 1, (0, 0, 1, 1), 9, 9, &mut out);
        // Baked-in bars: only the content rows are copied, the bars get the backdrop.
        b.compose(&frame, fw, fh, (0, 5, fw, 25), fw, fh, &mut out);
        assert_eq!(&out[5 * fw * 3..25 * fw * 3], &frame[5 * fw * 3..25 * fw * 3]);
    }

    #[test]
    fn finds_baked_in_black_bars() {
        let (w, h) = (160, 90);
        // A 160x90 frame with 15-row letterbox bars and a lit picture between them.
        let frame: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let v = if (15..75).contains(&(i / w)) { 120 } else { 6 };
                [v, v, v]
            })
            .collect();
        assert_eq!(content_rect(&frame, w, h), (0, 15, w, 75));
        assert_eq!(content_rect(&vec![0; w * h * 3], w, h), (0, 0, w, h));
        let full = vec![200; w * h * 3];
        assert_eq!(content_rect(&full, w, h), (0, 0, w, h));
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

    /// `cargo test --release bench_each_effect -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_each_effect() {
        let (w, h) = (1280, 720);
        let src: Vec<u8> = (0..w * h * 3).map(|i| (i * 37 % 256) as u8).collect();
        let only = |f: Option<Fx>| {
            let mut fx = Effects::new(PRESETS.iter().find(|p| p.fx.is_empty()).unwrap());
            if let Some(f) = f {
                fx.toggle(f);
            }
            fx
        };
        let mut line = String::new();
        for f in std::iter::once(None).chain(Fx::ALL.map(Some)) {
            let mut fx = only(f);
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
                // Keep glitch and invert running every frame.
                fx.glitch_left = 1.0;
                fx.invert_left = 1.0;
                fx.render(&src, w, h, d, 1.0 / 30.0, &mut out);
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            line += &format!("{} {ms:.2}  ", f.map_or("none", Fx::label));
        }
        println!("{w}x{h} ms/frame: {line}");
    }
}
