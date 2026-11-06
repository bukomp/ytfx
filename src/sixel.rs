//! Sixel output, for terminals without the kitty graphics protocol (foot, mlterm, xterm
//! with sixel, Windows Terminal...). Unlike kitty, a sixel image is not scaled by the
//! terminal: each frame is resampled to the pixel size of its cell rectangle, mapped to a
//! fixed 6x7x6 colour cube with ordered dithering, and sent as one sixel image.

use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;

const LEVELS: [usize; 3] = [6, 7, 6];
const COLORS: usize = LEVELS[0] * LEVELS[1] * LEVELS[2];

/// 4x4 Bayer matrix, values 0..16.
const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// Ask the terminal (Primary Device Attributes) whether it draws sixel graphics. Needs raw
/// mode, and must run before anything else reads the terminal's input.
pub fn probe() -> bool {
    let mut out = io::stdout();
    if out.write_all(b"\x1b[c").and_then(|()| out.flush()).is_err() {
        return false;
    }
    let reply = read_reply(Duration::from_millis(500));
    da1_has_sixel(&reply)
}

/// Bytes from stdin until a DA1 reply ends (`c`) or `timeout` passes.
fn read_reply(timeout: Duration) -> Vec<u8> {
    let fd = io::stdin().as_raw_fd();
    let end = Instant::now() + timeout;
    let mut reply = Vec::new();
    while reply.len() < 256 {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd; a byte buffer of the size passed to read.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis().max(1) as libc::c_int) };
        if ready <= 0 {
            break;
        }
        let mut buf = [0u8; 64];
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        reply.extend_from_slice(&buf[..n as usize]);
        // Keep only the reply: a key or mouse event may arrive around it.
        if let Some(i) = reply.windows(3).position(|w| w == b"\x1b[?") {
            reply.drain(..i);
            if let Some(end) = reply.iter().position(|&b| b == b'c') {
                reply.truncate(end + 1);
                break;
            }
        }
    }
    reply
}

/// Whether a DA1 reply (`ESC [ ? 62 ; 4 ; 22 c`) lists attribute 4, sixel graphics.
fn da1_has_sixel(reply: &[u8]) -> bool {
    let Some(body) = reply
        .strip_prefix(b"\x1b[?")
        .and_then(|r| r.strip_suffix(b"c"))
    else {
        return false;
    };
    body.split(|&b| b == b';').skip(1).any(|p| p == b"4")
}

pub struct Sixel {
    /// Where the last image went, so main can wipe it when that changes.
    on_screen: Option<Rect>,
    /// The still image on screen (see `draw_still`) and where.
    still: Option<(u64, Rect)>,
    lut: Box<[[[u8; 256]; 16]; 3]>,
    idx: Vec<u8>,
    masks: Vec<u8>,
    buf: Vec<u8>,
}

impl Sixel {
    pub fn new() -> Self {
        let mut lut = Box::new([[[0u8; 256]; 16]; 3]);
        let mul = [LEVELS[1] * LEVELS[2], LEVELS[2], 1];
        for (c, table) in lut.iter_mut().enumerate() {
            let top = (LEVELS[c] - 1) as f32;
            for (t, row) in table.iter_mut().enumerate() {
                let offset = (t as f32 + 0.5) / 16.0 - 0.5;
                for (v, out) in row.iter_mut().enumerate() {
                    let level = (v as f32 * top / 255.0 + 0.5 + offset)
                        .floor()
                        .clamp(0.0, top);
                    *out = (level as usize * mul[c]) as u8;
                }
            }
        }
        Sixel {
            on_screen: None,
            still: None,
            lut,
            idx: Vec::new(),
            masks: Vec::new(),
            buf: Vec::new(),
        }
    }

    pub fn on_screen(&self) -> Option<Rect> {
        self.on_screen
    }

    /// Show an RGB24 `w` x `h` frame stretched over `rect` (in cells of `cell` pixels).
    pub fn draw(
        &mut self,
        out: &mut impl Write,
        rect: Rect,
        data: &[u8],
        (w, h): (usize, usize),
        cell: (u16, u16),
    ) -> io::Result<()> {
        self.still = None;
        let ow = rect.width as usize * cell.0 as usize;
        // Whole sixel bands only: a partial band would spill into the next cell row.
        let oh = rect.height as usize * cell.1 as usize / 6 * 6;
        if ow == 0 || oh == 0 || w == 0 || h == 0 || data.len() < w * h * 3 {
            return Ok(());
        }
        self.quantize(data, w, h, ow, oh);
        self.encode(ow, oh);
        write!(out, "\x1b[{};{}H", rect.y + 1, rect.x + 1)?;
        out.write_all(&self.buf)?;
        self.on_screen = Some(rect);
        Ok(())
    }

    /// Like `draw`, for an image that doesn't change: it is only sent when `key` (unique
    /// per image) or `rect` differ from what is already on screen.
    pub fn draw_still(
        &mut self,
        out: &mut impl Write,
        rect: Rect,
        data: &[u8],
        size: (usize, usize),
        cell: (u16, u16),
        key: u64,
    ) -> io::Result<()> {
        if self.still == Some((key, rect)) {
            return Ok(());
        }
        self.draw(out, rect, data, size, cell)?;
        self.still = Some((key, rect));
        Ok(())
    }

    /// The screen was cleared, which removed the image.
    pub fn forget(&mut self) {
        self.still = None;
        self.on_screen = None;
    }

    /// Bilinear resample to `ow` x `oh` and map each pixel to a palette index.
    fn quantize(&mut self, data: &[u8], w: usize, h: usize, ow: usize, oh: usize) {
        self.idx.resize(ow * oh, 0);
        // Source position of each output column / row in 16.16 fixed point, pixel centres.
        let map = |o: usize, n: usize, i: usize| -> (usize, usize, u32) {
            let s = ((i as f64 + 0.5) * n as f64 / o as f64 - 0.5).max(0.0);
            let i0 = (s as usize).min(n - 1);
            let f = ((s - i0 as f64) * 256.0) as u32;
            (i0, (i0 + 1).min(n - 1), f.min(256))
        };
        let cols: Vec<_> = (0..ow).map(|x| map(ow, w, x)).collect();
        let [lr, lg, lb] = &*self.lut;
        for y in 0..oh {
            let (y0, y1, fy) = map(oh, h, y);
            let (r0, r1) = (&data[y0 * w * 3..], &data[y1 * w * 3..]);
            let bayer = &BAYER[y & 3];
            let row = &mut self.idx[y * ow..(y + 1) * ow];
            for (x, (out, &(x0, x1, fx))) in row.iter_mut().zip(&cols).enumerate() {
                let px = |c: usize| {
                    let top = r0[x0 * 3 + c] as u32 * (256 - fx) + r0[x1 * 3 + c] as u32 * fx;
                    let bot = r1[x0 * 3 + c] as u32 * (256 - fx) + r1[x1 * 3 + c] as u32 * fx;
                    ((top * (256 - fy) + bot * fy) >> 16) as usize
                };
                let t = bayer[x & 3] as usize;
                *out = lr[t][px(0)] + lg[t][px(1)] + lb[t][px(2)];
            }
        }
    }

    /// The sixel image for the palette indices in `idx` (`w` x `h`, `h` a multiple of 6).
    fn encode(&mut self, w: usize, h: usize) {
        let buf = &mut self.buf;
        buf.clear();
        // P2=1: unset pixels keep what is behind them (there are none: every pixel is set).
        buf.extend_from_slice(b"\x1bP0;1;0q\"1;1;");
        push_num(buf, w);
        buf.push(b';');
        push_num(buf, h);
        for i in 0..COLORS {
            let level = [
                i / (LEVELS[1] * LEVELS[2]),
                i / LEVELS[2] % LEVELS[1],
                i % LEVELS[2],
            ];
            buf.push(b'#');
            push_num(buf, i);
            buf.extend_from_slice(b";2");
            for (c, l) in level.into_iter().enumerate() {
                buf.push(b';');
                push_num(buf, (l * 100 + (LEVELS[c] - 1) / 2) / (LEVELS[c] - 1));
            }
        }

        self.masks.resize(COLORS * w, 0);
        let mut span = [(usize::MAX, 0usize); COLORS];
        let mut used = Vec::with_capacity(COLORS);
        for band in 0..h / 6 {
            for r in 0..6 {
                let y = band * 6 + r;
                for (x, &c) in self.idx[y * w..(y + 1) * w].iter().enumerate() {
                    let c = c as usize;
                    let s = &mut span[c];
                    if s.0 == usize::MAX {
                        used.push(c);
                    }
                    // Each row starts again at x = 0: keep the widest span.
                    s.0 = s.0.min(x);
                    s.1 = s.1.max(x);
                    self.masks[c * w + x] |= 1 << r;
                }
            }
            for (n, &c) in used.iter().enumerate() {
                let (lo, hi) = std::mem::replace(&mut span[c], (usize::MAX, 0));
                if n > 0 {
                    buf.push(b'$'); // back to the start of the band for the next colour
                }
                buf.push(b'#');
                push_num(buf, c);
                push_run(buf, b'?', lo);
                let row = &mut self.masks[c * w..(c + 1) * w];
                let mut x = lo;
                while x <= hi {
                    let m = row[x];
                    let mut end = x + 1;
                    while end <= hi && row[end] == m {
                        end += 1;
                    }
                    row[x..end].fill(0);
                    push_run(buf, b'?' + m, end - x);
                    x = end;
                }
            }
            used.clear();
            buf.push(b'-');
        }
        buf.extend_from_slice(b"\x1b\\");
    }
}

fn push_num(buf: &mut Vec<u8>, mut n: usize) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    buf.extend_from_slice(&digits[i..]);
}

/// `n` copies of the sixel `ch`, as a repeat introducer when that is shorter.
fn push_run(buf: &mut Vec<u8>, ch: u8, n: usize) {
    if n > 3 {
        buf.push(b'!');
        push_num(buf, n);
        buf.push(ch);
    } else {
        buf.extend(std::iter::repeat_n(ch, n));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sixel_from_device_attributes() {
        assert!(da1_has_sixel(b"\x1b[?62;4;22c"));
        assert!(da1_has_sixel(b"\x1b[?65;1;4c"));
        assert!(!da1_has_sixel(b"\x1b[?62;22c"));
        // The first number is the terminal class, not an attribute.
        assert!(!da1_has_sixel(b"\x1b[?4;6c"));
        assert!(!da1_has_sixel(b""));
    }

    #[test]
    fn encodes_a_solid_image() {
        let mut s = Sixel::new();
        let white = vec![255u8; 8 * 6 * 3];
        s.quantize(&white, 8, 6, 8, 6);
        assert!(s.idx.iter().all(|&i| i as usize == COLORS - 1));
        s.encode(8, 6);
        let text = String::from_utf8(s.buf.clone()).unwrap();
        assert!(text.starts_with("\x1bP0;1;0q\"1;1;8;6#0;2;0;0;0"));
        // One colour, one band: all six rows set ('~') eight times.
        assert!(text.ends_with("#251!8~-\x1b\\"), "{text}");
    }

    #[test]
    fn each_band_lists_every_colour_once() {
        let mut s = Sixel::new();
        // Left half black, right half white, 12 rows (two bands).
        let mut img = vec![0u8; 4 * 12 * 3];
        for y in 0..12 {
            for x in 2..4 {
                img[(y * 4 + x) * 3..][..3].fill(255);
            }
        }
        s.quantize(&img, 4, 12, 4, 12);
        s.encode(4, 12);
        let text = String::from_utf8(s.buf.clone()).unwrap();
        let body = &text[text.rfind(";100;100;100").unwrap() + 12..];
        assert_eq!(body, "#0~~$#251??~~-#0~~$#251??~~-\x1b\\");
        assert!(s.masks.iter().all(|&m| m == 0));
    }

    #[test]
    fn spans_cover_every_row_of_a_band() {
        let mut s = Sixel::new();
        // White at x=2 in row 0 and at x=0 in row 1; black elsewhere.
        let mut img = vec![0u8; 4 * 6 * 3];
        img[2 * 3..][..3].fill(255);
        img[4 * 3..][..3].fill(255);
        s.quantize(&img, 4, 6, 4, 6);
        s.encode(4, 6);
        let text = String::from_utf8(s.buf.clone()).unwrap();
        let body = &text[text.rfind(";100;100;100").unwrap() + 12..];
        assert_eq!(body, "#0|~}~$#251A?@-\x1b\\");
        assert!(s.masks.iter().all(|&m| m == 0));
    }
}
