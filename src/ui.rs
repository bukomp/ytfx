use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Paragraph, Widget};

use crate::account::BROWSERS;
use crate::analysis::{Analyzer, Band, SENS_MAX, SLICE_HZ, SLICES, slice_hz};
use crate::effects::{Fx, PRESETS};
use crate::preview::{self, Slot};
use crate::textinput::TextInput;
use crate::youtube::{Entry, Kind};
use crate::{AccountUi, App, Focus, FxPage, RES_STEPS, Renderer, Tab};
use ratatui::layout::Position;

pub const HUD_HEIGHT: u16 = 2;
const ACCENT: Color = Color::Rgb(255, 40, 60);
const DIM: Color = Color::Rgb(95, 95, 110);
const CHANNEL: Color = Color::Rgb(120, 180, 255);
const SPINNER: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧"];
const BARS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
/// Narrower lists than this get no preview pane.
const PREVIEW_MIN_WIDTH: u16 = 90;

/// Width of the effects side menu.
const FX_MENU_WIDTH: u16 = 34;

/// Rows the HUD takes at the bottom of the player: none while it's hidden.
fn hud_rows(hud: bool) -> u16 {
    if hud { HUD_HEIGHT } else { 0 }
}

/// The part of the screen the video may use: everything above the HUD (all of it while
/// the HUD is hidden), left of the effects menu or above the frequency panel when one is
/// open.
pub fn video_area(area: Rect, menu: Option<FxPage>, hud: bool) -> Rect {
    let mut video = Rect {
        height: area.height.saturating_sub(hud_rows(hud)),
        ..area
    };
    match menu {
        Some(FxPage::Effects) => video.width -= fx_menu_area(area, hud).width,
        Some(FxPage::Freq) => video.height -= freq_panel_area(area, hud).height,
        None => {}
    }
    video
}

/// Where the frequency panel goes: the full width, right above the HUD. About half the
/// screen, so the columns are tall, but the video keeps a few rows.
fn freq_panel_area(area: Rect, hud: bool) -> Rect {
    let room = area.height.saturating_sub(hud_rows(hud));
    let height = (room / 2).clamp(14, 30).min(room.saturating_sub(4));
    Rect {
        y: area.y + room - height,
        height,
        ..area
    }
}

/// Where the effects menu goes: the right edge, above the HUD.
fn fx_menu_area(area: Rect, hud: bool) -> Rect {
    let width = FX_MENU_WIDTH.min(area.width / 2);
    Rect {
        x: area.right() - width,
        width,
        height: area.height.saturating_sub(hud_rows(hud)),
        ..area
    }
}

/// Which effect (index into `Fx::ALL`) a click at `pos` lands on in the menu.
pub fn fx_row_at(app: &App, pos: Position) -> Option<usize> {
    let rect = app.fx_menu_rect?;
    if !rect.contains(pos) {
        return None;
    }
    app.fx_rows.iter().find(|r| r.0 == pos.y).map(|r| r.1)
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
    // Click targets are set again by whatever gets drawn this frame.
    app.fx_button = None;
    app.fx_menu_rect = None;
    app.fx_rows.clear();
    app.fx_tabs.clear();
    app.freq_geom = None;
    // Until the first frame arrives the browser stays up, with the spinner over it.
    let loading = app
        .player
        .as_ref()
        .is_none_or(|p| p.current.is_none() && p.video_error().is_none());
    if app.show_video && !loading {
        draw_player(f, app);
    } else {
        draw_browser(f, app);
    }
    let waiting = (app.show_video && loading).then(|| "buffering".to_string());
    // Overlays hide the preview image, which would cover their background.
    if waiting.is_some() || app.busy.is_some() || app.account_ui.is_some() || app.show_help {
        app.preview_image = None;
    }
    if let Some(msg) = app.busy.as_ref().or(waiting.as_ref()) {
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
    if app.account_ui.is_some() {
        draw_account(f, app);
    }
    if app.show_help {
        draw_help(f, app.show_video);
    }
}

fn rounded() -> Block<'static> {
    Block::bordered().border_type(BorderType::Rounded)
}

fn draw_browser(f: &mut Frame, app: &mut App) {
    let search_tab = app.tab == Tab::Search;
    let [header, search, list, playing, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(if search_tab { 3 } else { 0 }),
        Constraint::Min(1),
        Constraint::Length(u16::from(app.active)),
        Constraint::Length(1),
    ])
    .areas(f.area());

    // Header: logo, tabs, account.
    let mut spans = vec![
        Span::styled(
            " ▓▒░ YTFX ░▒▓ ",
            Style::new().fg(Color::Black).bg(ACCENT).bold(),
        ),
        Span::raw(" "),
    ];
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let style = if *tab == app.tab {
            Style::new()
                .fg(Color::White)
                .bg(Color::Rgb(70, 10, 20))
                .bold()
        } else {
            Style::new().fg(DIM)
        };
        let label = match tab {
            Tab::Queue if !app.queue.items.is_empty() => {
                format!(" {} {} ({}) ", i + 1, tab.label(), app.queue.items.len())
            }
            _ => format!(" {} {} ", i + 1, tab.label()),
        };
        spans.push(Span::styled(label, style));
    }
    let account = match &app.signed_in {
        Some(how) => Span::styled(format!("● {how} "), Style::new().fg(CHANNEL)),
        None => Span::styled("L sign in ", Style::new().fg(DIM)),
    };
    // Only as wide as its text, so the tabs keep the room on narrower terminals.
    let account_width = (account.width() as u16).min(30);
    let [tabs_area, account_area] =
        Layout::horizontal([Constraint::Min(1), Constraint::Length(account_width)]).areas(header);
    f.render_widget(Paragraph::new(Line::from(spans)), tabs_area);
    f.render_widget(
        Paragraph::new(Line::from(account)).alignment(Alignment::Right),
        account_area,
    );

    let search_focused = matches!(app.focus, Focus::Search);
    app.search_rect = None;
    if search_tab {
        let field = search.inner(Margin::new(1, 1));
        app.search_rect = Some(field);
        let line = if search_focused {
            input_line(&app.query, field.width.into())
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
                    .title(format!(" search {} (ctrl-t) ", app.search_kind.label()))
                    .border_style(Style::new().fg(if search_focused { ACCENT } else { DIM })),
            ),
            search,
        );
    }

    let border = Style::new().fg(if search_focused { DIM } else { ACCENT });
    let (list, pane) =
        if app.show_preview && list.width >= PREVIEW_MIN_WIDTH && app.tab != Tab::Settings {
            let width = (list.width * 2 / 5).clamp(36, 72);
            let [list, pane] =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(width)]).areas(list);
            (list, Some(pane))
        } else {
            (list, None)
        };
    match app.tab {
        Tab::Queue => draw_queue(f, app, list, border),
        Tab::Settings => draw_settings(f, app, list, border),
        _ => draw_list(f, app, list, border),
    }
    if let Some(pane) = pane {
        draw_preview(f, app, pane);
    }

    if app.active {
        draw_now_playing(f, app, playing);
    }

    let footer_line = match &app.status {
        Some((msg, true)) => Line::from(Span::styled(format!(" ✗ {msg}"), Style::new().fg(ACCENT))),
        Some((msg, false)) => Line::from(Span::styled(
            format!(" {msg}"),
            Style::new().fg(Color::Gray),
        )),
        None if app.tab == Tab::Queue => hints(&[
            ("enter", "play"),
            ("x", "remove"),
            ("R", "radio"),
            ("S", "shuffle"),
            ("1-7", "tabs"),
            ("?", "help"),
        ]),
        None if app.tab == Tab::Settings => hints(&[
            ("←→", "delay ±10 ms"),
            ("shift", "±50 ms"),
            ("0", "no delay"),
            ("t", "sync test"),
            ("1-7", "tabs"),
            ("?", "help"),
        ]),
        None if app.tab == Tab::History => hints(&[
            ("enter", "play"),
            ("x", "forget"),
            ("r", "radio"),
            ("a", "queue"),
            ("S", "shuffle all"),
            ("1-7", "tabs"),
            ("?", "help"),
        ]),
        None => hints(&[
            ("enter", "play/open"),
            ("r", "radio"),
            ("a", "queue"),
            ("S", "shuffle all"),
            ("1-7", "tabs"),
            ("L", "account"),
            ("?", "help"),
        ]),
    };
    f.render_widget(Paragraph::new(footer_line), footer);
}

/// The settings tab: the audio output delay, a sync test (clicks and a flash) and a
/// beat light to set it by.
fn draw_settings(f: &mut Frame, app: &mut App, area: Rect, border: Style) {
    let block = rounded().title(" settings ").border_style(border);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let ms = app.audio_delay_ms;
    let max = crate::audio::MAX_DELAY_MS;
    let bar_w = inner.width.saturating_sub(8).min(50) as u32;
    let knob = (ms * bar_w.saturating_sub(1)).checked_div(max).unwrap_or(0);
    let bar: String = (0..bar_w)
        .map(|j| match j.cmp(&knob) {
            std::cmp::Ordering::Less => '━',
            std::cmp::Ordering::Equal => '●',
            std::cmp::Ordering::Greater => '─',
        })
        .collect();
    let (fill, rest) = bar.split_at(bar.char_indices().nth(knob as usize).map_or(0, |c| c.0));
    let testing = app.audio.shared.test_on();
    let light = if testing {
        Line::styled(
            "   t / enter: stop the test (leaving this tab stops it too)",
            Style::new().fg(DIM),
        )
    } else if app.player.is_some() {
        let glow = app.heard_pulse.clamp(0.0, 1.0);
        let lit = |c: u8, a: u8| (f32::from(c) + (f32::from(a) - f32::from(c)) * glow) as u8;
        Line::from(vec![
            Span::styled("   beat light  ", Style::new().fg(Color::Gray)),
            Span::styled(
                "        ",
                Style::new().bg(Color::Rgb(lit(30, 255), lit(10, 40), lit(16, 60))),
            ),
            Span::styled(
                "  or set the delay until it flashes with each kick you hear",
                Style::new().fg(DIM),
            ),
        ])
    } else {
        Line::styled(
            "   while a video plays, a beat light shows here too",
            Style::new().fg(DIM),
        )
    };
    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(" › audio delay  ", Style::new().fg(Color::White).bold()),
            Span::styled(format!("{ms} ms"), Style::new().fg(ACCENT).bold()),
        ]),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(fill.to_string(), Style::new().fg(ACCENT)),
            Span::styled(rest.to_string(), Style::new().fg(DIM)),
            Span::styled(format!(" {max} ms"), Style::new().fg(DIM)),
        ]),
        Line::raw(""),
        Line::styled(
            "   Holds back the video and the effects so they line up with what you hear.",
            Style::new().fg(Color::Gray),
        ),
        Line::styled(
            "   Bluetooth headphones usually need about 150-300 ms; wired output needs none.",
            Style::new().fg(Color::Gray),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::styled("   sync test  ", Style::new().fg(Color::White).bold()),
            Span::styled(
                if testing {
                    "on: a click every 0.6 s (every 4th higher) and a flash"
                } else {
                    "t / enter: play clicks and show a flash for each"
                },
                Style::new().fg(if testing { ACCENT } else { Color::Gray }),
            ),
        ]),
        Line::styled(
            "   set the delay until each flash comes with its click, the big ones with the high clicks",
            Style::new().fg(DIM),
        ),
        light,
        Line::raw(""),
        Line::styled(
            "   ←→ ±10 ms   shift ←→ ±50 ms   0 no delay   t test   in the player: , . ±10 ms",
            Style::new().fg(DIM),
        ),
        Line::raw(""),
        Line::from(vec![
            Span::styled("   effects  ", Style::new().fg(Color::White).bold()),
            Span::styled(app.effects.backend(), Style::new().fg(Color::Gray)),
            Span::styled("   video decoding  ", Style::new().fg(Color::White).bold()),
            Span::styled(
                if crate::player::HW_DECODE.load(std::sync::atomic::Ordering::Relaxed) {
                    "gpu when ffmpeg can (-hwaccel auto)"
                } else {
                    "cpu (--no-hwdec)"
                },
                Style::new().fg(Color::Gray),
            ),
        ]),
    ];
    let text_h = lines.len() as u16;
    f.render_widget(Paragraph::new(lines), inner);

    // The flash, under the text, timed by the clicks that have been heard.
    let room = Rect {
        y: inner.y + text_h + 1,
        height: inner.height.saturating_sub(text_h + 1),
        ..inner
    };
    if let Some(heard) = app.audio.shared.heard_test_frames().filter(|_| testing)
        && room.height >= 2
    {
        let (beat, since) = crate::audio::metronome_beat(heard, app.audio.rate);
        let accent = beat.is_multiple_of(crate::audio::ACCENT_EVERY);
        draw_sync_flash(f, room, accent, since, beat);
    }
}

/// The sync test's flash: a block that lights at each heard click and fades in 0.15 s.
/// Accented clicks get a wider, brighter block.
fn draw_sync_flash(f: &mut Frame, room: Rect, accent: bool, since: f64, beat: u64) {
    const FADE: f64 = 0.15;
    let glow = (1.0 - since / FADE).clamp(0.0, 1.0) as f32;
    let lit = |c: u8, a: u8| (f32::from(c) + (f32::from(a) - f32::from(c)) * glow) as u8;
    let color = if accent {
        Color::Rgb(lit(25, 255), lit(25, 255), lit(30, 255))
    } else {
        Color::Rgb(lit(25, 255), lit(10, 40), lit(14, 60))
    };
    let (w, h) = if accent { (40, 7) } else { (24, 5) };
    let rect = Rect {
        x: room.x + 3,
        width: w.min(room.width.saturating_sub(3)),
        height: h.min(room.height - 1),
        ..room
    };
    f.render_widget(Block::new().style(Style::new().bg(color)), rect);
    let n = crate::audio::ACCENT_EVERY;
    let count: String = (0..n)
        .map(|i| if i == beat % n { "● " } else { "○ " })
        .collect();
    f.render_widget(
        Paragraph::new(format!("   {count}")).fg(Color::Gray),
        Rect {
            y: rect.bottom(),
            height: 1,
            ..room
        },
    );
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect, border: Style) {
    let spin = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
    let stack = &app.tabs[app.tab as usize];
    let path: Vec<&str> = stack.iter().map(|v| v.title.as_str()).collect();
    let view = stack.last().expect("tab stacks are never empty");
    let mut title = format!(" {} ", path.join(" › "));
    if view.loading {
        title.push_str(&format!("{spin} "));
    } else if !view.entries.is_empty() {
        title.push_str(&format!("({}) ", view.entries.len()));
    }
    let block = rounded().title(title).border_style(border);

    if view.entries.is_empty() {
        let (msg, color) = match &view.note {
            _ if view.loading => (format!("{spin} loading"), DIM),
            Some((msg, true)) => (format!("✗ {msg}"), ACCENT),
            Some((msg, false)) => (msg.clone(), DIM),
            None if app.tab == Tab::Search => ("type a search and press enter".into(), DIM),
            None => (String::new(), DIM),
        };
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(msg)
                .fg(color)
                .alignment(Alignment::Center)
                .wrap(ratatui::widgets::Wrap { trim: true }),
            centered(inner, inner.width, inner.height.min(3)),
        );
        return;
    }

    let items: Vec<ListItem> = view.entries.iter().map(|e| entry_item(e, None)).collect();
    let list_widget = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(Color::Rgb(70, 10, 20)))
        .highlight_symbol("▶ ");
    // Render with a copy of the selection state; the original stays borrowed by `items`.
    let mut state = view.state.clone();
    f.render_stateful_widget(list_widget, area, &mut state);
    let view = app.tabs[app.tab as usize]
        .last_mut()
        .expect("tab stacks are never empty");
    view.state = state;
}

fn draw_queue(f: &mut Frame, app: &mut App, area: Rect, border: Style) {
    let q = &app.queue;
    let mut title = format!(" queue ({}) ", q.items.len());
    if q.radio {
        title.push_str("· radio ");
    }
    let block = rounded().title(title).border_style(border);
    if q.items.is_empty() {
        f.render_widget(
            Paragraph::new("empty: play a playlist or feed, press r for radio, or a to add videos")
                .fg(DIM)
                .alignment(Alignment::Center)
                .block(block),
            area,
        );
        return;
    }
    let playing = app.active.then_some(q.pos);
    let items: Vec<ListItem> = q
        .items
        .iter()
        .enumerate()
        .map(|(i, e)| entry_item(e, Some(playing == Some(i))))
        .collect();
    if app.queue_state.selected().is_none() {
        app.queue_state.select(Some(q.pos));
    }
    let list_widget = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(Color::Rgb(70, 10, 20)))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list_widget, area, &mut app.queue_state);
}

/// One list row. `playing` is Some for queue rows (marks the current video).
fn entry_item(e: &Entry, playing: Option<bool>) -> ListItem<'_> {
    let mut spans = Vec::new();
    match playing {
        Some(true) => spans.push(Span::styled("♪ ", Style::new().fg(ACCENT).bold())),
        Some(false) => spans.push(Span::raw("  ")),
        None => {}
    }
    match e.kind {
        Kind::Video => {}
        // Not ☰: Unicode 16 made the trigrams wide, and terminals disagree on that.
        Kind::Playlist => spans.push(Span::styled("≡ ", Style::new().fg(ACCENT))),
        Kind::Channel => spans.push(Span::styled("◉ ", Style::new().fg(CHANNEL))),
    }
    spans.push(Span::styled(
        e.title.as_str(),
        Style::new().fg(Color::White).bold(),
    ));
    if !e.channel.is_empty() {
        spans.push(Span::styled(
            format!("  {}", e.channel),
            Style::new().fg(CHANNEL),
        ));
    }
    if let Some(d) = e.duration {
        spans.push(Span::styled(
            format!("  {}", fmt_time(d)),
            Style::new().fg(Color::Gray),
        ));
    }
    if let Some(v) = e.views {
        spans.push(Span::styled(
            format!("  {} views", fmt_count(v)),
            Style::new().fg(DIM),
        ));
    }
    ListItem::new(Line::from(spans))
}

/// The selected entry's thumbnail and details, to the right of the list.
fn draw_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let block = rounded()
        .title(" preview ")
        .border_style(Style::new().fg(DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let entry = if app.tab == Tab::Queue {
        app.queue_state
            .selected()
            .and_then(|i| app.queue.items.get(i))
    } else {
        app.view().selected().map(|(_, e)| e)
    };
    let Some(e) = entry.cloned() else {
        return;
    };

    // Thumbnail on top, at most half the pane, keeping its aspect ratio.
    let size = match e.kind {
        Kind::Channel => preview::SQUARE,
        _ => preview::WIDE,
    };
    let mut text_area = inner;
    if inner.height >= 8 {
        let room = Rect {
            height: inner.height / 2,
            ..inner
        };
        let img = Rect {
            y: inner.y,
            ..fit_cells(room, size.0 as f64 / size.1 as f64, app.cell_aspect)
        };
        draw_thumbnail(f, app, &e, img, size);
        text_area.y = img.bottom() + 1;
        text_area.height = inner.bottom().saturating_sub(text_area.y);
    }

    let hint = match (app.tab, e.kind) {
        (Tab::Queue, _) => "enter play   x remove",
        (Tab::History, Kind::Video) => "enter play   r radio   a queue   x forget",
        (_, Kind::Video) => "enter play   r radio   a queue",
        (_, Kind::Playlist) => "enter open",
        (_, Kind::Channel) => "enter open uploads",
    };
    if text_area.height > 1 {
        let hint_row = Rect {
            y: text_area.bottom() - 1,
            height: 1,
            ..text_area
        };
        f.render_widget(Paragraph::new(hint).fg(DIM), hint_row);
        text_area.height -= 1;
    }
    f.render_widget(
        Paragraph::new(preview_lines(&e)).wrap(ratatui::widgets::Wrap { trim: true }),
        text_area,
    );
}

/// The thumbnail in `rect`: pixels for the graphics renderer (drawn by main after
/// ratatui), half blocks otherwise, or a placeholder while it loads.
fn draw_thumbnail(f: &mut Frame, app: &mut App, e: &Entry, rect: Rect, size: (usize, usize)) {
    let url = e.thumbnail_url();
    if let Some(url) = &url {
        app.preview_want = Some((url.clone(), size));
    }
    let placeholder = match url.as_deref().map(|u| (u, app.thumbs.get(u))) {
        Some((url, Some(Slot::Ready(t)))) => {
            if app.renderer == Renderer::Graphics {
                app.preview_image = Some((rect, url.to_string()));
            } else {
                let (w, h) = (rect.width as usize, rect.height as usize * 2);
                let data = preview::resample(&t.data, t.w, t.h, w, h);
                f.render_widget(HalfBlocks { data: &data, w, h }, rect);
            }
            return;
        }
        Some((_, Some(Slot::Failed))) | None => "no image".to_string(),
        Some(_) => SPINNER[(app.ticks / 2) as usize % SPINNER.len()].to_string(),
    };
    f.render_widget(Block::new().bg(Color::Rgb(22, 22, 28)), rect);
    f.render_widget(
        Paragraph::new(placeholder)
            .fg(DIM)
            .alignment(Alignment::Center),
        centered(rect, rect.width, 1),
    );
}

/// Kind badge, title, channel, numbers and description.
fn preview_lines(e: &Entry) -> Vec<Line<'_>> {
    let (label, color) = match e.kind {
        Kind::Video => ("VIDEO", Color::Gray),
        Kind::Playlist => ("PLAYLIST", ACCENT),
        Kind::Channel => ("CHANNEL", CHANNEL),
    };
    let mut badge = vec![Span::styled(
        format!(" {label} "),
        Style::new().fg(Color::Black).bg(color).bold(),
    )];
    if e.meta.live {
        badge.push(Span::raw(" "));
        badge.push(Span::styled(
            " ● LIVE ",
            Style::new().fg(Color::White).bg(ACCENT).bold(),
        ));
    }
    let mut lines = vec![
        Line::from(badge),
        Line::raw(""),
        Line::styled(e.title.as_str(), Style::new().fg(Color::White).bold()),
    ];

    let mut who = Vec::new();
    if e.kind != Kind::Channel && !e.channel.is_empty() {
        who.push(Span::styled(e.channel.as_str(), Style::new().fg(CHANNEL)));
    }
    if let Some(handle) = &e.meta.handle {
        if !who.is_empty() {
            who.push(Span::raw("  "));
        }
        who.push(Span::styled(handle.as_str(), Style::new().fg(DIM)));
    }
    if !who.is_empty() {
        lines.push(Line::from(who));
    }

    let mut stats = Vec::new();
    if let Some(d) = e.duration {
        stats.push(fmt_time(d));
    }
    if let Some(v) = e.views {
        stats.push(format!("{} views", fmt_count(v)));
    }
    if let Some(w) = e.meta.watching {
        stats.push(format!("{} watching", fmt_count(w)));
    }
    if let Some(n) = e.meta.followers {
        stats.push(format!("{} subscribers", fmt_count(n)));
    }
    if !stats.is_empty() {
        lines.push(Line::styled(
            stats.join(" · "),
            Style::new().fg(Color::Gray),
        ));
    }
    if let Some(d) = &e.meta.description {
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            d.as_str(),
            Style::new().fg(Color::Rgb(150, 150, 165)),
        ));
    }
    lines
}

/// One line at the bottom of the browser while something plays in the background.
fn draw_now_playing(f: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    match &app.player {
        Some(p) => {
            let icon = if p.paused() { "⏸" } else { "▶" };
            let title: String = p.title.chars().take(50).collect();
            spans.push(Span::styled(
                format!(" {icon} {title} "),
                Style::new().fg(Color::White).bold(),
            ));
            let time = match p.duration {
                Some(d) => format!("{} / {} ", fmt_time(p.position()), fmt_time(d)),
                None => format!("{} ", fmt_time(p.position())),
            };
            spans.push(Span::styled(time, Style::new().fg(Color::Gray)));
        }
        None => {
            let spin = SPINNER[(app.ticks / 2) as usize % SPINNER.len()];
            spans.push(Span::styled(
                format!(" {spin} loading "),
                Style::new().fg(DIM),
            ));
        }
    }
    spans.extend(queue_badge(app));
    spans.push(Span::styled(
        "  v video  space pause  n/N next/prev",
        Style::new().fg(DIM),
    ));
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(Color::Rgb(25, 8, 12))),
        area,
    );
}

/// "[3/25] RADIO" when there is a queue.
fn queue_badge(app: &App) -> Vec<Span<'static>> {
    let q = &app.queue;
    let mut spans = Vec::new();
    if q.items.len() > 1 {
        spans.push(Span::styled(
            format!("[{}/{}] ", q.pos + 1, q.items.len()),
            Style::new().fg(Color::Gray),
        ));
    }
    if q.radio {
        spans.push(Span::styled(
            "RADIO ",
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

fn draw_player(f: &mut Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::new().bg(Color::Black)), area);

    let video = video_area(area, app.open_menu(), app.show_hud);
    let (fw, fh) = app.frame_dims;
    let have_frame = fw > 0 && app.frame_out.len() >= fw * fh * 3;
    if have_frame && app.renderer == Renderer::Graphics {
        // Leave the cells empty; main draws the image here after ratatui is done.
        // Overlays hide it, since the image would cover their background.
        if !app.show_help && app.busy.is_none() && app.account_ui.is_none() {
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
    } else if app.player.is_some() {
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

    match app.open_menu() {
        Some(FxPage::Effects) => draw_fx_menu(f, app, fx_menu_area(area, app.show_hud)),
        Some(FxPage::Freq) => draw_freq_panel(f, app, freq_panel_area(area, app.show_hud)),
        None => {}
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

fn draw_fx_menu(f: &mut Frame, app: &mut App, area: Rect) {
    if area.width < 8 || area.height < 6 {
        return;
    }
    const BG: Color = Color::Rgb(14, 6, 9);
    f.render_widget(Clear, area);
    let block = rounded()
        .title(Span::styled(
            " effects ",
            Style::new().fg(Color::White).bold(),
        ))
        .title(
            Line::styled(
                format!(" {}/{} ", app.effects.count(), Fx::ALL.len()),
                Style::new().fg(ACCENT),
            )
            .right_aligned(),
        )
        .border_style(Style::new().fg(ACCENT))
        .style(Style::new().bg(BG));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.fx_menu_rect = Some(inner);

    let [tabs, head, list, foot] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .areas(inner);

    draw_page_tabs(f, app, tabs);

    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!(" {}", PRESETS[app.preset].name),
                    Style::new().fg(Color::White).bold(),
                ),
                Span::styled(
                    format!("  ×{:.1}", app.effects.intensity),
                    Style::new().fg(DIM),
                ),
            ]),
            Line::styled(" p/P preset  [ ] amt  tab page", Style::new().fg(DIM)),
        ]),
        head,
    );

    // Rows: a header before each group, then its effects. Index into Fx::ALL, or None.
    let mut rows: Vec<Option<usize>> = Vec::with_capacity(Fx::ALL.len() + 8);
    for (i, fx) in Fx::ALL.into_iter().enumerate() {
        if i == 0 || Fx::ALL[i - 1].group() != fx.group() {
            rows.push(None);
        }
        rows.push(Some(i));
    }
    let cursor = app.fx_cursor.min(Fx::ALL.len() - 1);
    let at = rows.iter().position(|&r| r == Some(cursor)).unwrap_or(0);
    // Show the group header with the first effect of a group.
    let top = if at > 0 && rows[at - 1].is_none() {
        at - 1
    } else {
        at
    };
    let visible = list.height as usize;
    let max_scroll = rows.len().saturating_sub(visible);
    if top < app.fx_scroll {
        app.fx_scroll = top;
    } else if at >= app.fx_scroll + visible {
        app.fx_scroll = at + 1 - visible;
    }
    app.fx_scroll = app.fx_scroll.min(max_scroll);

    let mut lines = Vec::with_capacity(visible);
    for (n, row) in rows.iter().skip(app.fx_scroll).take(visible).enumerate() {
        let y = list.y + n as u16;
        match *row {
            None => {
                let i = rows[app.fx_scroll + n + 1].unwrap_or(0);
                lines.push(Line::styled(
                    format!(" {}", Fx::ALL[i].group().label()),
                    Style::new().fg(DIM).add_modifier(Modifier::BOLD),
                ));
            }
            Some(i) => {
                let fx = Fx::ALL[i];
                let on = app.effects.is_on(fx);
                let here = i == cursor;
                let mark = if on {
                    Span::styled("● ", Style::new().fg(ACCENT))
                } else {
                    Span::styled("○ ", Style::new().fg(DIM))
                };
                let name = Span::styled(
                    fx.label(),
                    if on {
                        Style::new().fg(Color::White).bold()
                    } else {
                        Style::new().fg(Color::Gray)
                    },
                );
                let line = Line::from(vec![
                    Span::styled(if here { " ›" } else { "  " }, Style::new().fg(ACCENT)),
                    Span::raw(" "),
                    mark,
                    name,
                ]);
                lines.push(if here {
                    line.style(Style::new().bg(Color::Rgb(60, 14, 24)))
                } else {
                    line
                });
                app.fx_rows.push((y, i));
            }
        }
    }
    f.render_widget(Paragraph::new(lines), list);

    // Scroll hints on the border when the list doesn't fit.
    if app.fx_scroll > 0 {
        f.render_widget(
            Paragraph::new("▲").fg(DIM).alignment(Alignment::Right),
            Rect {
                y: list.y,
                height: 1,
                ..list
            },
        );
    }
    if app.fx_scroll < max_scroll {
        f.render_widget(
            Paragraph::new("▼").fg(DIM).alignment(Alignment::Right),
            Rect {
                y: list.bottom() - 1,
                height: 1,
                ..list
            },
        );
    }

    draw_menu_foot(
        f,
        foot,
        Fx::ALL[cursor].about(),
        " spc toggle  c clear  e/esc",
    );
}

fn draw_menu_foot(f: &mut Frame, foot: Rect, about: &str, keys: &str) {
    f.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "─".repeat(foot.width as usize),
                Style::new().fg(Color::Rgb(70, 20, 30)),
            ),
            Line::styled(format!(" {about}"), Style::new().fg(Color::Gray)),
            Line::styled(keys.to_string(), Style::new().fg(DIM)),
        ]),
        foot,
    );
}

/// The menu's page tabs on one row, clickable.
fn draw_page_tabs(f: &mut Frame, app: &mut App, row: Rect) {
    let mut spans = vec![Span::raw(" ")];
    let mut x = row.x + 1;
    for (page, name) in [
        (FxPage::Effects, " effects "),
        (FxPage::Freq, " frequencies "),
    ] {
        let width = name.chars().count() as u16;
        app.fx_tabs.push((
            Rect {
                x,
                width,
                height: 1,
                ..row
            },
            page,
        ));
        x += width + 1;
        spans.push(Span::styled(
            name,
            if app.fx_page == page {
                Style::new().fg(Color::Black).bg(ACCENT).bold()
            } else {
                Style::new().fg(Color::Gray)
            },
        ));
        spans.push(Span::raw(" "));
    }
    f.render_widget(
        Line::from(spans),
        Rect {
            width: (x - row.x).min(row.width),
            height: 1,
            ..row
        },
    );
}

/// Rows of the sensitivity strip: 0, 0.5, 1.0, 1.5 and 2.0 each get a row to click.
const SENS_ROWS: u16 = 5;

/// Where the frequency panel's parts are, for drawing and for mouse clicks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreqGeom {
    /// The live level columns, one per slice.
    pub levels: Rect,
    /// The sensitivity bars, one per slice, under the levels.
    pub sens: Rect,
    /// The slice in the leftmost column.
    pub first: usize,
}

impl FreqGeom {
    /// Lay out the panel's inside, scrolled so `cursor` shows. `scroll` is the
    /// previous first column. None when there is no room.
    fn new(inner: Rect, cursor: usize, scroll: usize) -> Option<FreqGeom> {
        // Header, levels, axis, sensitivity, info.
        let levels_h = inner
            .height
            .checked_sub(3 + SENS_ROWS)
            .filter(|&h| h >= 2)?;
        let cols = inner.width.checked_sub(2).filter(|&w| w >= 8)?;
        let visible = usize::from(cols).min(SLICES);
        let first = scroll
            .min(cursor)
            .max((cursor + 1).saturating_sub(visible))
            .min(SLICES - visible);
        let levels = Rect::new(inner.x + 1, inner.y + 1, visible as u16, levels_h);
        let sens = Rect {
            y: levels.bottom() + 1,
            height: SENS_ROWS,
            ..levels
        };
        Some(FreqGeom {
            levels,
            sens,
            first,
        })
    }

    /// The slice whose column is at screen column `x`.
    pub fn slice_at(&self, x: u16) -> Option<usize> {
        (self.levels.x..self.levels.right())
            .contains(&x)
            .then(|| self.first + usize::from(x - self.levels.x))
    }

    /// The sensitivity a click on screen row `y` of the strip sets: the bottom row is
    /// off, the top one SENS_MAX.
    pub fn sens_at(&self, y: u16) -> Option<f32> {
        (self.sens.y..self.sens.bottom()).contains(&y).then(|| {
            let from_bottom = f32::from(self.sens.bottom() - 1 - y);
            from_bottom / f32::from(SENS_ROWS - 1) * SENS_MAX
        })
    }
}

fn band_color(band: Band) -> Color {
    match band {
        Band::Bass => Color::Rgb(255, 60, 75),
        Band::Mid => Color::Rgb(255, 160, 60),
        Band::High => Color::Rgb(90, 200, 255),
    }
}

/// The band slice `i` is grouped in.
pub fn band_of(i: usize) -> Band {
    Band::of_slice(i)
}

/// `from` moved toward `to` by `t` (0..1).
fn mix(from: (u8, u8, u8), to: (u8, u8, u8), t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let m = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
    Color::Rgb(m(from.0, to.0), m(from.1, to.1), m(from.2, to.2))
}

/// Axis label for a column whose slice starts at `hz`.
fn axis_label(hz: u32) -> Option<String> {
    match hz {
        100 | 250 | 500 => Some(hz.to_string()),
        _ if hz >= 1_000 && hz.is_multiple_of(1_000) => Some(format!("{}k", hz / 1_000)),
        _ => None,
    }
}

/// The frequency page: a panel across the screen with a tall live level column per
/// 50 Hz slice, and under it each slice's sensitivity.
fn draw_freq_panel(f: &mut Frame, app: &mut App, area: Rect) {
    const BG: Color = Color::Rgb(14, 6, 9);
    f.render_widget(Clear, area);
    let block = rounded()
        .border_style(Style::new().fg(ACCENT))
        .style(Style::new().bg(BG));
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.fx_menu_rect = Some(inner);
    draw_page_tabs(
        f,
        app,
        Rect {
            x: area.x + 1,
            width: area.width.saturating_sub(2),
            height: 1,
            ..area
        },
    );
    app.freq_cursor = app.freq_cursor.min(SLICES - 1);
    let Some(geom) = FreqGeom::new(inner, app.freq_cursor, app.freq_scroll) else {
        return;
    };
    app.freq_scroll = geom.first;
    app.freq_geom = Some(geom);
    f.render_widget(
        FreqView {
            analyzer: &app.analyzer,
            geom,
            cursor: app.freq_cursor,
            selected: app.freq_selection(),
        },
        inner,
    );
}

/// Draws the frequency panel's inside (see `draw_freq_panel`).
struct FreqView<'a> {
    analyzer: &'a Analyzer,
    geom: FreqGeom,
    cursor: usize,
    /// The slices that keys and clicks change; includes the cursor.
    selected: std::ops::RangeInclusive<usize>,
}

impl Widget for FreqView<'_> {
    fn render(self, inner: Rect, buf: &mut Buffer) {
        let FreqView {
            analyzer,
            geom,
            cursor,
            selected,
        } = self;
        let FreqGeom {
            levels,
            sens,
            first,
        } = geom;
        const CURSOR_BG: Color = Color::Rgb(80, 18, 32);
        const SELECTED_BG: Color = Color::Rgb(45, 12, 22);
        const OFF: Color = Color::Rgb(70, 70, 82);

        // Header: the slices that are on, as ranges, and a dot that lights on beats.
        let mut spans = vec![Span::styled(" on ", Style::new().fg(DIM))];
        let mut i = 0;
        while i < SLICES {
            if analyzer.slice_sensitivity(i) == 0.0 {
                i += 1;
                continue;
            }
            let start = i;
            while i < SLICES && analyzer.slice_sensitivity(i) > 0.0 {
                i += 1;
            }
            spans.push(Span::styled(
                format!("{}-{} ", slice_hz(start), slice_hz(i)),
                Style::new().fg(band_color(band_of(start))),
            ));
        }
        if spans.len() == 1 {
            spans.push(Span::styled("nothing ", Style::new().fg(Color::Gray)));
        }
        spans.push(Span::styled("Hz  beat ", Style::new().fg(DIM)));
        let glow = analyzer.pulse();
        spans.push(Span::styled(
            "●",
            Style::new().fg(mix((40, 40, 48), (255, 40, 60), glow)),
        ));
        Line::from(spans).render(Rect { height: 1, ..inner }, buf);

        for col in 0..levels.width {
            let i = first + usize::from(col);
            let x = levels.x + col;
            let v = analyzer.slice_sensitivity(i);
            let on = v > 0.0;
            let here = i == cursor;
            let chosen = selected.contains(&i);
            let color = if on { band_color(band_of(i)) } else { OFF };
            // A slice's own hits flash its level column white.
            let flash = match color {
                Color::Rgb(r, g, b) => mix((r, g, b), (255, 255, 255), analyzer.slice_glow(i)),
                c => c,
            };
            let bg = match (here, chosen) {
                (true, _) => Some(CURSOR_BG),
                (false, true) => Some(SELECTED_BG),
                _ => None,
            };

            // A vertical bar of `eighths`, bottom up, in `rect`'s column x.
            let mut bar = |rect: Rect, eighths: usize, fg: Color| {
                for r in 0..rect.height {
                    let y = rect.bottom() - 1 - r;
                    let filled = eighths.saturating_sub(usize::from(r) * 8).min(8);
                    let cell = &mut buf[(x, y)];
                    cell.set_char(BARS[filled]).set_fg(fg);
                    if let Some(bg) = bg {
                        cell.set_bg(bg);
                    }
                }
            };
            let level = analyzer.slice_levels[i];
            bar(
                levels,
                (level * f32::from(levels.height) * 8.0).round() as usize,
                flash,
            );
            bar(
                sens,
                (v / SENS_MAX * f32::from(SENS_ROWS) * 8.0).round() as usize,
                if chosen && on { Color::White } else { color },
            );
            if !on {
                buf[(x, sens.bottom() - 1)].set_char('·').set_fg(OFF);
            }

            // Axis between the two: a line, band edges, frequency labels.
            let axis = &mut buf[(x, levels.bottom())];
            let edge = i > 0 && band_of(i) != band_of(i - 1);
            let (ch, fg) = match (edge, chosen) {
                (true, _) => ('┼', Color::Gray),
                (false, true) => ('━', ACCENT),
                (false, false) => ('─', Color::Rgb(70, 20, 30)),
            };
            axis.set_char(ch).set_fg(fg);
        }
        // Labels over the axis line, skipping any that would overlap the last one.
        let mut free_from = levels.x;
        for col in 0..levels.width {
            let i = first + usize::from(col);
            let x = levels.x + col;
            let Some(label) = axis_label(slice_hz(i)) else {
                continue;
            };
            let w = label.len() as u16;
            if x < free_from || x + w > levels.right() {
                continue;
            }
            for (k, ch) in label.chars().enumerate() {
                buf[(x + k as u16, levels.bottom())]
                    .set_char(ch)
                    .set_fg(Color::Gray);
            }
            free_from = x + w + 1;
        }
        // The cursor's mark, unless a label is there (its column is highlighted anyway).
        if let Some(col) = cursor
            .checked_sub(first)
            .filter(|&c| c < usize::from(levels.width))
        {
            let cell = &mut buf[(levels.x + col as u16, levels.bottom())];
            if matches!(cell.symbol(), "─" | "━" | "┼") {
                cell.set_char('▲').set_fg(Color::White);
            }
        }

        // Info: the selected slices, then the keys.
        let (lo, hi) = (*selected.start(), *selected.end());
        let fmt = |v: f32| {
            if v == 0.0 {
                "off".to_string()
            } else {
                format!("{v:.1}")
            }
        };
        let (min, max) = selected
            .clone()
            .map(|i| analyzer.slice_sensitivity(i))
            .fold((f32::MAX, 0.0f32), |(a, b), v| (a.min(v), b.max(v)));
        let bands: Vec<&str> = Band::ALL
            .into_iter()
            .filter(|b| b.slices().any(|i| selected.contains(&i)))
            .map(Band::label)
            .collect();
        let mut spans = vec![Span::styled(
            format!(" {}-{} Hz ", slice_hz(lo), slice_hz(hi) + SLICE_HZ),
            Style::new().fg(Color::White).bold(),
        )];
        if hi > lo {
            // Several slices: only they are heard (see `App::solo`).
            spans.extend([
                Span::styled(
                    format!("{} slices ", hi - lo + 1),
                    Style::new().fg(Color::Gray),
                ),
                Span::styled("♪ solo ", Style::new().fg(ACCENT).bold()),
            ]);
        }
        spans.extend([
            Span::styled(
                format!("{} ", bands.join("+")),
                Style::new().fg(band_color(band_of(cursor))),
            ),
            Span::styled(
                if min == max {
                    fmt(min)
                } else {
                    format!("{}-{}", fmt(min), fmt(max))
                },
                Style::new().fg(Color::Gray),
            ),
            Span::styled(
                "   ←→ pick  opt ←→ ×10  shift ←→ select  a band  ↑↓ set  0 off  spc default  c reset all  tab effects",
                Style::new().fg(DIM),
            ),
        ]);
        let info = Line::from(spans);
        info.render(
            Rect {
                y: inner.bottom() - 1,
                height: 1,
                ..inner
            },
            buf,
        );
    }
}

fn draw_hud(f: &mut Frame, app: &mut App, area: Rect) {
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
    let badge = queue_badge(app);
    let badge_w: usize = badge.iter().map(Span::width).sum();
    let bar_w = (top.width as usize)
        .saturating_sub(left.width() + time.width() + vol.width() + badge_w + 1);
    let filled = p
        .duration
        .map_or(0, |d| ((pos / d).clamp(0.0, 1.0) * bar_w as f64) as usize);
    let mut line1 = vec![
        left,
        Span::styled("━".repeat(filled), Style::new().fg(ACCENT)),
        Span::styled("─".repeat(bar_w - filled), Style::new().fg(DIM)),
        time,
        Span::raw(" "),
    ];
    line1.extend(badge);
    line1.push(vol);
    let line1 = Line::from(line1);
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
                Renderer::Graphics => format!("{}p {} ", RES_STEPS[app.res], app.graphics_name()),
                Renderer::Blocks => format!("{}×{} blocks ", app.frame_dims.0, app.frame_dims.1),
            },
            Style::new().fg(Color::Gray),
        ),
        Span::styled(
            if app.effects.on_gpu() { "gpu " } else { "cpu " },
            Style::new().fg(DIM),
        ),
    ];
    // Names of the effects that are on, as many as fit before the menu button.
    let button = Span::styled(
        if app.fx_menu {
            format!(" ▸ FX {} ", app.effects.count())
        } else {
            format!(" ◂ FX {} ", app.effects.count())
        },
        if app.fx_menu {
            Style::new().fg(Color::Black).bg(ACCENT).bold()
        } else {
            Style::new()
                .fg(Color::White)
                .bg(Color::Rgb(90, 14, 26))
                .bold()
        },
    );
    let help = Span::styled("? help ", Style::new().fg(DIM));
    let room = (bottom.width as usize).saturating_sub(
        spans.iter().map(Span::width).sum::<usize>() + button.width() + help.width() + 1,
    );
    let mut names = String::new();
    for fx in app.effects.enabled() {
        let more = if names.is_empty() { 0 } else { 1 };
        if names.len() + more + fx.label().len() + 2 > room {
            names.push_str(" …");
            break;
        }
        if more == 1 {
            names.push(' ');
        }
        names.push_str(fx.label());
    }
    spans.push(Span::styled(names, Style::new().fg(ACCENT)));
    f.render_widget(Paragraph::new(Line::from(spans)), bottom);

    let bw = (button.width() + help.width()) as u16;
    if bottom.width > bw {
        let rect = Rect {
            x: bottom.right() - bw,
            width: bw,
            ..bottom
        };
        f.render_widget(Paragraph::new(Line::from(vec![help, button.clone()])), rect);
        app.fx_button = Some(Rect {
            x: bottom.right() - button.width() as u16,
            width: button.width() as u16,
            ..bottom
        });
    }
}

fn draw_help(f: &mut Frame, playing: bool) {
    let rows: &[(&str, &str)] = if playing {
        &[
            ("space", "pause / resume"),
            ("n / N", "next / previous video in the queue"),
            ("m", "radio: queue videos like this one"),
            ("z", "shuffle the rest of the queue"),
            ("b / tab", "browse while it keeps playing"),
            (
                "s",
                "toggle sync to the slices on (off = steady fx + random hits)",
            ),
            ("p / P", "next / previous preset"),
            ("e / FX button", "effects menu: pick any of the effects"),
            (
                "  in the menu",
                "↑↓ jk move, space/enter toggle, c clear all",
            ),
            (
                "  tab in the menu",
                "freq panel: ←→ pick (opt ×10), shift ←→ / a select, ↑↓ set",
            ),
            ("[ / ]", "effect intensity down / up"),
            (", / .", "audio delay -10 / +10 ms (bluetooth lag)"),
            ("← / →", "seek -10s / +10s"),
            ("↑ / ↓", "volume up / down"),
            ("r / R", "resolution up / down (graphics renderer)"),
            ("g", "switch pixel graphics / text blocks"),
            ("f", "blurred background around narrow videos on / off"),
            ("h", "hide / show HUD"),
            ("q / esc / bksp", "stop and go back"),
            ("ctrl-c", "quit"),
        ]
    } else {
        &[
            (
                "1-7 / tab",
                "search, home, subscriptions, library, queue, history, settings",
            ),
            ("/ or i", "search (ctrl-t: videos / playlists / channels)"),
            ("enter", "play video / open playlist or channel"),
            ("esc / bksp / h", "back out of a playlist or channel"),
            ("j/k ↑/↓", "move selection (pgup/pgdn, g/G)"),
            ("r", "radio from the selected video"),
            ("R", "radio from a random video in this list"),
            ("S", "shuffle-play this list"),
            ("a", "add the selected video to the queue"),
            ("x", "remove from the queue / forget from history"),
            ("u", "reload this list"),
            ("p", "preview pane on / off (wide terminals)"),
            ("L", "YouTube account (sign in / out)"),
            ("v", "back to the playing video"),
            ("space / n / N", "pause, next, previous while playing"),
            ("q / esc", "quit"),
        ]
    };
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::styled(format!("{k:>14}  "), Style::new().fg(ACCENT).bold()),
                Span::styled(*v, Style::new().fg(Color::White)),
            ])
        })
        .collect();
    let rect = centered(f.area(), 68, lines.len() as u16 + 2);
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

fn draw_account(f: &mut Frame, app: &App) {
    let key = |k: &'static str, v: &'static str| {
        Line::from(vec![
            Span::styled(format!("  {k:<4}"), Style::new().fg(ACCENT).bold()),
            Span::styled(v, Style::new().fg(Color::White)),
        ])
    };
    let note = |text: &'static str| Line::styled(text, Style::new().fg(Color::Gray));
    let mut lines = vec![
        Line::from(match &app.signed_in {
            Some(how) => Span::styled(format!("signed in with {how}"), Style::new().fg(CHANNEL)),
            None => Span::styled("not signed in", Style::new().fg(DIM)),
        }),
        Line::raw(""),
    ];
    match app.account_ui {
        Some(AccountUi::Menu) | None => {
            lines.extend([
                key("f", "import a cookies.txt file (recommended)"),
                key("b", "let yt-dlp read a browser's cookies"),
                key("o", "sign out and delete imported cookies"),
                key("esc", "close"),
                Line::raw(""),
                note("Signing in shows your recommendations, subscriptions and"),
                note("library. ytfx never reads browser data unless you pick b."),
                Line::raw(""),
                note("For f: in a browser signed in to YouTube, export cookies"),
                note("from a youtube.com tab with a cookies.txt extension (for"),
                note("example \"Get cookies.txt LOCALLY\"). ytfx keeps only the"),
                note("youtube.com cookies, in a private file in ~/.config/ytfx."),
                Line::raw(""),
                note("YouTube can rate-limit or flag accounts used by tools like"),
                note("yt-dlp; a secondary account is the safer choice."),
            ]);
        }
        Some(AccountUi::Path(ref path)) => {
            lines.extend([
                note("Path of the exported cookies.txt (~ works):"),
                {
                    // Inside the popup's border, after the "> ".
                    let cols = f.area().width.min(66).saturating_sub(4);
                    let mut line = input_line(path, cols.into());
                    line.spans
                        .insert(0, Span::styled("> ", Style::new().fg(ACCENT)));
                    line
                },
                Line::raw(""),
                note("enter import   esc back"),
            ]);
        }
        Some(AccountUi::Browser(sel)) => {
            for (i, b) in BROWSERS.iter().enumerate() {
                let style = if i == sel {
                    Style::new()
                        .fg(Color::White)
                        .bg(Color::Rgb(70, 10, 20))
                        .bold()
                } else {
                    Style::new().fg(Color::Gray)
                };
                lines.push(Line::styled(format!("  {b}"), style));
            }
            lines.extend([
                Line::raw(""),
                note("yt-dlp reads this browser's cookie store every time it"),
                note("runs. On macOS, Chromium browsers (chrome, edge, brave,"),
                note("...) ask for Keychain access to decrypt it; Safari needs"),
                note("Full Disk Access for the terminal; Firefox needs neither."),
                Line::raw(""),
                note("enter choose   j/k move   esc back"),
            ]);
        }
    }
    if let Some((msg, err)) = &app.account_msg {
        lines.push(Line::raw(""));
        let color = if *err { ACCENT } else { CHANNEL };
        lines.push(Line::styled(msg.clone(), Style::new().fg(color)));
    }
    // Lines wrap inside the 64-column interior.
    let rows: usize = lines.iter().map(|l| l.width().max(1).div_ceil(64)).sum();
    let rect = centered(f.area(), 66, rows as u16 + 2);
    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                rounded()
                    .title(" youtube account ")
                    .border_style(Style::new().fg(ACCENT)),
            ),
        rect,
    );
}

/// A focused text field: the part around the cursor that fits in `cols`, with a
/// block cursor.
fn input_line(input: &TextInput, cols: usize) -> Line<'_> {
    let (before, at, after) = input.view(cols);
    Line::from(vec![
        Span::raw(before),
        Span::styled(at.unwrap_or(" "), Style::new().fg(Color::Black).bg(ACCENT)),
        Span::raw(after),
    ])
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of screen row `y` of `buf` within `area`.
    fn row(buf: &Buffer, area: Rect, y: u16) -> String {
        (area.x..area.right())
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn freq_columns_show_level_and_sensitivity() {
        let mut a = Analyzer::new(48_000);
        a.set_slice_sensitivity(0, SENS_MAX);
        a.set_slice_sensitivity(1, 0.5);
        a.set_slice_sensitivity(2, 0.0);
        a.slice_levels[0] = 1.0;
        a.slice_levels[1] = 0.5;
        // Header, 4 level rows, axis, 5 sensitivity rows, info.
        let inner = Rect::new(1, 1, 30, 12);
        let geom = FreqGeom::new(inner, 1, 0).unwrap();
        assert_eq!(geom.levels, Rect::new(2, 2, 28, 4));
        assert_eq!(geom.sens, Rect::new(2, 7, 28, 5));
        let mut buf = Buffer::empty(inner);
        FreqView {
            analyzer: &a,
            geom,
            cursor: 1,
            selected: 1..=1,
        }
        .render(inner, &mut buf);
        let col = |x: u16, rows: std::ops::Range<u16>| -> String {
            rows.map(|y| buf[(x, y)].symbol().to_string()).collect()
        };
        // Levels top-down: full, half; sensitivity: full, a quarter, off.
        assert_eq!(col(2, 2..6), "████");
        assert_eq!(col(3, 2..6), "  ██");
        assert_eq!(col(2, 7..12), "█████");
        assert_eq!(col(3, 7..12), "   ▂█");
        assert_eq!(col(4, 7..12), "    ·");
        // The axis marks the cursor, the 100 Hz label and the bass / mid edge.
        let axis = row(&buf, inner, 6);
        assert_eq!(axis.chars().take(8).collect::<String>(), " ─▲100┼─");
        assert!(row(&buf, inner, 12).starts_with(" 50-100 Hz bass 0.5"));
    }

    #[test]
    fn freq_header_lists_the_slices_that_are_on() {
        let mut a = Analyzer::new(48_000);
        a.set_slice_sensitivity(20, 1.0);
        let inner = Rect::new(0, 0, 60, 12);
        let geom = FreqGeom::new(inner, 0, 0).unwrap();
        let mut buf = Buffer::empty(inner);
        FreqView {
            analyzer: &a,
            geom,
            cursor: 0,
            selected: 0..=0,
        }
        .render(inner, &mut buf);
        assert!(
            row(&buf, inner, 0).starts_with(" on 0-150 1000-1050 Hz  beat ●"),
            "{}",
            row(&buf, inner, 0)
        );
    }

    #[test]
    fn freq_selection_is_marked_and_summed_up() {
        let mut a = Analyzer::new(48_000);
        a.set_slice_sensitivity(1, 0.4);
        let inner = Rect::new(0, 0, 50, 12);
        let geom = FreqGeom::new(inner, 6, 0).unwrap();
        let mut buf = Buffer::empty(inner);
        FreqView {
            analyzer: &a,
            geom,
            cursor: 6,
            selected: 1..=6,
        }
        .render(inner, &mut buf);
        let axis: String = row(&buf, inner, 5).chars().take(9).collect();
        // Slice 2 is under the 100 label, 5 is the bass / mid edge, 6 the cursor.
        assert_eq!(axis, " ─━100┼▲─");
        let info = row(&buf, inner, 11);
        assert!(
            info.starts_with(" 50-350 Hz 6 slices ♪ solo bass+mid off-1.0"),
            "{info}"
        );
        // Selected columns are shaded, the cursor more.
        assert_eq!(buf[(2, 1)].bg, Color::Rgb(45, 12, 22));
        assert_eq!(buf[(7, 1)].bg, Color::Rgb(80, 18, 32));
        assert_eq!(buf[(8, 1)].bg, Color::Reset);
    }

    #[test]
    fn freq_clicks_map_to_slice_and_value() {
        let inner = Rect::new(0, 0, 40, 14);
        let geom = FreqGeom::new(inner, 0, 0).unwrap();
        assert_eq!(geom.slice_at(geom.levels.x), Some(0));
        assert_eq!(geom.slice_at(geom.levels.x + 7), Some(7));
        assert_eq!(geom.slice_at(geom.levels.right()), None);
        let s = geom.sens;
        assert_eq!(geom.sens_at(s.y), Some(SENS_MAX));
        assert_eq!(geom.sens_at(s.y + 2), Some(1.0));
        assert_eq!(geom.sens_at(s.bottom() - 1), Some(0.0));
        assert_eq!(geom.sens_at(s.bottom()), None);
    }

    #[test]
    fn freq_panel_scrolls_to_the_cursor() {
        let inner = Rect::new(0, 0, 42, 14);
        assert_eq!(FreqGeom::new(inner, 100, 0).unwrap().first, 61);
        assert_eq!(FreqGeom::new(inner, 3, 61).unwrap().first, 3);
        let last = FreqGeom::new(inner, SLICES - 1, 0).unwrap();
        assert_eq!(last.first, SLICES - 40);
        assert_eq!(last.slice_at(last.levels.right() - 1), Some(SLICES - 1));
        // Too small: no panel.
        assert!(FreqGeom::new(Rect::new(0, 0, 42, 9), 0, 0).is_none());
    }
}
