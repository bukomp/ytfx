mod analysis;
mod audio;
mod effects;
mod kitty;
mod player;
mod ui;
mod youtube;

use std::io::{BufWriter, Stdout, stdout};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{
    self as crossterm_terminal, BeginSynchronizedUpdate, EndSynchronizedUpdate,
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute, queue};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;

use analysis::Analyzer;
use audio::AudioOut;
use effects::{Drive, Effects, Fx, PRESETS, Rng};
use kitty::Kitty;
use player::Player;
use youtube::{SearchResult, StreamInfo};

type Term = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

const SEARCH_COUNT: usize = 25;
const SEEK_STEP: f64 = 10.0;
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(300);
/// Video heights in pixels for the graphics renderer (r / R cycle through them).
pub const RES_STEPS: [usize; 6] = [144, 240, 360, 480, 720, 1080];
const DEFAULT_RES: usize = 3;

const USAGE: &str = "\
ytfx - terminal YouTube browser with bass-synced edit effects

usage: ytfx [options] [search terms | youtube url]

options:
  --fps <n>          video/render frame rate (default 30)
  --res <height>     video resolution for the graphics renderer:
                     144, 240, 360, 480 (default), 720 or 1080
  --renderer <name>  auto (default), kitty (pixel graphics) or blocks (text cells)
  --max-width <n>    max video width in columns for the blocks renderer (default 200)
  --no-sync          start with bass sync off
  --volume <0-200>   start volume in percent (default 80)
  --cookies-from-browser <name>
                     pass browser cookies to yt-dlp (chrome, firefox, safari, ...);
                     use this if YouTube says \"sign in to confirm you're not a bot\"
  --cookies <file>   pass a Netscape cookies.txt file to yt-dlp
  -h, --help         show this help

needs yt-dlp and ffmpeg on PATH, and a truecolor terminal.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Renderer {
    /// Half-block characters: works everywhere, 2 pixels per cell.
    Blocks,
    /// Kitty graphics protocol: real pixels at a chosen resolution.
    Kitty,
}

struct Config {
    fps: u32,
    res: usize,
    renderer: Option<Renderer>,
    max_width: u16,
    sync: bool,
    volume: f32,
    initial: Option<String>,
}

fn parse_args() -> Result<Config> {
    let mut cfg = Config {
        fps: 30,
        res: DEFAULT_RES,
        renderer: None,
        max_width: 200,
        sync: true,
        volume: 0.8,
        initial: None,
    };
    let mut words = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fps" => {
                cfg.fps = args
                    .next()
                    .context("--fps needs a value")?
                    .parse::<u32>()?
                    .clamp(5, 60)
            }
            "--max-width" => {
                cfg.max_width = args
                    .next()
                    .context("--max-width needs a value")?
                    .parse::<u16>()?
                    .max(16)
            }
            "--res" => {
                let h: usize = args.next().context("--res needs a value")?.parse()?;
                cfg.res = RES_STEPS
                    .iter()
                    .position(|&r| r == h)
                    .with_context(|| format!("--res must be one of {RES_STEPS:?}"))?;
            }
            "--renderer" => {
                cfg.renderer = match args.next().context("--renderer needs a value")?.as_str() {
                    "auto" => None,
                    "kitty" => Some(Renderer::Kitty),
                    "blocks" => Some(Renderer::Blocks),
                    other => bail!("unknown renderer `{other}` (auto, kitty, blocks)"),
                }
            }
            "--no-sync" => cfg.sync = false,
            "--volume" => {
                cfg.volume = args
                    .next()
                    .context("--volume needs a value")?
                    .parse::<f32>()?
                    / 100.0
            }
            "--cookies-from-browser" | "--cookies" => {
                let value = args
                    .next()
                    .with_context(|| format!("{arg} needs a value"))?;
                youtube::add_auth_arg(&arg, &value);
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            _ => words.push(arg),
        }
    }
    if !words.is_empty() {
        cfg.initial = Some(words.join(" "));
    }
    Ok(cfg)
}

fn check_tool(name: &str, version_flag: &str) -> Result<()> {
    let ok = Command::new(name)
        .arg(version_flag)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        bail!("`{name}` was not found on PATH (try: brew install {name})");
    }
    Ok(())
}

pub enum Focus {
    Search,
    Results,
}

pub struct App {
    cfg: Config,
    pub focus: Focus,
    pub query: String,
    pub results: Vec<SearchResult>,
    pub list: ListState,
    /// (message, is_error)
    pub status: Option<(String, bool)>,
    /// Background work in progress, shown as a spinner overlay.
    pub busy: Option<String>,
    search_rx: Option<Receiver<Result<Vec<SearchResult>>>>,
    resolve_rx: Option<Receiver<Result<StreamInfo>>>,
    pub audio: AudioOut,
    pub player: Option<Player>,
    pub effects: Effects,
    pub analyzer: Analyzer,
    pub drive: Drive,
    pub sync: bool,
    pub preset: usize,
    pub show_hud: bool,
    pub show_help: bool,
    pub frame_out: Vec<u8>,
    pub frame_dims: (usize, usize),
    pub renderer: Renderer,
    /// Present when the terminal supports the kitty graphics protocol.
    kitty: Option<Kitty>,
    pub res: usize,
    /// Cell height / width in pixels, used to keep the picture's aspect ratio.
    pub cell_aspect: f64,
    /// Where ui::draw placed the video this frame (graphics renderer only).
    pub image_rect: Option<Rect>,
    pub ticks: u64,
    tap: Vec<f32>,
    resize_at: Option<Instant>,
    rng: Rng,
    free_pulse: f32,
    quit: bool,
}

impl App {
    fn new(cfg: Config, audio: AudioOut) -> Self {
        let analyzer = Analyzer::new(audio.rate);
        let transport = Kitty::detect();
        let kitty = match cfg.renderer {
            Some(Renderer::Blocks) => None,
            // Forced on: trust the user even if detection didn't recognise the terminal.
            Some(Renderer::Kitty) => {
                Some(Kitty::new(transport.unwrap_or(kitty::Transport::Direct)))
            }
            None => transport.map(Kitty::new),
        };
        let mut app = App {
            sync: cfg.sync,
            focus: Focus::Search,
            query: String::new(),
            results: Vec::new(),
            list: ListState::default(),
            status: None,
            busy: None,
            search_rx: None,
            resolve_rx: None,
            audio,
            player: None,
            effects: Effects::new(&PRESETS[0]),
            analyzer,
            drive: Drive::default(),
            preset: 0,
            show_hud: true,
            show_help: false,
            frame_out: Vec::new(),
            frame_dims: (0, 0),
            renderer: if kitty.is_some() {
                Renderer::Kitty
            } else {
                Renderer::Blocks
            },
            kitty,
            res: cfg.res,
            cell_aspect: 2.0,
            image_rect: None,
            ticks: 0,
            tap: Vec::new(),
            resize_at: None,
            rng: Rng::new(0x2545_f491_4f6c_dd1d),
            free_pulse: 0.0,
            quit: false,
            cfg,
        };
        if let Some(q) = app.cfg.initial.take() {
            app.query = q;
            app.submit_query();
        }
        app
    }

    fn submit_query(&mut self) {
        let q = self.query.trim().to_string();
        if q.is_empty() || self.busy.is_some() {
            return;
        }
        if q.starts_with("https://") || q.starts_with("http://") {
            self.open(q.clone(), q);
            return;
        }
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(youtube::search(&q, SEARCH_COUNT));
        });
        self.search_rx = Some(rx);
        self.busy = Some("searching".into());
        self.status = None;
    }

    fn open(&mut self, url: String, title: String) {
        if self.busy.is_some() {
            return;
        }
        // Fetch a source at least as sharp as what we'll show.
        let max_height = match self.renderer {
            Renderer::Kitty => RES_STEPS[self.res].max(720),
            Renderer::Blocks => 480,
        };
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = tx.send(youtube::resolve(&url, max_height));
        });
        self.resolve_rx = Some(rx);
        let short: String = title.chars().take(50).collect();
        self.busy = Some(format!("loading {short}"));
        self.status = None;
    }

    fn close_player(&mut self, status: Option<(String, bool)>) {
        self.player = None;
        self.frame_dims = (0, 0);
        self.focus = if self.results.is_empty() {
            Focus::Search
        } else {
            Focus::Results
        };
        self.status = status;
    }

    fn poll_jobs(&mut self, area: Rect) {
        match self.search_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(res)) => {
                self.search_rx = None;
                self.busy = None;
                match res {
                    Ok(results) => {
                        self.status = results.is_empty().then(|| ("no results".into(), false));
                        self.list.select((!results.is_empty()).then_some(0));
                        self.results = results;
                        self.focus = Focus::Results;
                    }
                    Err(e) => self.status = Some((format!("{e:#}"), true)),
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.search_rx = None;
                self.busy = None;
            }
            _ => {}
        }

        match self.resolve_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(res)) => {
                self.resolve_rx = None;
                self.busy = None;
                let started = res.and_then(|info| {
                    let size = self.decode_size(area, info.aspect);
                    Player::new(info, &self.audio, size, self.cfg.fps)
                });
                match started {
                    Ok(p) => {
                        self.player = Some(p);
                        self.analyzer.reset();
                        self.effects.reset();
                        self.status = None;
                    }
                    Err(e) => self.status = Some((format!("{e:#}"), true)),
                }
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.resolve_rx = None;
                self.busy = None;
            }
            _ => {}
        }
    }

    fn tick(&mut self, dt: f32, area: Rect) {
        self.ticks += 1;
        self.poll_jobs(area);

        let Some(p) = self.player.as_mut() else {
            return;
        };
        if let Err(e) = p.retry_transient() {
            self.status = Some((format!("{e:#}"), true));
        }
        p.update();
        if let Some(end) = p.finished() {
            let status = match end {
                Ok(()) => ("finished".to_string(), false),
                Err(e) => (e, true),
            };
            self.close_player(Some(status));
            return;
        }

        // Restart the video decoder when the terminal size settles on a new value.
        let aspect = p.aspect;
        let want = self.decode_size(area, aspect);
        let Some(p) = self.player.as_mut() else {
            return;
        };
        if want == p.size() {
            self.resize_at = None;
        } else {
            match self.resize_at {
                None => self.resize_at = Some(Instant::now() + RESIZE_DEBOUNCE),
                Some(at) if Instant::now() >= at => {
                    self.resize_at = None;
                    if let Err(e) = p.resize(want) {
                        self.status = Some((format!("{e:#}"), true));
                    }
                }
                Some(_) => {}
            }
        }

        let paused = p.paused();
        let analysis = if paused {
            self.analyzer.idle(dt)
        } else {
            self.audio.shared.tap_snapshot(&mut self.tap);
            self.analyzer.update(&self.tap, dt)
        };
        self.drive = if self.sync || paused {
            analysis
        } else {
            self.free_drive(dt)
        };

        if let Some(f) = self.player.as_ref().and_then(|p| p.current.as_ref()) {
            self.effects
                .render(&f.data, f.w, f.h, self.drive, dt, &mut self.frame_out);
            self.frame_dims = (f.w, f.h);
        }
    }

    /// Pixel size the video decoder should produce for the current renderer.
    fn decode_size(&self, area: Rect, aspect: f64) -> (usize, usize) {
        match self.renderer {
            Renderer::Blocks => ui::fit(ui::video_area(area), aspect, self.cfg.max_width),
            Renderer::Kitty => {
                let h = RES_STEPS[self.res];
                (((h as f64 * aspect) as usize).max(2) & !1, h)
            }
        }
    }

    fn step_res(&mut self, up: bool) {
        if self.renderer == Renderer::Blocks {
            self.status = Some((
                "blocks renderer: resolution follows the terminal size (shrink the font for more)"
                    .into(),
                false,
            ));
            return;
        }
        self.res = if up {
            (self.res + 1).min(RES_STEPS.len() - 1)
        } else {
            self.res.saturating_sub(1)
        };
        self.resize_at = Some(Instant::now()); // apply on the next tick, no debounce
    }

    fn toggle_renderer(&mut self) {
        if self.kitty.is_none() {
            self.status = Some((
                "this terminal doesn't support the kitty graphics protocol".into(),
                false,
            ));
            return;
        }
        self.renderer = match self.renderer {
            Renderer::Kitty => Renderer::Blocks,
            Renderer::Blocks => Renderer::Kitty,
        };
        self.resize_at = Some(Instant::now());
    }

    /// Sync off: effects sit at a steady base level with random hits now and then.
    fn free_drive(&mut self, dt: f32) -> Drive {
        let beat = self.rng.f32() < dt * 0.7;
        let strength = if beat { self.rng.f32() } else { 0.0 };
        self.free_pulse = if beat {
            0.6 + 0.4 * strength
        } else {
            (self.free_pulse * (-dt * 5.0).exp()).max(0.3)
        };
        Drive {
            pulse: self.free_pulse,
            level: 0.4,
            beat,
            strength,
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.player.is_some() {
            self.player_key(k);
        } else {
            self.browse_key(k);
        }
    }

    fn browse_key(&mut self, k: KeyEvent) {
        match self.focus {
            Focus::Search => match k.code {
                KeyCode::Enter => self.submit_query(),
                KeyCode::Esc if self.results.is_empty() => self.quit = true,
                KeyCode::Esc | KeyCode::Tab | KeyCode::Down if !self.results.is_empty() => {
                    self.focus = Focus::Results
                }
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            },
            Focus::Results => {
                let len = self.results.len();
                let sel = self.list.selected().unwrap_or(0);
                let select = |i: usize| Some(i.min(len.saturating_sub(1)));
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
                    KeyCode::Char('/') | KeyCode::Char('i') | KeyCode::Tab => {
                        self.focus = Focus::Search
                    }
                    KeyCode::Char('?') => self.show_help = true,
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.list.select(select(sel.saturating_sub(1)))
                    }
                    KeyCode::Down | KeyCode::Char('j') => self.list.select(select(sel + 1)),
                    KeyCode::PageUp => self.list.select(select(sel.saturating_sub(10))),
                    KeyCode::PageDown => self.list.select(select(sel + 10)),
                    KeyCode::Home | KeyCode::Char('g') => self.list.select(select(0)),
                    KeyCode::End | KeyCode::Char('G') => self.list.select(select(len)),
                    KeyCode::Enter => {
                        if let Some(r) = self.list.selected().and_then(|i| self.results.get(i)) {
                            let (url, title) = (r.url(), r.title.clone());
                            self.open(url, title);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn player_key(&mut self, k: KeyEvent) {
        let Some(p) = self.player.as_mut() else {
            return;
        };
        let seek = |p: &mut Player, d: f64| p.seek(d).err().map(|e| (format!("{e:#}"), true));
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.close_player(None),
            KeyCode::Char(' ') => p.toggle_pause(),
            KeyCode::Left => self.status = seek(p, -SEEK_STEP),
            KeyCode::Right => self.status = seek(p, SEEK_STEP),
            KeyCode::Up => self
                .audio
                .shared
                .set_volume(self.audio.shared.volume() + 0.1),
            KeyCode::Down => self
                .audio
                .shared
                .set_volume(self.audio.shared.volume() - 0.1),
            KeyCode::Char('s') => self.sync = !self.sync,
            KeyCode::Char('p') => self.set_preset((self.preset + 1) % PRESETS.len()),
            KeyCode::Char('P') => {
                self.set_preset((self.preset + PRESETS.len() - 1) % PRESETS.len())
            }
            KeyCode::Char('[') => self.effects.intensity = (self.effects.intensity - 0.1).max(0.1),
            KeyCode::Char(']') => self.effects.intensity = (self.effects.intensity + 0.1).min(2.0),
            KeyCode::Char('h') => self.show_hud = !self.show_hud,
            KeyCode::Char('r') => self.step_res(true),
            KeyCode::Char('R') => self.step_res(false),
            KeyCode::Char('g') => self.toggle_renderer(),
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char(c) => {
                if let Some(fx) = Fx::from_key(c) {
                    self.effects.toggle(fx);
                }
            }
            _ => {}
        }
    }

    fn set_preset(&mut self, i: usize) {
        self.preset = i;
        self.effects.apply_preset(&PRESETS[i]);
    }
}

fn run(term: &mut Term, app: &mut App) -> Result<()> {
    let mut last = Instant::now();
    while !app.quit {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.1);
        last = now;

        let size = term.size()?;
        if let Ok(ws) = crossterm_terminal::window_size()
            && ws.width > 0
            && ws.columns > 0
            && ws.rows > 0
        {
            let cell_w = ws.width as f64 / ws.columns as f64;
            let cell_h = ws.height as f64 / ws.rows as f64;
            app.cell_aspect = (cell_h / cell_w).clamp(1.0, 4.0);
        }
        app.tick(dt, Rect::new(0, 0, size.width, size.height));

        // Synchronized output stops the terminal from showing half-drawn frames.
        queue!(term.backend_mut(), BeginSynchronizedUpdate)?;
        app.image_rect = None;
        term.draw(|f| ui::draw(f, app))?;
        draw_image(term, app)?;
        execute!(term.backend_mut(), EndSynchronizedUpdate)?;

        let fps = if app.player.is_some() {
            app.cfg.fps
        } else {
            20
        };
        let deadline = now + Duration::from_secs_f64(1.0 / fps as f64);
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if !event::poll(timeout)? {
                break;
            }
            if let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                app.on_key(k);
            }
            if app.quit {
                break;
            }
        }
    }
    Ok(())
}

/// Graphics renderer: put the processed frame where ui::draw left room for it.
fn draw_image(term: &mut Term, app: &mut App) -> Result<()> {
    let Some(kitty) = app.kitty.as_mut() else {
        return Ok(());
    };
    let out = term.backend_mut();
    let (w, h) = app.frame_dims;
    match app.image_rect {
        Some(rect) if w > 0 && app.frame_out.len() >= w * h * 3 => {
            kitty.draw(out, rect, &app.frame_out, w, h)?
        }
        _ => kitty.clear(out)?,
    }
    Ok(())
}

fn setup_terminal() -> Result<Term> {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        hook(info);
    }));
    enable_raw_mode()?;
    let mut out = BufWriter::with_capacity(1 << 20, stdout());
    execute!(out, EnterAlternateScreen, cursor::Hide)?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(stdout(), LeaveAlternateScreen, cursor::Show);
}

fn main() -> Result<()> {
    let cfg = parse_args()?;
    check_tool("yt-dlp", "--version")?;
    check_tool("ffmpeg", "-version")?;
    let audio = AudioOut::open()?;
    audio.shared.set_volume(cfg.volume);

    let mut app = App::new(cfg, audio);
    let mut term = setup_terminal()?;
    let result = run(&mut term, &mut app);
    if let Some(k) = app.kitty.as_mut() {
        let _ = k.clear(term.backend_mut());
        let _ = std::io::Write::flush(term.backend_mut());
    }
    drop(app);
    restore_terminal();
    result
}
