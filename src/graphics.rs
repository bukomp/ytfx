//! The pixel graphics renderer: the kitty graphics protocol where the terminal speaks it,
//! sixel otherwise.

use std::io::{self, Write};

use ratatui::layout::Rect;

use crate::kitty::Kitty;
use crate::sixel::Sixel;

pub enum Graphics {
    Kitty(Kitty),
    Sixel(Sixel),
}

impl Graphics {
    pub fn name(&self) -> &'static str {
        match self {
            Graphics::Kitty(_) => "kitty",
            Graphics::Sixel(_) => "sixel",
        }
    }

    /// Show an RGB24 `w` x `h` video frame stretched over `rect`. `cell` is the cell size
    /// in pixels (sixel images are not scaled by the terminal). `generation` changes with
    /// the frame: a sixel image is large, so an unchanged frame (paused) isn't sent again.
    pub fn draw(
        &mut self,
        out: &mut impl Write,
        rect: Rect,
        data: &[u8],
        (w, h): (usize, usize),
        cell: (u16, u16),
        generation: u64,
    ) -> io::Result<()> {
        // Kept apart from thumbnail keys, which are small ids.
        const VIDEO: u64 = 1 << 63;
        match self {
            Graphics::Kitty(k) => k.draw(out, rect, data, w, h),
            Graphics::Sixel(s) => s.draw_still(out, rect, data, (w, h), cell, generation | VIDEO),
        }
    }

    /// Like `draw`, for an image that doesn't change (`key` is unique per image).
    pub fn draw_still(
        &mut self,
        out: &mut impl Write,
        rect: Rect,
        data: &[u8],
        size: (usize, usize),
        cell: (u16, u16),
        key: u64,
    ) -> io::Result<()> {
        match self {
            Graphics::Kitty(k) => k.draw_still(out, rect, data, size, key),
            Graphics::Sixel(s) => s.draw_still(out, rect, data, size, cell, key),
        }
    }

    /// The screen was cleared, which may have removed the image: send it again next time.
    pub fn forget(&mut self) {
        match self {
            Graphics::Kitty(k) => k.forget(),
            Graphics::Sixel(s) => s.forget(),
        }
    }

    /// Remove the image. A sixel image is part of the cells it covers, so it can only go
    /// by redrawing them: see `needs_wipe`.
    pub fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        match self {
            Graphics::Kitty(k) => k.clear(out),
            Graphics::Sixel(_) => Ok(()),
        }
    }

    /// Whether the screen has to be cleared and redrawn before an image goes to `target`:
    /// a sixel image stays on screen until its cells are overwritten, and ratatui doesn't
    /// rewrite cells it thinks are unchanged.
    pub fn needs_wipe(&self, target: Option<Rect>) -> bool {
        match self {
            Graphics::Kitty(_) => false,
            Graphics::Sixel(s) => s.on_screen().is_some_and(|r| Some(r) != target),
        }
    }
}
