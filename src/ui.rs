use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph, Widget};

use crate::effects::{Fx, PRESETS};
use crate::{App, Focus, RES_STEPS, Renderer};

pub const HUD_HEIGHT: u16 = 2;
const ACCENT: Color = Color::Rgb(255, 40, 60);
const DIM: Color = Color::Rgb(95, 95, 110);
const CHANNEL: Color = Color::Rgb(120, 180, 255);
const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// The part of the screen the video may use (everything above the HUD).
pub fn video_area(area: Rect) -> Rect {
    Rect {
        height: area.height.saturating_sub(HUD_HEIGHT),
        ..area
    }
}

/// The largest cell rectangle with the picture's aspect ratio, centered in `area`.
/// `cell_aspect` is cell height / cell width in pixels.
fn fit_cells(area: Rect, aspect: f64, cell_aspect: f64) -> Rect {
    let mut cols = area.width as f64;
    let mut rows = cols / aspect / cell_aspect;
    if rows > area.height as f64 {
        rows = area.height as f64;
        cols = rows * aspect * cell_aspect;
    }
    centered(
        area,
        (cols.round() as u16).max(1),
        (rows.round() as u16).max(1),
    )
}

/// Decode size in pixels for a video area. Each cell shows two vertically stacked
/// pixels (half blocks), which makes pixels roughly square.
pub fn fit(area: Rect, aspect: f64, max_width: u16) -> (usize, usize) {
    let max_w = area.width.min(max_width).max(2) as f64;
    let max_h = (area.height as f64 * 2.0).max(2.0);
    let (mut w, mut h) = (max_w, max_w / aspect);
    if h > max_h {
        h = max_h;
        w = h * aspect;
    }
    ((w as usize).max(2) & !1, (h as usize).max(2) & !1)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    if app.player.is_some() {
        draw_player(f, app);
    } else {
        draw_browser(f, app);
    }
    if let Some(msg) = &app.busy {
        let spin = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
        let text = format!(" {spin} {msg} ");
        let width = (Span::raw(&text).width() as u16 + 4).min(f.area().width);
        let rect = centered(f.area(), width, 3);
        f.render_widget(Clear, rect);
        f.render_widget(
            Paragraph::new(text)
                .fg(Color::White)
                .block(rounded().border_style(Style::new().fg(ACCENT))),
            rect,
        );
    }
    if app.show_help {
        draw_help(f, app.player.is_some());
    }
}

fn rounded() -> Block<'static> {
    Block::bordered().border_type(BorderType::Rounded)
}

fn draw_browser(f: &mut Frame, app: &mut App) {
    let [header, search, list, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(f.area());

    let title = Line::from(vec![
        Span::styled(
            " ▓▒░ YTFX ░▒▓ ",
            Style::new().fg(Color::Black).bg(ACCENT).bold(),
        ),
        Span::styled(
            "  youtube browser · bass-synced edit fx",
            Style::new().fg(DIM),
        ),
    ]);
    f.render_widget(Paragraph::new(title), header);

    let search_focused = matches!(app.focus, Focus::Search);
    let line = if search_focused {
        Line::from(vec![
            Span::raw(app.query.as_str()),
            Span::styled("▏", Style::new().fg(ACCENT)),
        ])
    } else if app.query.is_empty() {
        Line::from(Span::styled(
            "press / to search, or paste a YouTube URL",
            Style::new().fg(DIM),
        ))
    } else {
        Line::from(app.query.as_str())
    };
    f.render_widget(
        Paragraph::new(line).block(
            rounded()
                .title(" search ")
                .border_style(Style::new().fg(if search_focused { ACCENT } else { DIM })),
        ),
        search,
    );

    let block = rounded()
        .title(format!(" results ({}) ", app.results.len()))
        .border_style(Style::new().fg(if search_focused { DIM } else { ACCENT }));
    if app.results.is_empty() {
        let hint = Paragraph::new("type a search and press enter")
            .fg(DIM)
            .alignment(Alignment::Center);
        f.render_widget(hint.block(block), list);
    } else {
        let items: Vec<ListItem> = app
            .results
            .iter()
            .map(|r| {
                let mut spans = vec![Span::styled(
                    r.title.as_str(),
                    Style::new().fg(Color::White).bold(),
                )];
                if !r.channel.is_empty() {
                    spans.push(Span::styled(
                        format!("  {}", r.channel),
                        Style::new().fg(CHANNEL),
                    ));
                }
                if let Some(d) = r.duration {
                    spans.push(Span::styled(
                        format!("  {}", fmt_time(d)),
                        Style::new().fg(Color::Gray),
                    ));
                }
                if let Some(v) = r.views {
                    spans.push(Span::styled(
                        format!("  {} views", fmt_count(v)),
                        Style::new().fg(DIM),
                    ));
                }
                ListItem::new(Line::from(spans))
            })
            .collect();
        let list_widget = List::new(items)
            .block(block)
            .highlight_style(Style::new().bg(Color::Rgb(70, 10, 20)))
            .highlight_symbol("▶ ");
        f.render_stateful_widget(list_widget, list, &mut app.list);
    }

    let footer_line = match &app.status {
        Some((msg, true)) => Line::from(Span::styled(format!(" ✗ {msg}"), Style::new().fg(ACCENT))),
        Some((msg, false)) => Line::from(Span::styled(
            format!(" {msg}"),
            Style::new().fg(Color::Gray),
        )),
        None => hints(&[
            ("enter", "play"),
            ("/", "search"),
            ("j/k", "move"),
            ("?", "help"),
            ("q", "quit"),
        ]),
    };
    f.render_widget(Paragraph::new(footer_line), footer);
}

fn draw_player(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(Color::Black)), area);

    let video = video_area(area);
    let (fw, fh) = app.frame_dims;
    let have_frame = fw > 0 && app.frame_out.len() >= fw * fh * 3;
    if have_frame && app.renderer == Renderer::Kitty {
        // Leave the cells empty; main draws the image here after ratatui is done.
        // Overlays hide it, since the image would cover their background.
        if !app.show_help && app.busy.is_none() {
            app.image_rect = Some(fit_cells(video, fw as f64 / fh as f64, app.cell_aspect));
        }
    } else if have_frame {
        let cols = (fw as u16).min(video.width);
        let rows = ((fh / 2) as u16).min(video.height);
        let rect = Rect {
            x: video.x + (video.width - cols) / 2,
            y: video.y + (video.height - rows) / 2,
            width: cols,
            height: rows,
        };
        f.render_widget(
            HalfBlocks {
                data: &app.frame_out,
                w: fw,
                h: fh,
            },
            rect,
        );
    } else {
        let spin = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
        let error = app
            .player
            .as_ref()
            .filter(|p| !p.retrying())
            .and_then(|p| p.video_error());
        let (msg, color) = match error {
            Some(err) => (format!("✗ video stream failed: {err}"), ACCENT),
            None => (format!("{spin} buffering"), DIM),
        };
        let rect = centered(video, video.width, 1);
        f.render_widget(
            Paragraph::new(msg).fg(color).alignment(Alignment::Center),
            rect,
        );
    }

    if app.show_hud && area.height > HUD_HEIGHT {
        let hud = Rect {
            y: area.bottom() - HUD_HEIGHT,
            height: HUD_HEIGHT,
            ..area
        };
        draw_hud(f, app, hud);
    }
}

fn draw_hud(f: &mut Frame, app: &App, area: Rect) {
    let Some(p) = &app.player else { return };
    let [top, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

    // Line 1: title, progress, time, volume.
    let pos = p.position();
    let icon = if p.paused() { "⏸" } else { "▶" };
    let title: String = p.title.chars().take(48).collect();
    let left = Span::styled(
        format!(" {icon} {title} "),
        Style::new().fg(Color::White).bold(),
    );
    let time = Span::styled(
        match p.duration {
            Some(d) => format!(" {} / {} ", fmt_time(pos), fmt_time(d)),
            None => format!(" {} ", fmt_time(pos)),
        },
        Style::new().fg(Color::Gray),
    );
    let vol = Span::styled(
        format!(
            " vol {:>3}% ",
            (app.audio.shared.volume() * 100.0).round() as u32
        ),
        Style::new().fg(DIM),
    );
    let bar_w = (top.width as usize).saturating_sub(left.width() + time.width() + vol.width());
    let filled = p
        .duration
        .map_or(0, |d| ((pos / d).clamp(0.0, 1.0) * bar_w as f64) as usize);
    let line1 = Line::from(vec![
        left,
        Span::styled("━".repeat(filled), Style::new().fg(ACCENT)),
        Span::styled("─".repeat(bar_w - filled), Style::new().fg(DIM)),
        time,
        vol,
    ]);
    f.render_widget(Paragraph::new(line1), top);

    // Line 2: sync state, beat light, spectrum, preset, effect toggles.
    let hit = app.drive.pulse > 0.35;
    let sync = if app.sync {
        let style = if hit {
            Style::new().fg(Color::Black).bg(ACCENT).bold()
        } else {
            Style::new().fg(ACCENT).bold()
        };
        Span::styled(" SYNC ", style)
    } else {
        Span::styled(" FREE ", Style::new().fg(DIM).add_modifier(Modifier::BOLD))
    };
    let spectrum: String = app
        .analyzer
        .bands
        .iter()
        .map(|v| BARS[((v * 8.0).round() as usize).min(8)])
        .collect();

    let mut spans = vec![
        sync,
        Span::styled(if hit { " ● " } else { " ○ " }, Style::new().fg(ACCENT)),
        Span::styled(spectrum, Style::new().fg(Color::Rgb(255, 110, 110))),
        Span::styled(
            format!("  {} ", PRESETS[app.preset].name),
            Style::new().fg(Color::White).bold(),
        ),
        Span::styled(
            format!("×{:.1}  ", app.effects.intensity),
            Style::new().fg(DIM),
        ),
        Span::styled(
            match app.renderer {
                Renderer::Kitty => format!("{}p ", RES_STEPS[app.res]),
                Renderer::Blocks => format!("{}×{} blocks ", app.frame_dims.0, app.frame_dims.1),
            },
            Style::new().fg(Color::Gray),
        ),
    ];
    for fx in Fx::ALL {
        let style = if app.effects.is_on(fx) {
            Style::new().fg(Color::Black).bg(ACCENT)
        } else {
            Style::new().fg(DIM)
        };
        spans.push(Span::styled(format!("{}{}", fx.key(), fx.label()), style));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(" ? help", Style::new().fg(DIM)));
    f.render_widget(Paragraph::new(Line::from(spans)), bottom);
}

fn draw_help(f: &mut Frame, playing: bool) {
    let rows: &[(&str, &str)] = if playing {
        &[
            ("space", "pause / resume"),
            ("s", "toggle bass sync (off = steady fx + random hits)"),
            ("p / P", "next / previous preset"),
            ("1-9 0 - =", "toggle individual effects"),
            ("[ / ]", "effect intensity down / up"),
            ("← / →", "seek -10s / +10s"),
            ("↑ / ↓", "volume up / down"),
            ("r / R", "resolution up / down (graphics renderer)"),
            ("g", "switch pixel graphics / text blocks"),
            ("h", "hide / show HUD"),
            ("q / esc / bksp", "back to results"),
            ("ctrl-c", "quit"),
        ]
    } else {
        &[
            ("/ or i", "focus search box"),
            ("enter", "search / play selected"),
            ("j/k ↑/↓", "move selection"),
            ("pgup / pgdn", "move by 10"),
            ("g / G", "first / last result"),
            ("tab", "switch search / results"),
            ("?", "this help"),
            ("q / esc", "quit"),
        ]
    };
    let mut lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("{k:>14}  "), Style::new().fg(ACCENT).bold()),
                Span::styled(*v, Style::new().fg(Color::White)),
            ])
        })
        .collect();
    if playing {
        lines.push(Line::raw(""));
        let fx: Vec<String> = Fx::ALL
            .iter()
            .map(|fx| format!("{} {}", fx.key(), fx.label()))
            .collect();
        for chunk in fx.chunks(4) {
            lines.push(Line::styled(
                format!("  {}", chunk.join("   ")),
                Style::new().fg(Color::Gray),
            ));
        }
    }
    let rect = centered(f.area(), 60, lines.len() as u16 + 2);
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines).block(
            rounded()
                .title(" keys ")
                .border_style(Style::new().fg(ACCENT)),
        ),
        rect,
    );
}

fn hints(pairs: &[(&str, &str)]) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (k, v) in pairs {
        spans.push(Span::styled(k.to_string(), Style::new().fg(ACCENT).bold()));
        spans.push(Span::styled(format!(" {v}   "), Style::new().fg(DIM)));
    }
    Line::from(spans)
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn fmt_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}K", n as f64 / 1e3),
        1_000_000..=999_999_999 => format!("{:.1}M", n as f64 / 1e6),
        _ => format!("{:.1}B", n as f64 / 1e9),
    }
}

/// Draws an RGB frame with '▀': the foreground colour is the top pixel, background the bottom.
struct HalfBlocks<'a> {
    data: &'a [u8],
    w: usize,
    h: usize,
}

impl Widget for HalfBlocks<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let d = self.data;
        let cols = area.width.min(self.w as u16);
        let rows = area.height.min((self.h / 2) as u16);
        for cy in 0..rows {
            let top = cy as usize * 2 * self.w;
            let bottom = top + self.w;
            for cx in 0..cols {
                let t = (top + cx as usize) * 3;
                let b = (bottom + cx as usize) * 3;
                if let Some(cell) = buf.cell_mut((area.x + cx, area.y + cy)) {
                    cell.set_char('▀')
                        .set_fg(Color::Rgb(d[t], d[t + 1], d[t + 2]))
                        .set_bg(Color::Rgb(d[b], d[b + 1], d[b + 2]));
                }
            }
        }
    }
}
