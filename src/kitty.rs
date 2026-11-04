//! Kitty graphics protocol output: frames are sent to the terminal as real pixel images
//! (scaled by the terminal into a cell rectangle) instead of half-block characters.
//! kitty itself gets frames through POSIX shared memory, so only a short escape sequence
//! goes through the tty; other terminals that speak the protocol get base64 inline data.

use std::collections::VecDeque;
use std::ffi::CString;
use std::io::{self, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use ratatui::layout::Rect;

const IMAGE_ID: u32 = 7331;
/// Shared memory objects we created recently. kitty unlinks each one after reading it; we
/// unlink stragglers ourselves so a terminal that never reads them can't leak memory.
const SHM_KEEP: usize = 60;
const CHUNK: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    SharedMemory,
    Direct,
}

pub struct Kitty {
    transport: Transport,
    counter: u64,
    recent: VecDeque<CString>,
    shown: bool,
    /// The still image on screen (see `draw_still`) and where.
    still: Option<(u64, Rect)>,
}

impl Kitty {
    /// Which terminals get the graphics renderer, based on the environment they set.
    pub fn detect() -> Option<Transport> {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        if std::env::var_os("TMUX").is_some() {
            return None; // tmux doesn't pass graphics through
        }
        if std::env::var_os("KITTY_WINDOW_ID").is_some() || var("TERM") == "xterm-kitty" {
            return Some(Transport::SharedMemory);
        }
        match var("TERM_PROGRAM").as_str() {
            "ghostty" | "WezTerm" => Some(Transport::Direct),
            _ => None,
        }
    }

    pub fn new(transport: Transport) -> Self {
        Kitty {
            transport,
            counter: 0,
            recent: VecDeque::new(),
            shown: false,
            still: None,
        }
    }

    /// Show an RGB24 frame stretched over `rect` (in cells), replacing the previous frame.
    pub fn draw(
        &mut self,
        out: &mut impl Write,
        rect: Rect,
        data: &[u8],
        w: usize,
        h: usize,
    ) -> io::Result<()> {
        self.still = None;
        // z=-1 puts the image under text, so overlays drawn by ratatui stay readable.
        let ctrl = format!(
            "a=T,f=24,s={w},v={h},i={IMAGE_ID},p=1,c={},r={},C=1,z=-1,q=2",
            rect.width, rect.height
        );
        write!(out, "\x1b[{};{}H", rect.y + 1, rect.x + 1)?;

        if self.transport == Transport::SharedMemory {
            match self.write_shm(&data[..w * h * 3]) {
                Ok(name) => {
                    let name = B64.encode(name.as_bytes());
                    write!(out, "\x1b_G{ctrl},t=s;{name}\x1b\\")?;
                    self.shown = true;
                    return Ok(());
                }
                // No usable shared memory: fall back to inline data for good.
                Err(_) => self.transport = Transport::Direct,
            }
        }

        let payload = B64.encode(&data[..w * h * 3]);
        let chunks: Vec<&[u8]> = payload.as_bytes().chunks(CHUNK).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let more = u8::from(i + 1 < chunks.len());
            if i == 0 {
                write!(out, "\x1b_G{ctrl},t=d,m={more};")?;
            } else {
                write!(out, "\x1b_Gm={more};")?;
            }
            out.write_all(chunk)?;
            out.write_all(b"\x1b\\")?;
        }
        self.shown = true;
        Ok(())
    }

    /// Like `draw`, for an image that doesn't change: it is only sent when `key` (unique
    /// per image) or `rect` differ from what is already on screen.
    pub fn draw_still(
        &mut self,
        out: &mut impl Write,
        rect: Rect,
        data: &[u8],
        (w, h): (usize, usize),
        key: u64,
    ) -> io::Result<()> {
        if self.still == Some((key, rect)) {
            return Ok(());
        }
        self.draw(out, rect, data, w, h)?;
        self.still = Some((key, rect));
        Ok(())
    }

    /// The screen was cleared, which may have removed the image: send it again next time.
    pub fn forget(&mut self) {
        self.still = None;
    }

    /// Remove the image (leaving the player, opening an overlay).
    pub fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.still = None;
        if self.shown {
            write!(out, "\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\")?;
            self.shown = false;
        }
        Ok(())
    }

    fn write_shm(&mut self, data: &[u8]) -> io::Result<CString> {
        self.counter += 1;
        // macOS limits shm names to 31 bytes.
        let name = CString::new(format!("/ytfx{}.{}", std::process::id(), self.counter))?;
        // SAFETY: plain POSIX calls on a fresh object we own; the mapping is exactly
        // `data.len()` bytes and is unmapped before returning.
        unsafe {
            let fd = libc::shm_open(
                name.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                0o600 as libc::c_uint,
            );
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let fail = |fd| {
                let err = io::Error::last_os_error();
                libc::close(fd);
                libc::shm_unlink(name.as_ptr());
                err
            };
            if libc::ftruncate(fd, data.len() as libc::off_t) != 0 {
                return Err(fail(fd));
            }
            let ptr = libc::mmap(
                std::ptr::null_mut(),
                data.len(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );
            if ptr == libc::MAP_FAILED {
                return Err(fail(fd));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), ptr.cast::<u8>(), data.len());
            libc::munmap(ptr, data.len());
            libc::close(fd);
        }
        self.recent.push_back(name.clone());
        while self.recent.len() > SHM_KEEP {
            if let Some(old) = self.recent.pop_front() {
                // SAFETY: unlinking a name; ENOENT (kitty already did it) is fine.
                unsafe { libc::shm_unlink(old.as_ptr()) };
            }
        }
        Ok(name)
    }
}

impl Drop for Kitty {
    fn drop(&mut self) {
        for name in self.recent.drain(..) {
            // SAFETY: see write_shm.
            unsafe { libc::shm_unlink(name.as_ptr()) };
        }
    }
}
