//! "Edit" style frame effects. Everything works on packed RGB24 buffers, and every
//! effect is scaled by the drive signal (bass/mid/high pulse when synced).

/// What the effects react to. `pulse` jumps on a bass beat (less on mid and high hits)
/// and decays, `level` follows bass energy. `beat` is set on bass beats only.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drive {
    pub pulse: f32,
    pub level: f32,
    pub beat: bool,
    pub strength: f32,
}

/// Menu sections the effects are listed under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Motion,
    Time,
    Glitch,
    Color,
    Light,
    Texture,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Motion => "MOTION",
            Group::Time => "TIME",
            Group::Glitch => "GLITCH",
            Group::Color => "COLOR",
            Group::Light => "LIGHT",
            Group::Texture => "TEXTURE",
        }
    }
}

/// Every effect, in menu order (grouped). `fx as usize` is its index in `Fx::ALL`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fx {
    // Motion
    Zoom,
    Shake,
    Bounce,
    Spin,
    Wave,
    Bulge,
    Mirror,
    Kaleido,
    // Time
    Trails,
    Echo,
    Tunnel,
    Stutter,
    // Glitch
    Split,
    Glitch,
    Slice,
    Static,
    Swap,
    // Color
    Red,
    Noir,
    Thermal,
    Vapor,
    Hue,
    Poster,
    Solar,
    Invert,
    // Light
    Strobe,
    Flash,
    Glow,
    Vignette,
    Cinema,
    // Texture
    Blur,
    Pixel,
    Dither,
    Edges,
    Crt,
    Film,
    Vhs,
}

impl Fx {
    pub const ALL: [Fx; 37] = [
        Fx::Zoom,
        Fx::Shake,
        Fx::Bounce,
        Fx::Spin,
        Fx::Wave,
        Fx::Bulge,
        Fx::Mirror,
        Fx::Kaleido,
        Fx::Trails,
        Fx::Echo,
        Fx::Tunnel,
        Fx::Stutter,
        Fx::Split,
        Fx::Glitch,
        Fx::Slice,
        Fx::Static,
        Fx::Swap,
        Fx::Red,
        Fx::Noir,
        Fx::Thermal,
        Fx::Vapor,
        Fx::Hue,
        Fx::Poster,
        Fx::Solar,
        Fx::Invert,
        Fx::Strobe,
        Fx::Flash,
        Fx::Glow,
        Fx::Vignette,
        Fx::Cinema,
        Fx::Blur,
        Fx::Pixel,
        Fx::Dither,
        Fx::Edges,
        Fx::Crt,
        Fx::Film,
        Fx::Vhs,
    ];

    pub fn from_label(label: &str) -> Option<Fx> {
        Self::ALL.into_iter().find(|fx| fx.label() == label)
    }

    /// Short name, also the key in the settings file (don't rename).
    pub fn label(self) -> &'static str {
        self.info().0
    }

    /// One line for the menu: what the effect does.
    pub fn about(self) -> &'static str {
        self.info().1
    }

    pub fn group(self) -> Group {
        use Fx::*;
        match self {
            Zoom | Shake | Bounce | Spin | Wave | Bulge | Mirror | Kaleido => Group::Motion,
            Trails | Echo | Tunnel | Stutter => Group::Time,
            Split | Glitch | Slice | Static | Swap => Group::Glitch,
            Red | Noir | Thermal | Vapor | Hue | Poster | Solar | Invert => Group::Color,
            Strobe | Flash | Glow | Vignette | Cinema => Group::Light,
            Blur | Pixel | Dither | Edges | Crt | Film | Vhs => Group::Texture,
        }
    }

    fn info(self) -> (&'static str, &'static str) {
        match self {
            Fx::Zoom => ("zoom", "punch in on every hit"),
            Fx::Shake => ("shake", "camera shake on hits"),
            Fx::Bounce => ("bounce", "picture jumps up on hits"),
            Fx::Spin => ("spin", "rotation kick, alternating"),
            Fx::Wave => ("wave", "rows sway in a sine wave"),
            Fx::Bulge => ("bulge", "fisheye pop on hits"),
            Fx::Mirror => ("mirror", "left half mirrored right"),
            Fx::Kaleido => ("kaleido", "four-way mirror"),
            Fx::Trails => ("trails", "motion smear, hits cut through"),
            Fx::Echo => ("echo", "colour channels lag behind"),
            Fx::Tunnel => ("tunnel", "zooming feedback loop"),
            Fx::Stutter => ("stutter", "freeze frame on big hits"),
            Fx::Split => ("rgb", "chromatic aberration"),
            Fx::Glitch => ("glitch", "datamosh slices and blocks"),
            Fx::Slice => ("slice", "bands slide apart on hits"),
            Fx::Static => ("static", "tv snow, bursts on hits"),
            Fx::Swap => ("swap", "colour channels swap on hits"),
            Fx::Red => ("red", "blood-red duotone + vignette"),
            Fx::Noir => ("noir", "hard black and white"),
            Fx::Thermal => ("thermal", "heat camera colours"),
            Fx::Vapor => ("vapor", "purple / cyan duotone"),
            Fx::Hue => ("hue", "hue drifts, jumps on hits"),
            Fx::Poster => ("poster", "flat colour bands"),
            Fx::Solar => ("solar", "solarized highlights"),
            Fx::Invert => ("invert", "negative flash on big hits"),
            Fx::Strobe => ("strobe", "brightness pumps on hits"),
            Fx::Flash => ("flash", "white flash on hits"),
            Fx::Glow => ("glow", "bloom around highlights"),
            Fx::Vignette => ("vignette", "dark edges that pump"),
            Fx::Cinema => ("cinema", "black bars slam in"),
            Fx::Blur => ("blur", "soft focus, blurs on hits"),
            Fx::Pixel => ("pixel", "chunky pixels"),
            Fx::Dither => ("dither", "ordered-dither retro look"),
            Fx::Edges => ("edges", "neon outlines on black"),
            Fx::Crt => ("crt", "scanlines and phosphor mask"),
            Fx::Film => ("film", "sepia, grain and scratches"),
            Fx::Vhs => ("vhs", "tape wobble, tracking, grain"),
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
        name: "vaporwave",
        fx: &[Fx::Vapor, Fx::Echo, Fx::Wave, Fx::Glow, Fx::Crt],
    },
    Preset {
        name: "noir",
        fx: &[Fx::Noir, Fx::Film, Fx::Vignette, Fx::Cinema, Fx::Stutter],
    },
    Preset {
        name: "rave",
        fx: &[
            Fx::Kaleido,
            Fx::Tunnel,
            Fx::Hue,
            Fx::Flash,
            Fx::Spin,
            Fx::Zoom,
        ],
    },
    Preset {
        name: "heat",
        fx: &[Fx::Thermal, Fx::Bulge, Fx::Split, Fx::Static, Fx::Shake],
    },
    Preset {
        name: "broken tv",
        fx: &[Fx::Crt, Fx::Slice, Fx::Swap, Fx::Static, Fx::Bounce],
    },
    Preset {
        name: "neon",
        fx: &[Fx::Edges, Fx::Glow, Fx::Hue, Fx::Trails, Fx::Zoom],
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
    thermal_lut: [[u8; 3]; 256],
    vapor_lut: [[u8; 3]; 256],
    sepia_lut: [[u8; 3]; 256],
    /// Vignette strength per pixel (0..=256) for the red grade, cached per frame size.
    vignette: Vec<u16>,
    vignette_size: (usize, usize),
    /// Grain values for VHS, read at random offsets instead of a fresh random per pixel.
    noise: Vec<i16>,
    /// Running column sums for the blur's vertical pass.
    sums: Vec<u32>,
    /// Frame held by stutter, and how long it still holds.
    held: Vec<u8>,
    stutter_left: f32,
    /// Tunnel feedback: the previous tunnel output.
    feedback: Vec<u8>,
    /// Echo: two progressively slower copies of the picture.
    lag1: Vec<u8>,
    lag2: Vec<u8>,
    /// Glow's bright pass, and edge detection's luminance.
    scratch: Vec<u8>,
    luma: Vec<u8>,
    spin_dir: f32,
    hue: f32,
    flash: f32,
    /// Channel order while swap holds (1..=SWAPS, see `swap_channels`), and for how long.
    swap_perm: usize,
    swap_left: f32,
    /// Dither thresholds: 16 Bayer steps x 256 input values, for `dither_levels` levels.
    dither_lut: Vec<u8>,
    dither_levels: u32,
}

/// Number of non-identity channel orders swap picks from.
const SWAPS: usize = 5;

/// Copy `src` into `buf` when `buf` doesn't hold a frame of this size yet.
fn prime(buf: &mut Vec<u8>, src: &[u8]) {
    if buf.len() != src.len() {
        buf.clear();
        buf.extend_from_slice(src);
    }
}

fn apply_lut(out: &mut [u8], lut: &[u8; 256]) {
    for v in out.iter_mut() {
        *v = lut[*v as usize];
    }
}

/// Append a per-channel curve to the fused tone curve (identity when empty).
fn chain(tone: &mut Option<[u8; 256]>, f: impl Fn(u8) -> u8) {
    let base = tone.unwrap_or(std::array::from_fn(|v| v as u8));
    *tone = Some(base.map(f));
}

/// Replace each pixel by `lut[luminance]`, mixed with the original by `amount` (0..1).
fn grade(out: &mut [u8], lut: &[[u8; 3]; 256], amount: f32) {
    let mix = (amount.clamp(0.0, 1.0) * 256.0) as u32;
    if mix >= 256 {
        for px in out.chunks_exact_mut(3) {
            px.copy_from_slice(&lut[luma(px) as usize]);
        }
        return;
    }
    // The table side of the mix, premultiplied.
    let pre: [[u16; 3]; 256] = std::array::from_fn(|l| lut[l].map(|t| (t as u32 * mix) as u16));
    let keep = 256 - mix;
    for px in out.chunks_exact_mut(3) {
        let t = pre[luma(px) as usize];
        for c in 0..3 {
            px[c] = ((px[c] as u32 * keep + t[c] as u32) >> 8) as u8;
        }
    }
}

/// A random run of `w` grain values (the table is at least `w` long).
fn noise_row<'a>(noise: &'a [i16], w: usize, rng: &mut Rng) -> &'a [i16] {
    let start = rng.below(noise.len() - w + 1);
    &noise[start..start + w]
}

/// Swap colour channels: output channel 0 takes input channel A, and so on.
fn swap_channels<const A: usize, const B: usize, const C: usize>(out: &mut [u8]) {
    for px in out.chunks_exact_mut(3) {
        let c = [px[0], px[1], px[2]];
        px[0] = c[A];
        px[1] = c[B];
        px[2] = c[C];
    }
}

fn luma(px: &[u8]) -> u32 {
    (px[0] as u32 * 77 + px[1] as u32 * 150 + px[2] as u32 * 29) >> 8
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
            thermal_lut: gradient_lut(&[
                (0.0, [0, 0, 0]),
                (0.2, [20, 0, 110]),
                (0.4, [150, 0, 160]),
                (0.6, [240, 40, 40]),
                (0.8, [255, 170, 0]),
                (1.0, [255, 255, 220]),
            ]),
            vapor_lut: gradient_lut(&[
                (0.0, [20, 0, 45]),
                (0.35, [140, 20, 160]),
                (0.65, [255, 90, 200]),
                (0.85, [80, 230, 255]),
                (1.0, [230, 255, 255]),
            ]),
            sepia_lut: gradient_lut(&[
                (0.0, [12, 6, 2]),
                (0.5, [150, 110, 70]),
                (1.0, [255, 240, 205]),
            ]),
            vignette: Vec::new(),
            vignette_size: (0, 0),
            noise: {
                let mut r = Rng::new(0x51f1_5eed);
                (0..NOISE_LEN)
                    .map(|_| (r.signed() * 256.0) as i16)
                    .collect()
            },
            sums: Vec::new(),
            held: Vec::new(),
            stutter_left: 0.0,
            feedback: Vec::new(),
            lag1: Vec::new(),
            lag2: Vec::new(),
            scratch: Vec::new(),
            luma: Vec::new(),
            spin_dir: 1.0,
            hue: 0.0,
            flash: 0.0,
            swap_perm: 0,
            swap_left: 0.0,
            dither_lut: Vec::new(),
            dither_levels: 0,
        };
        e.apply_preset(preset);
        e
    }

    pub fn apply_preset(&mut self, preset: &Preset) {
        self.set_enabled(preset.fx);
    }

    pub fn is_on(&self, fx: Fx) -> bool {
        self.enabled[fx as usize]
    }

    pub fn enabled(&self) -> Vec<Fx> {
        Fx::ALL.into_iter().filter(|&fx| self.is_on(fx)).collect()
    }

    pub fn count(&self) -> usize {
        self.enabled.iter().filter(|&&on| on).count()
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

    /// Forget per-video state (trail history, running glitches, held frames).
    pub fn reset(&mut self) {
        self.prev.clear();
        self.held.clear();
        self.feedback.clear();
        self.lag1.clear();
        self.lag2.clear();
        self.glitch_left = 0.0;
        self.invert_left = 0.0;
        self.stutter_left = 0.0;
        self.swap_left = 0.0;
        self.flash = 0.0;
    }

    fn vignette_for(&mut self, w: usize, h: usize) {
        if self.vignette_size != (w, h) {
            self.vignette = vignette(w, h);
            self.vignette_size = (w, h);
        }
    }

    pub fn render(&mut self, src: &[u8], w: usize, h: usize, d: Drive, dt: f32, out: &mut Vec<u8>) {
        let n = w * h * 3;
        if w == 0 || h == 0 || src.len() < n {
            return;
        }
        let src = &src[..n];
        out.resize(n, 0);
        self.tmp.resize(n, 0);
        self.time += dt;

        let on = self.enabled;
        let on = |fx: Fx| on[fx as usize];
        // Free the history buffers of effects that are off (a full frame each).
        for (fx, buf) in [
            (Fx::Trails, &mut self.prev),
            (Fx::Tunnel, &mut self.feedback),
            (Fx::Echo, &mut self.lag1),
            (Fx::Echo, &mut self.lag2),
            (Fx::Stutter, &mut self.held),
            (Fx::Edges, &mut self.luma),
        ] {
            if !on(fx) && buf.capacity() > 0 {
                *buf = Vec::new();
            }
        }
        if self.noise.len() <= w {
            let mut r = Rng::new(0x51f1_5eed);
            self.noise = (0..(w + 1).next_power_of_two())
                .map(|_| (r.signed() * 256.0) as i16)
                .collect();
        }
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
            if on(Fx::Stutter) && d.strength > 0.45 && self.stutter_left <= 0.0 {
                self.held.clear();
                self.held.extend_from_slice(src);
                self.stutter_left = 0.1 + 0.12 * d.strength;
            }
            if on(Fx::Swap) && self.rng.f32() < 0.35 + 0.65 * d.strength {
                self.swap_perm = 1 + self.rng.below(SWAPS);
                self.swap_left = 0.12 + 0.25 * d.strength;
            }
            self.spin_dir = -self.spin_dir;
            self.hue += std::f32::consts::FRAC_PI_3 * (0.5 + d.strength) * i.min(1.5);
            self.flash = self.flash.max(d.strength.max(0.4));
        }
        self.glitch_left -= dt;
        self.invert_left -= dt;
        self.stutter_left -= dt;
        self.swap_left -= dt;
        self.flash *= (-dt * 10.0).exp();
        self.hue += dt * 0.35 * i;

        let src = if on(Fx::Stutter) && self.stutter_left > 0.0 && self.held.len() == n {
            &self.held[..]
        } else {
            src
        };

        // Geometry: zoom, shake, bounce, spin, bulge and mirrors in one resampling pass.
        let mut warp = Warp {
            zoom: 1.0,
            mirror: on(Fx::Mirror) || on(Fx::Kaleido),
            mirror_y: on(Fx::Kaleido),
            ..Warp::default()
        };
        if on(Fx::Zoom) {
            warp.zoom = 1.0 + 0.2 * p + 0.04 * d.level * i;
        }
        if on(Fx::Shake) {
            warp.dx = self.rng.signed() * w as f32 * 0.035 * p;
            warp.dy = self.rng.signed() * h as f32 * 0.035 * p;
        }
        if on(Fx::Bounce) {
            warp.dy += h as f32 * 0.07 * p.min(1.5);
        }
        if on(Fx::Spin) {
            warp.angle = self.spin_dir * 0.14 * p.min(1.5);
            // Zoom in just enough that the rotated picture still covers the corners.
            let (s, c) = warp.angle.abs().sin_cos();
            let ratio = (w as f32 / h as f32).max(h as f32 / w as f32);
            warp.zoom *= c + s * ratio;
        }
        if on(Fx::Bulge) {
            warp.bulge = (0.15 + 0.45 * p).min(0.85);
        }
        geometry(src, out, w, h, &warp);

        if on(Fx::Wave) {
            let amp = (0.6 + 5.0 * p) * px;
            let freq = std::f32::consts::TAU * 2.5 / h as f32;
            for (y, row) in out.chunks_exact_mut(w * 3).enumerate() {
                let off = (self.time * 5.0 + y as f32 * freq).sin() * amp;
                shift_row(row, off.round() as isize);
            }
        }
        if on(Fx::Tunnel) {
            prime(&mut self.feedback, out);
            let zoom = 1.035 + 0.05 * p;
            // No rotation: a plain zoom takes the fast resampling path.
            let fb = Warp {
                zoom,
                ..Warp::default()
            };
            geometry(&self.feedback, &mut self.tmp, w, h, &fb);
            let k = (0.62 * i).min(0.86) * (1.0 - 0.35 * d.pulse.min(1.0));
            blend(out, &self.tmp, k);
            self.feedback.copy_from_slice(out);
        }
        if on(Fx::Trails) {
            // Smear into the previous output, but let beats cut through sharply.
            prime(&mut self.prev, out);
            let k = (0.55 * i).min(0.85) * (1.0 - 0.6 * d.pulse.min(1.0));
            blend(out, &self.prev, k);
            self.prev.copy_from_slice(out);
        }
        if on(Fx::Echo) {
            prime(&mut self.lag1, out);
            prime(&mut self.lag2, out);
            // lag1 follows the picture, lag2 follows lag1; green shows lag1, blue lag2.
            let k = ((0.45 * i).min(0.8) * 256.0) as u32;
            let lerp = |a: u8, b: u8| ((a as u32 * k + b as u32 * (256 - k)) >> 8) as u8;
            for ((o, a), b) in out.iter().zip(&mut self.lag1).zip(&mut self.lag2) {
                *a = lerp(*a, *o);
                *b = lerp(*b, *a);
            }
            for ((o, a), b) in out
                .chunks_exact_mut(3)
                .zip(self.lag1.chunks_exact(3))
                .zip(self.lag2.chunks_exact(3))
            {
                o[1] = a[1];
                o[2] = b[2];
            }
        }

        if on(Fx::Blur) {
            let radius = ((0.6 + 4.0 * p) * px).round() as usize;
            let radius = radius.min((10.0 * px) as usize);
            soft_blur(
                out,
                &mut self.scratch,
                &mut self.tmp,
                &mut self.sums,
                w,
                h,
                radius,
            );
        }
        if on(Fx::Glow) {
            let gain = (0.5 + 0.9 * p) * i.min(1.5);
            let radius = ((2.5 + 4.0 * p) * px).round() as usize;
            glow(
                out,
                &mut self.scratch,
                &mut self.tmp,
                &mut self.sums,
                w,
                h,
                radius,
                gain,
            );
        }
        if on(Fx::Split) {
            let off = ((0.5 + 6.0 * p) * px).round() as isize;
            rgb_split(out, &mut self.tmp, w, h, off);
        }
        if on(Fx::Slice) && p > 0.02 {
            let bands = 9;
            let shift = w as f32 * 0.08 * p.min(1.5);
            for (y, row) in out.chunks_exact_mut(w * 3).enumerate() {
                let band = y * bands / h;
                let dir = if band.is_multiple_of(2) { 1.0 } else { -1.0 };
                shift_row(row, (dir * shift).round() as isize);
            }
        }

        if on(Fx::Red) {
            let amount = ((0.55 + 0.45 * d.pulse) * i.min(1.0)).clamp(0.0, 1.0);
            self.vignette_for(w, h);
            red_grade(out, &self.vignette, &self.red_lut, amount);
        }
        if on(Fx::Noir) {
            // An S-curve that gets harder on hits.
            let k = 4.0 + 6.0 * p;
            let gray: [u8; 256] = std::array::from_fn(|v| {
                let x = v as f32 / 255.0 - 0.5;
                let y = 0.5 + 0.5 * (x * k).tanh() / (0.5 * k).tanh();
                (y.clamp(0.0, 1.0) * 255.0) as u8
            });
            for px in out.chunks_exact_mut(3) {
                let g = gray[luma(px) as usize];
                px.copy_from_slice(&[g, g, g]);
            }
        }
        if on(Fx::Thermal) {
            // Hits heat everything up a little.
            let lift = (40.0 * p) as usize;
            let lut: [[u8; 3]; 256] =
                std::array::from_fn(|v| self.thermal_lut[(v + lift).min(255)]);
            grade(out, &lut, 1.0);
        }
        if on(Fx::Vapor) {
            grade(out, &self.vapor_lut, (0.7 + 0.3 * p) * i.min(1.0));
        }
        if on(Fx::Hue) {
            hue_rotate(out, self.hue);
        }
        if on(Fx::Swap) && self.swap_left > 0.0 {
            match self.swap_perm {
                1 => swap_channels::<1, 2, 0>(out),
                2 => swap_channels::<2, 0, 1>(out),
                3 => swap_channels::<0, 2, 1>(out),
                4 => swap_channels::<2, 1, 0>(out),
                5 => swap_channels::<1, 0, 2>(out),
                _ => {}
            }
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
        if on(Fx::Static) {
            let amount = ((0.07 + 0.55 * p) * i.min(1.5)).min(0.9);
            snow(out, w, amount, &self.noise, &mut self.rng);
        }
        if on(Fx::Edges) {
            edges(out, &mut self.luma, w, h, 1.6 + 2.0 * p);
        }
        if on(Fx::Pixel) {
            let block = ((2.0 + 6.0 * p) * px).round() as usize;
            pixelate(out, w, h, block);
        }
        if on(Fx::Dither) {
            let levels = if p > 0.6 { 2 } else { 3 };
            let cell = (px * 0.5).round().max(1.0) as usize;
            if self.dither_levels != levels {
                self.dither_lut = dither_table(levels);
                self.dither_levels = levels;
            }
            dither(out, w, &self.dither_lut, cell);
        }
        if on(Fx::Vhs) {
            vhs(out, w, h, self.time, p, i, px, &self.noise, &mut self.rng);
        }
        if on(Fx::Crt) {
            crt(out, w, px, p);
        }
        if on(Fx::Film) {
            film(out, w, h, &self.sepia_lut, &self.noise, &mut self.rng, p, i);
        }
        if on(Fx::Vignette) {
            self.vignette_for(w, h);
            let k = ((0.5 + 0.9 * p) * i.min(1.5)).min(1.6);
            darken_edges(out, &self.vignette, k);
        }
        if on(Fx::Cinema) {
            let bar = ((0.07 + 0.09 * p.min(1.2)) * h as f32) as usize;
            let bar = bar.min(h / 2);
            out[..bar * w * 3].fill(0);
            out[(h - bar) * w * 3..].fill(0);
        }
        // Per-channel curves are composed into one table and applied in a single pass.
        let mut tone = None;
        if on(Fx::Poster) {
            let levels = (6.0 - 3.5 * p).round().max(2.0);
            let step = 255.0 / (levels - 1.0);
            chain(&mut tone, |v| {
                ((v as f32 / step).round() * step).min(255.0) as u8
            });
        }
        if on(Fx::Solar) {
            let t = (210.0 - 130.0 * p.min(1.2)) as u8;
            chain(&mut tone, |v| if v > t { 255 - v } else { v });
        }
        if on(Fx::Strobe) {
            let gain = 1.0 + 1.4 * p;
            chain(&mut tone, |v| (v as f32 * gain).min(255.0) as u8);
        }
        if on(Fx::Flash) && self.flash > 0.02 {
            let a = (self.flash * 0.85 * i).min(1.0);
            chain(&mut tone, |v| (v as f32 + (255.0 - v as f32) * a) as u8);
        }
        if on(Fx::Invert) && self.invert_left > 0.0 {
            chain(&mut tone, |v| 255 - v);
        }
        let only_invert = tone.is_some_and(|t| t[0] == 255 && t[255] == 0)
            && !on(Fx::Poster)
            && !on(Fx::Solar)
            && !on(Fx::Strobe)
            && !(on(Fx::Flash) && self.flash > 0.02);
        if only_invert {
            for v in out.iter_mut() {
                *v = 255 - *v;
            }
        } else if let Some(lut) = tone {
            apply_lut(out, &lut);
        }
    }
}

/// How `geometry` maps output pixels to source pixels.
#[derive(Default)]
struct Warp {
    zoom: f32,
    dx: f32,
    dy: f32,
    /// Fold the right half onto the left (and the bottom onto the top).
    mirror: bool,
    mirror_y: bool,
    /// Rotation in radians.
    angle: f32,
    /// Fisheye strength, 0 = none.
    bulge: f32,
}

fn geometry(src: &[u8], out: &mut [u8], w: usize, h: usize, warp: &Warp) {
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let inv = 1.0 / warp.zoom.max(1e-3);
    let (wmax, hmax) = ((w - 1) as f32, (h - 1) as f32);
    let row = w * 3;
    let fold_x = |x: usize| {
        if warp.mirror && x >= w / 2 {
            w - 1 - x
        } else {
            x
        }
    };
    let fold_y = |y: usize| {
        if warp.mirror_y && y >= h / 2 {
            h - 1 - y
        } else {
            y
        }
    };

    if warp.angle.abs() > 1e-4 || warp.bulge > 1e-4 {
        // Per-pixel path: every output pixel has its own source position. Everything
        // that depends only on the column is computed once per frame.
        let (sin, cos) = warp.angle.sin_cos();
        let (cs, sn) = (cos * inv, sin * inv);
        let (ox, oy) = (cx + warp.dx, cy + warp.dy);
        let us: Vec<f32> = (0..w).map(|x| fold_x(x) as f32 - cx).collect();
        let ucs: Vec<f32> = us.iter().map(|u| u * cs).collect();
        let usn: Vec<f32> = us.iter().map(|u| u * sn).collect();
        let (rx2, ry2) = (1.0 / (cx * cx).max(0.25), 1.0 / (cy * cy).max(0.25));
        let ur2: Vec<f32> = us.iter().map(|u| u * u * rx2).collect();
        let fetch = |o: &mut [u8], su: f32, sv: f32| {
            let s = (sv.clamp(0.0, hmax) as usize * w + su.clamp(0.0, wmax) as usize) * 3;
            o.copy_from_slice(&src[s..s + 3]);
        };
        for (y, out_row) in out.chunks_exact_mut(row).enumerate().take(h) {
            let v = fold_y(y) as f32 - cy;
            let (av, bv) = (-v * sn, v * cs);
            let pixels = out_row.chunks_exact_mut(3).zip(ucs.iter().zip(&usn));
            if warp.bulge > 1e-4 {
                // Sample nearer the middle inside the unit ellipse: a magnifying bulge.
                let vr2 = v * v * ry2;
                for ((o, (&uc, &us)), &ur) in pixels.zip(&ur2) {
                    let t = (1.0 - (ur + vr2)).max(0.0);
                    let s = 1.0 - warp.bulge * t * t;
                    fetch(o, s * (uc + av) + ox, s * (us + bv) + oy);
                }
            } else {
                for (o, (&uc, &us)) in pixels {
                    fetch(o, uc + av + ox, us + bv + oy);
                }
            }
        }
        return;
    }

    let identity_x = !warp.mirror && (warp.zoom - 1.0).abs() < 1e-4 && warp.dx.round() == 0.0;
    // Source column of every output column (in bytes), shared by all rows.
    let cols: Vec<usize> = if identity_x {
        Vec::new()
    } else {
        (0..w)
            .map(|x| ((fold_x(x) as f32 - cx) * inv + cx + warp.dx).clamp(0.0, wmax) as usize * 3)
            .collect()
    };
    for (y, out_row) in out.chunks_exact_mut(row).enumerate().take(h) {
        let sy = ((fold_y(y) as f32 - cy) * inv + cy + warp.dy).clamp(0.0, hmax) as usize;
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
        let (add, sub) = (
            line(y as isize + r as isize + 1),
            line(y as isize - r as isize),
        );
        for ((s, &a), &b) in sums.iter_mut().zip(add).zip(sub) {
            *s = *s + a as u32 - b as u32;
        }
    }
}

/// Chromatic aberration: red shifted one way, blue the other.
fn rgb_split(out: &mut [u8], tmp: &mut [u8], w: usize, h: usize, off: isize) {
    if off == 0 {
        return;
    }
    tmp.copy_from_slice(out);
    let voff = off / 3;
    let (wi, hi) = (w as isize - 1, h as isize - 1);
    // Source byte of every column's red and blue, shared by all rows.
    let col = |d: isize, c: usize| -> Vec<usize> {
        (0..w as isize)
            .map(|x| (x + d).clamp(0, wi) as usize * 3 + c)
            .collect()
    };
    let (red, blue) = (col(off, 0), col(-off, 2));
    let row = w * 3;
    for (y, dst) in out.chunks_exact_mut(row).enumerate() {
        let yr = (y as isize + voff).clamp(0, hi) as usize;
        let yb = (y as isize - voff).clamp(0, hi) as usize;
        let (rr, rb) = (&tmp[yr * row..][..row], &tmp[yb * row..][..row]);
        for ((o, &r), &b) in dst.chunks_exact_mut(3).zip(&red).zip(&blue) {
            o[0] = rr[r];
            o[2] = rb[b];
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
    let keep = 256 - mix;
    // The table side of the mix, premultiplied.
    let pre: [[u16; 3]; 256] = std::array::from_fn(|l| lut[l].map(|t| (t as u32 * mix) as u16));
    for (px, &v) in out.chunks_exact_mut(3).zip(vignette) {
        let t = pre[luma(px) as usize];
        for c in 0..3 {
            let graded = (px[c] as u32 * keep + t[c] as u32) >> 8;
            px[c] = ((graded * v as u32) >> 8) as u8;
        }
    }
}

/// A luminance -> colour table from (position 0..1, colour) stops, in order.
fn gradient_lut(stops: &[(f32, [u8; 3])]) -> [[u8; 3]; 256] {
    std::array::from_fn(|i| {
        let t = i as f32 / 255.0;
        let k = stops.iter().rposition(|s| s.0 <= t).unwrap_or(0);
        let (a, b) = (stops[k], stops[(k + 1).min(stops.len() - 1)]);
        let f = if b.0 > a.0 {
            (t - a.0) / (b.0 - a.0)
        } else {
            0.0
        };
        std::array::from_fn(|c| (a.1[c] as f32 + (b.1[c] as f32 - a.1[c] as f32) * f) as u8)
    })
}

/// Rotate hue by `angle` radians: a rotation around the gray axis in RGB space.
fn hue_rotate(out: &mut [u8], angle: f32) {
    let (s, c) = angle.sin_cos();
    let (a, b) = ((1.0 - c) / 3.0, (1.0f32 / 3.0).sqrt() * s);
    let m = [
        [c + a, a - b, a + b],
        [a + b, c + a, a - b],
        [a - b, a + b, c + a],
    ];
    // Fixed point, 1.0 = 256.
    let m: [[i32; 3]; 3] = m.map(|r| r.map(|v| (v * 256.0) as i32));
    for px in out.chunks_exact_mut(3) {
        let (r, g, bl) = (px[0] as i32, px[1] as i32, px[2] as i32);
        for (o, k) in px.iter_mut().zip(&m) {
            *o = ((r * k[0] + g * k[1] + bl * k[2]) >> 8).clamp(0, 255) as u8;
        }
    }
}

/// Bloom: blur the bright parts and add them back on top. Works at reduced resolution
/// (the result is blurry anyway), so it costs little at any frame size.
#[allow(clippy::too_many_arguments)]
fn glow(
    out: &mut [u8],
    small: &mut Vec<u8>,
    tmp: &mut [u8],
    sums: &mut Vec<u32>,
    w: usize,
    h: usize,
    radius: usize,
    gain: f32,
) {
    let f = blur_scale(radius);
    let (sw, sh) = downsample(out, w, h, f, small);
    for v in small.iter_mut() {
        *v = v.saturating_sub(140).saturating_mul(2);
    }
    box_blur(small, tmp, sums, sw, sh, (radius / f).max(1));
    let g = (gain * 256.0) as u32;
    upsample(small, sw, sh, out, w, h, |o, v| {
        (o as u32 + ((v * g) >> 8)).min(255) as u8
    });
}

/// Box blur that runs at reduced resolution for large radii and scales back up smoothly.
#[allow(clippy::too_many_arguments)]
fn soft_blur(
    out: &mut [u8],
    small: &mut Vec<u8>,
    tmp: &mut [u8],
    sums: &mut Vec<u32>,
    w: usize,
    h: usize,
    radius: usize,
) {
    let f = blur_scale(radius);
    if f == 1 {
        return box_blur(out, tmp, sums, w, h, radius);
    }
    let (sw, sh) = downsample(out, w, h, f, small);
    box_blur(small, tmp, sums, sw, sh, (radius / f).max(1));
    upsample(small, sw, sh, out, w, h, |_, v| v as u8);
}

/// Downscale factor for blurring with `radius`: a power of two, 1 to 8, leaving a radius
/// of at least 2 pixels at the reduced size.
fn blur_scale(radius: usize) -> usize {
    let f = (radius / 2).max(1);
    (1usize << f.ilog2()).min(8)
}

/// Average `f` x `f` blocks of `src` (w x h) into `dst`. Returns the reduced size.
fn downsample(src: &[u8], w: usize, h: usize, f: usize, dst: &mut Vec<u8>) -> (usize, usize) {
    let (sw, sh) = (w.div_ceil(f), h.div_ceil(f));
    dst.clear();
    dst.resize(sw * sh * 3, 0);
    let row = w * 3;
    let mut acc = vec![0u32; sw * 3];
    for (sy, drow) in dst.chunks_exact_mut(sw * 3).enumerate() {
        acc.fill(0);
        let (y0, y1) = (sy * f, ((sy + 1) * f).min(h));
        for y in y0..y1 {
            let srow = &src[y * row..][..row];
            for (sx, a) in acc.chunks_exact_mut(3).enumerate() {
                let (x0, x1) = (sx * f, ((sx + 1) * f).min(w));
                for px in srow[x0 * 3..x1 * 3].chunks_exact(3) {
                    a[0] += px[0] as u32;
                    a[1] += px[1] as u32;
                    a[2] += px[2] as u32;
                }
            }
        }
        for (sx, (d, a)) in drow
            .chunks_exact_mut(3)
            .zip(acc.chunks_exact(3))
            .enumerate()
        {
            let count = ((((sx + 1) * f).min(w) - sx * f) * (y1 - y0)) as u32;
            for c in 0..3 {
                d[c] = (a[c] / count) as u8;
            }
        }
    }
    (sw, sh)
}

/// Per output index (of n), the two source indices (of m) and the weight of the second
/// (0..256), for bilinear scaling.
fn bilinear_taps(n: usize, m: usize) -> Vec<(usize, usize, u32)> {
    (0..n)
        .map(|i| {
            let f = ((i as f32 + 0.5) * m as f32 / n as f32 - 0.5).clamp(0.0, (m - 1) as f32);
            let a = f as usize;
            (a, (a + 1).min(m - 1), ((f - a as f32) * 256.0) as u32)
        })
        .collect()
}

/// Scale `small` (sw x sh) up to w x h with bilinear filtering, combining each value
/// (0..=255) with the pixel already in `out` through `op`. The horizontal pass runs on the
/// few small rows; the vertical pass, per output row, is plain element-wise arithmetic.
fn upsample(
    small: &[u8],
    sw: usize,
    sh: usize,
    out: &mut [u8],
    w: usize,
    h: usize,
    op: impl Fn(u8, u32) -> u8,
) {
    let row = w * 3;
    // Every small row stretched to the full width (values x 256).
    let mut wide = vec![0u16; sh * row];
    let xs = bilinear_taps(w, sw);
    for (src, dst) in small.chunks_exact(sw * 3).zip(wide.chunks_exact_mut(row)) {
        for (d, &(a, b, wx)) in dst.chunks_exact_mut(3).zip(&xs) {
            let (pa, pb) = (&src[a * 3..a * 3 + 3], &src[b * 3..b * 3 + 3]);
            for c in 0..3 {
                d[c] = (pa[c] as u32 * (256 - wx) + pb[c] as u32 * wx) as u16;
            }
        }
    }
    for (dst, &(a, b, wy)) in out.chunks_exact_mut(row).zip(&bilinear_taps(h, sh)) {
        let (ra, rb) = (&wide[a * row..][..row], &wide[b * row..][..row]);
        for ((o, &p), &q) in dst.iter_mut().zip(ra).zip(rb) {
            *o = op(*o, (p as u32 * (256 - wy) + q as u32 * wy) >> 16);
        }
    }
}

/// TV snow mixed over the picture.
fn snow(out: &mut [u8], w: usize, amount: f32, noise: &[i16], rng: &mut Rng) {
    let a = (amount.clamp(0.0, 1.0) * 256.0) as i32;
    let keep = 256 - a;
    for row in out.chunks_exact_mut(w * 3) {
        for (px, &n) in row.chunks_exact_mut(3).zip(noise_row(noise, w, rng)) {
            let snow = ((n as i32 + 256) >> 1) * a;
            for v in px {
                *v = ((*v as i32 * keep + snow) >> 8) as u8;
            }
        }
    }
}

/// Neon outlines: Sobel edge strength lights up the picture's own colours, the rest goes
/// dark.
fn edges(out: &mut [u8], luma_buf: &mut Vec<u8>, w: usize, h: usize, gain: f32) {
    luma_buf.clear();
    luma_buf.extend(out.chunks_exact(3).map(|px| luma(px) as u8));
    let g = (gain * 256.0) as i32;
    // Edge strength (0..=256) from the 3x3 neighbourhood: columns a, b, c of rows up,
    // mid, down.
    let strength = |up: [u8; 3], mid: [u8; 3], dn: [u8; 3]| -> i32 {
        let l = |v: u8| v as i32;
        let gx = l(up[2]) + 2 * l(mid[2]) + l(dn[2]) - l(up[0]) - 2 * l(mid[0]) - l(dn[0]);
        let gy = l(dn[0]) + 2 * l(dn[1]) + l(dn[2]) - l(up[0]) - 2 * l(up[1]) - l(up[2]);
        // |g| approximated as max + min/2, scaled to 0..256.
        let (ax, ay) = (gx.abs(), gy.abs());
        (((ax.max(ay) + ax.min(ay) / 2) * g) >> 10).min(256)
    };
    // Lift dark channels so edges glow even where the picture is dim.
    let light = |o: &mut [u8], e: u32| {
        for v in o {
            *v = ((((*v as u32 >> 1) + 64) * e) >> 7).min(255) as u8;
        }
    };
    let at = |r: &[u8], x: usize| {
        let (a, b) = (x.saturating_sub(1), (x + 1).min(w - 1));
        [r[a], r[x], r[b]]
    };
    let mut strengths = vec![0u16; w];
    for y in 0..h {
        let up = &luma_buf[y.saturating_sub(1) * w..][..w];
        let mid = &luma_buf[y * w..][..w];
        let dn = &luma_buf[(y + 1).min(h - 1) * w..][..w];
        // The two edge columns repeat their outermost pixel.
        let ends: &[usize] = if w > 1 { &[0, w - 1] } else { &[0] };
        for &x in ends {
            strengths[x] = strength(at(up, x), at(mid, x), at(dn, x)) as u16;
        }
        if w >= 3 {
            // Interior: equal-length slices, so the loop needs no bounds checks and
            // vectorizes.
            let n = w - 2;
            let (ua, ub, uc) = (&up[..n], &up[1..n + 1], &up[2..]);
            let (ma, mc) = (&mid[..n], &mid[2..]);
            let (da, db, dc) = (&dn[..n], &dn[1..n + 1], &dn[2..]);
            let e = &mut strengths[1..n + 1];
            for k in 0..n {
                let l = |s: &[u8]| s[k] as i32;
                let gx = l(uc) - l(ua) + 2 * (l(mc) - l(ma)) + l(dc) - l(da);
                let gy = l(da) + 2 * l(db) + l(dc) - l(ua) - 2 * l(ub) - l(uc);
                let (ax, ay) = (gx.abs(), gy.abs());
                e[k] = (((ax.max(ay) + (ax.min(ay) >> 1)) * g) >> 10).min(256) as u16;
            }
        }
        let row = &mut out[y * w * 3..(y + 1) * w * 3];
        for (o, &e) in row.chunks_exact_mut(3).zip(&strengths) {
            light(o, e as u32);
        }
    }
}

/// Ordered 4x4 Bayer dithering through a table from `dither_table`.
fn dither(out: &mut [u8], w: usize, lut: &[u8], cell: usize) {
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    // Bayer column of every pixel column.
    let cols: Vec<usize> = (0..w).map(|x| (x / cell) % 4).collect();
    for (y, row) in out.chunks_exact_mut(w * 3).enumerate() {
        let br = &BAYER[(y / cell) % 4];
        for (px, &c) in row.chunks_exact_mut(3).zip(&cols) {
            let t = &lut[(br[c] as usize) << 8..][..256];
            for v in px {
                *v = t[*v as usize];
            }
        }
    }
}

/// Ordered dithering down to `levels` values per channel, as a table: for each of the
/// 16 Bayer thresholds, the output of every input value.
fn dither_table(levels: u32) -> Vec<u8> {
    let steps = levels.max(2) - 1;
    (0..16u32)
        .flat_map(|b| {
            // Threshold in 0..255 for this Bayer step.
            let t = b * 16 + 8;
            (0..256u32).map(move |v| {
                let scaled = v * steps;
                let (q, r) = (scaled / 255, scaled % 255);
                let q = if r > t { q + 1 } else { q };
                (q * 255 / steps) as u8
            })
        })
        .collect()
}

/// Scanlines and an RGB phosphor mask; hits brighten the tube.
fn crt(out: &mut [u8], w: usize, px: f32, p: f32) {
    let line = (px * 0.5).round().max(1.0) as usize;
    let cell = (px * 0.34).round().max(1.0) as usize;
    let boost = (256.0 * (1.15 + 0.35 * p)) as u32;
    // Gain of every byte in a row (phosphor mask x brightness), for lit and dark lines.
    let gains = |scan: u32| -> Vec<u16> {
        let k = (scan * boost) >> 8;
        (0..w * 3)
            .map(|i| {
                let m: u32 = if i % 3 == (i / 3 / cell) % 3 {
                    256
                } else {
                    170
                };
                ((m * k) >> 8) as u16
            })
            .collect()
    };
    let (lit, dark) = (gains(256), gains(150));
    for (y, row) in out.chunks_exact_mut(w * 3).enumerate() {
        let g = if (y / line) % 2 == 1 { &dark } else { &lit };
        for (v, &k) in row.iter_mut().zip(g) {
            *v = ((*v as u32 * k as u32) >> 8).min(255) as u8;
        }
    }
}

/// Old film: sepia, flicker, grain, and now and then a scratch or a dust speck.
#[allow(clippy::too_many_arguments)]
fn film(
    out: &mut [u8],
    w: usize,
    h: usize,
    sepia: &[[u8; 3]; 256],
    noise: &[i16],
    rng: &mut Rng,
    p: f32,
    intensity: f32,
) {
    let flicker = 1.0 + rng.signed() * 0.05 + 0.15 * p;
    let mix = ((0.85 * intensity).min(1.0) * 256.0) as i32;
    let keep = 256 - mix;
    // Sepia with this frame's flicker, premultiplied by the mix.
    let lut: [[i32; 3]; 256] = std::array::from_fn(|v| {
        sepia[((v as f32 * flicker) as usize).min(255)].map(|t| t as i32 * mix)
    });
    let grain = (18.0 * intensity) as i32;
    // Grade and grain in one pass.
    for row in out.chunks_exact_mut(w * 3) {
        for (px, &n) in row.chunks_exact_mut(3).zip(noise_row(noise, w, rng)) {
            let t = lut[luma(px) as usize];
            let n = (n as i32 * grain) >> 8;
            for c in 0..3 {
                px[c] = (((px[c] as i32 * keep + t[c]) >> 8) + n).clamp(0, 255) as u8;
            }
        }
    }
    // Scratches: thin bright vertical lines.
    for _ in 0..rng.below(3) {
        let x = rng.below(w);
        for y in 0..h {
            let o = (y * w + x) * 3;
            out[o..o + 3].copy_from_slice(&[230, 220, 200]);
        }
    }
    // Dust: a few dark specks.
    let size = (w / 160).max(1);
    for _ in 0..rng.below(4) {
        let (x0, y0) = (rng.below(w), rng.below(h));
        for y in y0..(y0 + size).min(h) {
            for x in x0..(x0 + size).min(w) {
                let o = (y * w + x) * 3;
                out[o..o + 3].copy_from_slice(&[15, 10, 5]);
            }
        }
    }
}

/// Darken towards the edges by `k` (1 = the vignette table as is, more = darker).
fn darken_edges(out: &mut [u8], vignette: &[u16], k: f32) {
    let k = (k * 256.0) as u32;
    for (px, &v) in out.chunks_exact_mut(3).zip(vignette) {
        let dark = (((256 - v as u32) * k) >> 8).min(256);
        let f = 256 - dark;
        for c in px {
            *c = ((*c as u32 * f) >> 8) as u8;
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
        for (pxl, &n) in dst.chunks_exact_mut(3).zip(noise_row(noise, w, rng)) {
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
    let y1 = if h - (bottom + 1) >= min_y {
        bottom + 1
    } else {
        h
    };
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
            box_blur(
                &mut self.small,
                &mut self.tmp,
                &mut self.sums,
                sw,
                sh,
                radius,
            );
        }

        // Bilinear lookup tables: per output column, the two backdrop columns and weight.
        let (xs, ys) = (bilinear_taps(cw, sw), bilinear_taps(ch, sh));
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
    fn each_effect_alone_survives_tiny_frames() {
        for f in Fx::ALL {
            let mut fx = Effects::new(PRESETS.iter().find(|p| p.fx.is_empty()).unwrap());
            fx.toggle(f);
            fx.intensity = 2.0;
            let mut out = Vec::new();
            for &(w, h) in &[(1, 1), (1, 4), (4, 1), (2, 3), (9, 7)] {
                let src: Vec<u8> = (0..w * h * 3).map(|i| (i * 53 % 256) as u8).collect();
                for step in 0..12 {
                    let d = Drive {
                        pulse: (step % 4) as f32 * 0.6,
                        level: 1.0,
                        beat: step % 2 == 0,
                        strength: 1.0,
                    };
                    fx.render(&src, w, h, d, 1.0 / 30.0, &mut out);
                    assert_eq!(out.len(), w * h * 3, "{} at {w}x{h}", f.label());
                }
            }
        }
    }

    #[test]
    fn catalogue_is_consistent() {
        for (i, f) in Fx::ALL.into_iter().enumerate() {
            assert_eq!(f as usize, i, "{} is out of place in Fx::ALL", f.label());
            assert_eq!(Fx::from_label(f.label()), Some(f));
            assert!(!f.about().is_empty());
        }
        // Menu groups are contiguous, so each header appears once.
        let mut seen = Vec::new();
        for f in Fx::ALL {
            if seen.last() != Some(&f.group()) {
                assert!(!seen.contains(&f.group()), "{:?} is split up", f.group());
                seen.push(f.group());
            }
        }
    }

    #[test]
    fn dither_and_poster_hit_their_levels() {
        let mut buf: Vec<u8> = (0..=255).flat_map(|v| [v, v, v]).collect();
        dither(&mut buf, 256, &dither_table(2), 1);
        assert!(buf.iter().all(|&v| v == 0 || v == 255));
        let mut buf = vec![0, 128, 255];
        dither(&mut buf, 1, &dither_table(3), 1);
        assert_eq!(buf[0], 0);
        assert_eq!(buf[2], 255);
    }

    #[test]
    fn hue_rotation_keeps_gray_and_wraps() {
        let mut gray = vec![100, 100, 100];
        hue_rotate(&mut gray, 1.3);
        assert!(gray.iter().all(|&v| v.abs_diff(100) <= 2));
        let mut red = vec![200, 10, 10];
        hue_rotate(&mut red, std::f32::consts::TAU);
        assert!(red[0] >= 195 && red[1] <= 15);
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
        assert_eq!(
            &out[5 * fw * 3..25 * fw * 3],
            &frame[5 * fw * 3..25 * fw * 3]
        );
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
mod preview_each {
    use super::*;

    /// Writes one PPM per effect, on a hit: `YTFX_FRAME=frame.rgb YTFX_OUT=dir cargo test preview_each -- --ignored`
    /// (frame.rgb is a raw RGB24 frame, 200x112 unless YTFX_SIZE says otherwise, e.g. 1280x720).
    #[test]
    #[ignore]
    fn preview_each() {
        let src = std::fs::read(std::env::var("YTFX_FRAME").unwrap()).unwrap();
        let dir = std::env::var("YTFX_OUT").unwrap();
        let (w, h) = std::env::var("YTFX_SIZE")
            .ok()
            .and_then(|s| {
                let (w, h) = s.split_once('x')?;
                Some((w.parse().ok()?, h.parse().ok()?))
            })
            .unwrap_or((200, 112));
        for f in Fx::ALL {
            let mut fx = Effects::new(PRESETS.iter().find(|p| p.fx.is_empty()).unwrap());
            fx.toggle(f);
            let mut out = Vec::new();
            for step in 0..6 {
                let d = Drive {
                    pulse: if step == 5 { 0.8 } else { 0.2 },
                    level: 0.6,
                    beat: step == 5,
                    strength: 0.8,
                };
                fx.render(&src, w, h, d, 1.0 / 30.0, &mut out);
            }
            let mut ppm = format!("P6 {w} {h} 255\n").into_bytes();
            ppm.extend_from_slice(&out);
            std::fs::write(format!("{dir}/{}.ppm", f.label()), ppm).unwrap();
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
                "{w}x{h}: {:.1} ms/frame (all {} effects)",
                t.elapsed().as_secs_f64() * 1000.0 / n as f64,
                Fx::ALL.len()
            );
        }
    }

    /// `cargo test --release bench_presets -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_presets() {
        let (w, h) = (1280, 720);
        let src: Vec<u8> = (0..w * h * 3).map(|i| (i * 37 % 256) as u8).collect();
        let mut line = String::new();
        for preset in PRESETS {
            let mut fx = Effects::new(preset);
            let mut out = Vec::new();
            let n = 60;
            let t = Instant::now();
            for k in 0..n {
                let d = Drive {
                    pulse: if k % 8 == 0 { 0.9 } else { 0.3 },
                    level: 0.6,
                    beat: k % 8 == 0,
                    strength: 0.8,
                };
                fx.render(&src, w, h, d, 1.0 / 30.0, &mut out);
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            line += &format!("{} {ms:.2}  ", preset.name);
        }
        println!("{w}x{h} preset ms/frame: {line}");
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
                fx.swap_left = 1.0;
                fx.flash = 1.0;
                fx.render(&src, w, h, d, 1.0 / 30.0, &mut out);
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / n as f64;
            line += &format!("{} {ms:.2}  ", f.map_or("none", Fx::label));
        }
        println!("{w}x{h} ms/frame: {line}");
    }
}
