mod account;
mod analysis;
mod audio;
mod effects;
mod history;
mod kitty;
mod player;
mod preview;
mod queue;
mod settings;
mod textinput;
mod ui;
mod youtube;

use std::io::{BufWriter, Stdout, stdout};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::crossterm::terminal::{
    self as crossterm_terminal, BeginSynchronizedUpdate, EndSynchronizedUpdate,
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::crossterm::{cursor, execute, queue};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;

use account::Account;
use analysis::Analyzer;
use audio::AudioOut;
use effects::{Backdrop, Drive, Effects, Fx, PRESETS, Rng};
use history::History;
use kitty::Kitty;
use player::Player;
use queue::{Queue, RADIO_LOW};
use textinput::TextInput;
use youtube::{Entry, Kind, SearchKind, StreamInfo};

type Term = Terminal<CrosstermBackend<BufWriter<Stdout>>>;

const SEARCH_COUNT: usize = 25;
/// Most entries fetched for a feed, playlist or channel.
const LIST_LIMIT: usize = 100;
const MIX_LIMIT: usize = 25;
const SEEK_STEP: f64 = 10.0;
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(300);
/// Check for baked-in black bars every this many frames; use a new crop once this many
/// checks in a row agree (dark scenes and fades shouldn't make it jump).
const CROP_EVERY: u64 = 8;
const CROP_VOTES: u32 = 3;
/// Consecutive videos that may fail to play before the queue stops.
const MAX_FAILS: u32 = 3;
/// Resolve the next video once the current one has played this long.
const PREFETCH_AFTER: f64 = 5.0;
/// Stream URLs expire after a few hours; don't use older prefetched ones.
const PREFETCH_MAX_AGE: Duration = Duration::from_secs(3600);
const RADIO_RETRY: Duration = Duration::from_secs(3);
/// Fetches in a row that found nothing new before radio gives up waiting.
const RADIO_MAX_MISSES: u32 = 5;
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
  --cookies <file>   use a Netscape cookies.txt for this session only
  --cookies-from-browser <name>
                     let yt-dlp read this browser's cookies, this session only
  -h, --help         show this help

To stay signed in, press L in the app.
Needs yt-dlp and ffmpeg on PATH, and a truecolor terminal.";

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
    /// Cookies from the command line; they override the saved account.
    auth: Option<youtube::Auth>,
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
        auth: None,
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
            "--cookies-from-browser" => {
                let name = args
                    .next()
                    .context("--cookies-from-browser needs a value")?;
                cfg.auth = Some(youtube::Auth::Browser(name));
            }
            "--cookies" => {
                let file = args.next().context("--cookies needs a value")?;
                cfg.auth = Some(youtube::Auth::CookiesFile(file.into()));
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Search,
    Home,
    Subs,
    Library,
    Queue,
    History,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Search,
        Tab::Home,
        Tab::Subs,
        Tab::Library,
        Tab::Queue,
        Tab::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Search => "search",
            Tab::Home => "home",
            Tab::Subs => "subscriptions",
            Tab::Library => "library",
            Tab::Queue => "queue",
            Tab::History => "history",
        }
    }

    fn root(self) -> ListView {
        match self {
            Tab::Search => ListView::new("results", None),
            Tab::Home => ListView::new("home", Some(youtube::HOME.into())),
            Tab::Subs => ListView::new("subscriptions", Some(youtube::SUBSCRIPTIONS.into())),
            Tab::Library => ListView::fixed("library", youtube::library()),
            Tab::Queue => ListView::fixed("queue", Vec::new()),
            Tab::History => ListView::fixed("history", Vec::new()),
        }
    }
}

/// One list on screen: search results, a feed, a playlist or a channel's videos.
pub struct ListView {
    pub title: String,
    target: Option<String>,
    pub entries: Vec<Entry>,
    pub state: ListState,
    pub loading: bool,
    pub loaded: bool,
    /// Error or empty-list message.
    pub note: Option<(String, bool)>,
    token: u64,
}

impl ListView {
    fn new(title: &str, target: Option<String>) -> Self {
        ListView {
            title: title.into(),
            target,
            entries: Vec::new(),
            state: ListState::default(),
            loading: false,
            loaded: false,
            note: None,
            token: 0,
        }
    }

    fn fixed(title: &str, entries: Vec<Entry>) -> Self {
        let mut v = ListView::new(title, None);
        v.state.select((!entries.is_empty()).then_some(0));
        v.entries = entries;
        v.loaded = true;
        v
    }

    pub fn selected(&self) -> Option<(usize, &Entry)> {
        let i = self.state.selected()?;
        Some((i, self.entries.get(i)?))
    }
}

pub enum AccountUi {
    Menu,
    /// Typing the path of a cookies.txt.
    Path(TextInput),
    /// Picking a browser from account::BROWSERS.
    Browser(usize),
}

/// Results of background yt-dlp work, tagged with the token of the request.
enum Msg {
    Listed(u64, Result<Vec<Entry>>),
    Resolved(u64, Result<StreamInfo>),
    Mix(u64, Result<Vec<Entry>>),
    /// A preview thumbnail: its URL, decode size and pixels.
    Thumb(String, (usize, usize), Result<Vec<u8>>),
}

/// The next queued video, resolved ahead of time so it starts without a wait.
struct Prefetch {
    id: String,
    token: u64,
    height: usize,
    at: Instant,
    result: Option<Result<StreamInfo>>,
}

struct RadioStart {
    seed: Entry,
    /// Start playing the seed (from the browser), or keep the current video (from the player).
    play: bool,
}

pub struct App {
    cfg: Config,
    pub tab: Tab,
    /// Per tab, the root list and the lists opened from it.
    pub tabs: [Vec<ListView>; 6],
    pub focus: Focus,
    pub query: TextInput,
    /// Where the search field's text is on screen, for mouse clicks. Set while drawing.
    pub search_rect: Option<Rect>,
    pub search_kind: SearchKind,
    pub queue: Queue,
    pub queue_state: ListState,
    /// Videos played here, newest first; shown in the history tab.
    history: History,
    /// (message, is_error)
    pub status: Option<(String, bool)>,
    /// Work the user is waiting for, shown as a spinner overlay.
    pub busy: Option<String>,
    pub account_ui: Option<AccountUi>,
    pub account_msg: Option<(String, bool)>,
    /// How we're signed in, if at all.
    pub signed_in: Option<String>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    next_token: u64,
    resolving: Option<u64>,
    prefetch: Option<Prefetch>,
    mix_token: Option<u64>,
    radio_start: Option<RadioStart>,
    radio_retry_at: Option<Instant>,
    radio_misses: u32,
    /// Playback ran out and radio is fetching more.
    awaiting_radio: bool,
    fails: u32,
    /// A playback session is running (possibly between videos).
    pub active: bool,
    /// The video screen is shown; otherwise the browser is, with playback continuing.
    pub show_video: bool,
    pub audio: AudioOut,
    pub player: Option<Player>,
    pub effects: Effects,
    pub analyzer: Analyzer,
    pub drive: Drive,
    pub sync: bool,
    pub preset: usize,
    pub show_hud: bool,
    pub show_help: bool,
    /// The effects side menu is open, its selected effect (index into `Fx::ALL`) and
    /// first visible row.
    pub fx_menu: bool,
    pub fx_cursor: usize,
    pub fx_scroll: usize,
    /// Set by ui::draw for mouse clicks: the FX button, the menu's inside, and which
    /// effect each menu row shows (screen row, index into `Fx::ALL`).
    pub fx_button: Option<Rect>,
    pub fx_menu_rect: Option<Rect>,
    pub fx_rows: Vec<(u16, usize)>,
    /// What's shown: the effects output, on a backdrop canvas when `fill` is on.
    pub frame_out: Vec<u8>,
    pub frame_dims: (usize, usize),
    fx_out: Vec<u8>,
    /// While paused: what the shown frame was made from. Effects only run again when
    /// this changes (a seek, another effect set, a resize), not on every tick.
    paused_key: Option<u64>,
    backdrop: Backdrop,
    /// Fill space around the video with a blurred, darkened copy of it.
    pub fill: bool,
    /// Picture area inside black bars baked into the video, once stable.
    crop: Option<effects::Crop>,
    /// Latest detected crop and how many detections in a row agreed with it.
    crop_vote: (effects::Crop, u32),
    pub renderer: Renderer,
    /// Present when the terminal supports the kitty graphics protocol.
    kitty: Option<Kitty>,
    pub res: usize,
    /// Cell height / width in pixels, used to keep the picture's aspect ratio.
    pub cell_aspect: f64,
    /// Where ui::draw placed the video this frame (graphics renderer only).
    pub image_rect: Option<Rect>,
    /// Preview pane next to the browser lists (when the terminal is wide enough).
    pub show_preview: bool,
    pub thumbs: preview::Thumbs,
    /// Set by ui::draw: the thumbnail the pane shows and its decode size.
    pub preview_want: Option<(String, (usize, usize))>,
    /// Set by ui::draw: where the graphics renderer should put that thumbnail.
    pub preview_image: Option<(Rect, String)>,
    pub ticks: u64,
    area: Rect,
    tap: Vec<f32>,
    resize_at: Option<Instant>,
    rng: Rng,
    free_pulse: f32,
    quit: bool,
}

impl App {
    fn new(cfg: Config, audio: AudioOut, signed_in: Option<String>) -> Self {
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
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x2545_f491_4f6c_dd1d, |d| d.as_nanos() as u64);
        let (tx, rx) = mpsc::channel();
        let mut app = App {
            sync: cfg.sync,
            tab: Tab::Search,
            tabs: Tab::ALL.map(|t| vec![t.root()]),
            focus: Focus::Search,
            query: TextInput::default(),
            search_rect: None,
            search_kind: SearchKind::Videos,
            queue: Queue::default(),
            queue_state: ListState::default(),
            history: History::load(),
            status: None,
            busy: None,
            account_ui: None,
            account_msg: None,
            signed_in,
            tx,
            rx,
            next_token: 0,
            resolving: None,
            prefetch: None,
            mix_token: None,
            radio_start: None,
            radio_retry_at: None,
            radio_misses: 0,
            awaiting_radio: false,
            fails: 0,
            active: false,
            show_video: false,
            audio,
            player: None,
            effects: Effects::new(&PRESETS[0]),
            analyzer,
            drive: Drive::default(),
            preset: 0,
            show_hud: true,
            show_help: false,
            fx_menu: false,
            fx_cursor: 0,
            fx_scroll: 0,
            fx_button: None,
            fx_menu_rect: None,
            fx_rows: Vec::new(),
            frame_out: Vec::new(),
            frame_dims: (0, 0),
            fx_out: Vec::new(),
            paused_key: None,
            backdrop: Backdrop::default(),
            fill: true,
            crop: None,
            crop_vote: ((0, 0, 0, 0), 0),
            renderer: if kitty.is_some() {
                Renderer::Kitty
            } else {
                Renderer::Blocks
            },
            kitty,
            res: cfg.res,
            cell_aspect: 2.0,
            image_rect: None,
            show_preview: true,
            thumbs: preview::Thumbs::default(),
            preview_want: None,
            preview_image: None,
            ticks: 0,
            area: Rect::default(),
            tap: Vec::new(),
            resize_at: None,
            rng: Rng::new(seed),
            free_pulse: 0.0,
            quit: false,
            cfg,
        };
        app.apply_settings(settings::load());
        app.sync_history();
        if let Some(q) = app.cfg.initial.take() {
            app.query.set(&q);
            app.submit_query();
        } else if app.signed_in.is_some() {
            app.switch_tab(Tab::Home);
        }
        app
    }

    // ---- background jobs ----

    fn spawn<T: Send + 'static>(
        &mut self,
        job: impl FnOnce() -> T + Send + 'static,
        wrap: fn(u64, T) -> Msg,
    ) -> u64 {
        self.next_token += 1;
        let token = self.next_token;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let _ = tx.send(wrap(token, job()));
        });
        token
    }

    fn poll_messages(&mut self) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Listed(token, result) => self.on_listed(token, result),
                Msg::Resolved(token, result) => self.on_resolved(token, result),
                Msg::Mix(token, result) => self.on_mix(token, result),
                Msg::Thumb(url, size, result) => self.thumbs.done(url, size, result),
            }
        }
    }

    // ---- lists ----

    pub fn view(&self) -> &ListView {
        self.tabs[self.tab as usize]
            .last()
            .expect("tab stacks are never empty")
    }

    fn view_mut(&mut self) -> &mut ListView {
        self.tabs[self.tab as usize]
            .last_mut()
            .expect("tab stacks are never empty")
    }

    /// Fetch the entries of the current tab's top list.
    fn load_view(&mut self) {
        let signed_in = self.signed_in.is_some();
        let view = self.view_mut();
        let Some(target) = view.target.clone() else {
            return;
        };
        if youtube::needs_account(&target) && !signed_in {
            view.note = Some(("sign in to see this: press L".into(), false));
            return;
        }
        let limit = if self.tab == Tab::Search && self.tabs[Tab::Search as usize].len() == 1 {
            SEARCH_COUNT
        } else {
            LIST_LIMIT
        };
        let token = self.spawn(move || youtube::list(&target, limit), Msg::Listed);
        let view = self.view_mut();
        view.token = token;
        view.loading = true;
        view.note = None;
    }

    fn on_listed(&mut self, token: u64, result: Result<Vec<Entry>>) {
        let Some(view) = self
            .tabs
            .iter_mut()
            .flatten()
            .find(|v| v.loading && v.token == token)
        else {
            return;
        };
        view.loading = false;
        view.loaded = true;
        match result {
            Ok(entries) => {
                let empty = match view.target.as_deref() {
                    Some(t) if youtube::needs_account(t) => {
                        "nothing here: if that's unexpected, the sign-in may have expired (L)"
                    }
                    _ => "nothing here",
                };
                view.note = entries.is_empty().then(|| (empty.into(), false));
                view.state.select((!entries.is_empty()).then_some(0));
                view.entries = entries;
            }
            Err(e) => view.note = Some((format!("{e:#}"), true)),
        }
    }

    fn switch_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.focus = Focus::Results;
        let view = self.view();
        if !view.loaded && !view.loading && view.note.is_none() {
            self.load_view();
        }
    }

    fn push_view(&mut self, title: String, target: String) {
        self.tabs[self.tab as usize].push(ListView::new(&title, Some(target)));
        self.load_view();
    }

    fn submit_query(&mut self) {
        let q = self.query.as_str().trim().to_string();
        if q.is_empty() {
            return;
        }
        self.tab = Tab::Search;
        self.tabs[Tab::Search as usize].truncate(1);
        self.focus = Focus::Results;
        self.status = None;
        if q.starts_with("https://") || q.starts_with("http://") {
            self.open_url(q);
            return;
        }
        let kind = self.search_kind;
        let view = self.view_mut();
        view.title = format!("{} for “{q}”", kind.label());
        view.target = Some(kind.target(&q, SEARCH_COUNT));
        view.entries.clear();
        self.load_view();
    }

    /// A pasted URL: playlists and channels open as lists, anything else plays.
    fn open_url(&mut self, url: String) {
        let path = match youtube::is_youtube_target(&url) {
            true => url.split_once("youtube.com/").map_or("", |(_, p)| p),
            false => "",
        };
        let is_list = path.starts_with("playlist?")
            || ["@", "channel/", "c/", "user/"]
                .iter()
                .any(|p| path.starts_with(p));
        if is_list {
            self.push_view("link".into(), youtube::list_url(&url));
            return;
        }
        match video_id(&url) {
            Some(id) => {
                let entry = Entry {
                    kind: Kind::Video,
                    target: id,
                    title: url.clone(),
                    channel: String::new(),
                    duration: None,
                    views: None,
                    meta: Default::default(),
                };
                self.play_entries(&[entry], 0);
            }
            // Not YouTube: yt-dlp may still know the site. No queue for it.
            None => {
                self.queue = Queue::default();
                self.begin_playback();
                self.start_resolve(url.clone(), url, None);
            }
        }
    }

    // ---- playback ----

    fn begin_playback(&mut self) {
        self.active = true;
        self.show_video = true;
        self.fails = 0;
        self.awaiting_radio = false;
    }

    fn play_entries(&mut self, entries: &[Entry], start: usize) {
        self.queue.set(entries, start);
        self.begin_playback();
        self.play_current();
    }

    fn play_current(&mut self) {
        let Some(entry) = self.queue.current().cloned() else {
            return;
        };
        self.start_resolve(entry.url(), entry.title, Some(entry.target));
    }

    fn max_height(&self) -> usize {
        // Fetch a source at least as sharp as what we'll show.
        match self.renderer {
            Renderer::Kitty => RES_STEPS[self.res].max(720),
            Renderer::Blocks => 480,
        }
    }

    fn start_resolve(&mut self, url: String, title: String, id: Option<String>) {
        self.player = None;
        self.frame_dims = (0, 0);
        let short: String = title.chars().take(50).collect();
        self.busy = Some(format!("loading {short}"));

        let height = self.max_height();
        if let Some(id) = &id
            && let Some(pf) = self
                .prefetch
                .take_if(|pf| &pf.id == id && pf.height == height)
        {
            match pf.result {
                Some(Ok(info)) if pf.at.elapsed() < PREFETCH_MAX_AGE => {
                    self.busy = None;
                    self.start_player(info);
                    return;
                }
                None => {
                    // Still resolving: wait for that request instead of starting another.
                    self.resolving = Some(pf.token);
                    return;
                }
                _ => {}
            }
        }
        self.resolving = Some(self.spawn(move || youtube::resolve(&url, height), Msg::Resolved));
    }

    fn on_resolved(&mut self, token: u64, result: Result<StreamInfo>) {
        if self.resolving == Some(token) {
            self.resolving = None;
            self.busy = None;
            match result {
                Ok(info) => self.start_player(info),
                Err(e) => self.track_failed(format!("{e:#}")),
            }
        } else if let Some(pf) = self.prefetch.as_mut().filter(|pf| pf.token == token) {
            pf.result = Some(result);
        }
    }

    fn start_player(&mut self, info: StreamInfo) {
        if !self.active {
            return;
        }
        let size = self.decode_size(self.area, info.aspect);
        match Player::new(info, &self.audio, size, self.cfg.fps) {
            Ok(p) => {
                self.remember_played(&p);
                self.player = Some(p);
                self.crop = None;
                self.crop_vote.1 = 0;
                self.analyzer.reset();
                self.effects.reset();
            }
            Err(e) => self.track_failed(format!("{e:#}")),
        }
    }

    /// Add the video that just started to the history (YouTube videos only: other sites
    /// have no queue entry to replay from).
    fn remember_played(&mut self, p: &Player) {
        let Some(mut entry) = self.queue.current().cloned() else {
            return;
        };
        // Pasted links use the URL as a stand-in title until the real one is known.
        if entry.title.starts_with("http") && !p.title.is_empty() {
            entry.title = p.title.clone();
        }
        entry.duration = entry.duration.or(p.duration);
        self.history.record(&entry);
        if let Err(e) = self.history.save() {
            self.status = Some((format!("couldn't save history: {e:#}"), true));
        }
        self.sync_history();
    }

    /// Refresh the history tab's list, keeping the selection on the same video.
    fn sync_history(&mut self) {
        let view = &mut self.tabs[Tab::History as usize][0];
        let selected = view.selected().map(|(_, e)| e.target.clone());
        view.entries = self.history.entries();
        let i = selected
            .and_then(|id| view.entries.iter().position(|e| e.target == id))
            .or_else(|| view.state.selected())
            .map(|i| i.min(view.entries.len().saturating_sub(1)))
            .unwrap_or(0);
        view.state.select((!view.entries.is_empty()).then_some(i));
        view.note = view.entries.is_empty().then(|| {
            (
                "nothing played yet: videos you watch show up here".into(),
                false,
            )
        });
    }

    /// The current video couldn't play: skip it, unless too many failed in a row.
    fn track_failed(&mut self, err: String) {
        self.player = None;
        self.fails += 1;
        if self.fails < MAX_FAILS && (self.queue.remaining() > 0 || self.queue.radio) {
            self.status = Some((format!("skipped: {err}"), true));
            self.next_track();
        } else {
            self.stop_playback(Some((err, true)));
        }
    }

    /// Move on after the current video, automatically.
    fn next_track(&mut self) {
        self.player = None;
        self.frame_dims = (0, 0);
        if self.queue.advance().is_some() {
            self.play_current();
        } else if self.queue.radio {
            self.awaiting_radio = true;
            self.radio_retry_at = None;
            self.busy = Some("radio: finding more".into());
        } else {
            self.stop_playback(Some(("finished".into(), false)));
        }
    }

    /// `n` / `N`: skip forward or back by hand.
    fn skip(&mut self, forward: bool) {
        if !self.active || self.resolving.is_some() || self.awaiting_radio {
            return;
        }
        let moved = if forward {
            self.queue.advance().is_some()
        } else {
            self.queue.back().is_some()
        };
        if moved {
            self.fails = 0;
            self.play_current();
        } else if forward && self.queue.radio {
            self.next_track();
        } else {
            let edge = if forward { "last" } else { "first" };
            self.status = Some((format!("this is the {edge} video in the queue"), false));
        }
    }

    fn stop_playback(&mut self, status: Option<(String, bool)>) {
        self.player = None;
        self.frame_dims = (0, 0);
        self.active = false;
        self.show_video = false;
        self.resolving = None;
        self.awaiting_radio = false;
        self.busy = None;
        self.focus = Focus::Results;
        self.status = status;
    }

    fn prefetch_next(&mut self) {
        let started = self
            .player
            .as_ref()
            .is_some_and(|p| p.position() > PREFETCH_AFTER);
        let Some(next) = self.queue.peek_next() else {
            return;
        };
        let height = self.max_height();
        if !started
            || self
                .prefetch
                .as_ref()
                .is_some_and(|pf| pf.id == next.target && pf.height == height)
        {
            return;
        }
        let (id, url) = (next.target.clone(), next.url());
        let token = self.spawn(move || youtube::resolve(&url, height), Msg::Resolved);
        self.prefetch = Some(Prefetch {
            id,
            token,
            height,
            at: Instant::now(),
            result: None,
        });
    }

    // ---- radio ----

    fn start_radio(&mut self, seed: Entry, play: bool) {
        if seed.kind != Kind::Video || self.radio_start.is_some() {
            return;
        }
        let from = seed.clone();
        let msg = format!("radio: finding videos like {}", seed.title);
        if play {
            self.busy = Some(msg);
        } else {
            self.status = Some((msg, false));
        }
        self.mix_token = Some(self.spawn(move || youtube::radio(&from, MIX_LIMIT), Msg::Mix));
        self.radio_start = Some(RadioStart { seed, play });
    }

    /// Keep a radio queue topped up from Mixes of recently played videos.
    fn refill_radio(&mut self) {
        let due = self.radio_retry_at.is_none_or(|at| Instant::now() >= at);
        if !self.queue.radio
            || self.mix_token.is_some()
            || self.queue.remaining() >= RADIO_LOW
            || !due
        {
            return;
        }
        let Some(seed) = self.queue.radio_seed(&mut self.rng).cloned() else {
            return;
        };
        self.mix_token = Some(self.spawn(move || youtube::radio(&seed, MIX_LIMIT), Msg::Mix));
    }

    fn on_mix(&mut self, token: u64, result: Result<Vec<Entry>>) {
        if self.mix_token != Some(token) {
            return;
        }
        self.mix_token = None;
        if let Some(start) = self.radio_start.take() {
            self.busy = None;
            match result {
                Ok(mix) if mix.iter().all(|e| e.target == start.seed.target) => {
                    self.status = Some(("radio: found no videos like that one".into(), false));
                }
                Ok(mix) => {
                    self.queue.start_radio(start.seed, mix, &mut self.rng);
                    self.radio_retry_at = None;
                    self.radio_misses = 0;
                    self.status = Some((
                        format!("radio: {} videos queued", self.queue.remaining()),
                        false,
                    ));
                    if start.play {
                        self.begin_playback();
                        self.play_current();
                    }
                }
                Err(e) => self.status = Some((format!("radio: {e:#}"), true)),
            }
            return;
        }

        let added = match result {
            Ok(mix) => self.queue.extend_radio(mix, &mut self.rng),
            Err(e) => {
                self.status = Some((format!("radio: {e:#}"), true));
                0
            }
        };
        if added > 0 {
            self.radio_misses = 0;
            self.radio_retry_at = None;
            if self.awaiting_radio {
                self.awaiting_radio = false;
                self.busy = None;
                self.next_track();
            }
        } else if self.awaiting_radio && self.radio_misses >= RADIO_MAX_MISSES {
            self.stop_playback(Some(("radio ran out of new videos".into(), false)));
        } else {
            // Nothing new from that seed: try another one, backing off while there's no hurry.
            self.radio_misses += 1;
            let delay = if self.awaiting_radio {
                Duration::from_secs(1)
            } else {
                RADIO_RETRY * (1 << self.radio_misses.min(6))
            };
            self.radio_retry_at = Some(Instant::now() + delay);
        }
    }

    // ---- account ----

    fn set_account(&mut self, result: Result<(Account, String)>) {
        match result.and_then(|(account, msg)| Ok((account.auth()?, account, msg))) {
            Ok((auth, account, msg)) => {
                youtube::set_auth(Some(auth));
                self.signed_in = Some(account.describe());
                self.reset_account_tabs();
                self.account_ui = None;
                self.status = Some((msg, false));
                self.switch_tab(Tab::Home);
            }
            Err(e) => self.account_msg = Some((format!("{e:#}"), true)),
        }
    }

    fn sign_out(&mut self) {
        match account::sign_out() {
            Ok(()) => {
                youtube::set_auth(None);
                self.signed_in = None;
                self.reset_account_tabs();
                self.account_msg = Some(("signed out, imported cookies deleted".into(), false));
            }
            Err(e) => self.account_msg = Some((format!("{e:#}"), true)),
        }
    }

    fn reset_account_tabs(&mut self) {
        for tab in [Tab::Home, Tab::Subs, Tab::Library] {
            self.tabs[tab as usize] = vec![tab.root()];
        }
    }

    fn account_key(&mut self, k: KeyEvent) {
        let Some(ui) = self.account_ui.as_mut() else {
            return;
        };
        match ui {
            AccountUi::Menu => match k.code {
                KeyCode::Char('f') => {
                    *ui = AccountUi::Path(TextInput::default());
                    self.account_msg = None;
                }
                KeyCode::Char('b') => {
                    *ui = AccountUi::Browser(0);
                    self.account_msg = None;
                }
                KeyCode::Char('o') => self.sign_out(),
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('L') => {
                    self.account_ui = None;
                    self.account_msg = None;
                }
                _ => {}
            },
            AccountUi::Path(path) => match k.code {
                KeyCode::Esc => *ui = AccountUi::Menu,
                _ if path.handle(k) => {}
                KeyCode::Enter => {
                    let path = account::expand_home(path.as_str());
                    let result = account::import_cookies(&path).map(|n| {
                        let msg = format!(
                            "signed in: kept {n} youtube.com cookies; you can delete the exported file"
                        );
                        (Account::CookiesFile, msg)
                    });
                    self.set_account(result);
                }
                _ => {}
            },
            AccountUi::Browser(i) => match k.code {
                KeyCode::Esc => *ui = AccountUi::Menu,
                KeyCode::Up | KeyCode::Char('k') => *i = i.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    *i = (*i + 1).min(account::BROWSERS.len() - 1)
                }
                KeyCode::Enter => {
                    let account = Account::Browser(account::BROWSERS[*i].into());
                    let result = account::save(&account).map(|()| {
                        let msg = format!("signed in with {}", account.describe());
                        (account, msg)
                    });
                    self.set_account(result);
                }
                _ => {}
            },
        }
    }

    // ---- frame loop ----

    fn tick(&mut self, dt: f32, area: Rect) {
        self.ticks += 1;
        self.area = area;
        self.poll_messages();
        let want = self.preview_want.clone();
        if let Some(url) = self
            .thumbs
            .poll(want.as_ref().map(|w| w.0.as_str()), Instant::now())
        {
            let size = want.map_or(preview::WIDE, |w| w.1);
            self.spawn(
                move || {
                    let result = preview::fetch(&url, size);
                    (url, size, result)
                },
                |_, (url, size, result)| Msg::Thumb(url, size, result),
            );
        }
        if self.active {
            self.refill_radio();
            self.prefetch_next();
        }

        let Some(p) = self.player.as_mut() else {
            return;
        };
        if let Err(e) = p.retry_transient() {
            self.status = Some((format!("{e:#}"), true));
        }
        p.update();
        if let Some(end) = p.finished() {
            match end {
                Ok(()) => {
                    self.fails = 0;
                    self.next_track();
                }
                Err(e) => self.track_failed(e),
            }
            return;
        }

        // Restart the video decoder when the terminal size settles on a new value.
        let aspect = p.aspect;
        let want = self.decode_size(area, aspect);
        let Some(p) = self.player.as_mut() else {
            return;
        };
        if want == p.size() || !self.show_video {
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

        if !self.show_video {
            return;
        }
        if let Some(f) = self.player.as_ref().and_then(|p| p.current.as_ref()) {
            let key = paused.then(|| {
                use std::hash::{DefaultHasher, Hash, Hasher};
                let mut h = DefaultHasher::new();
                (f.pts.to_bits(), f.w, f.h, self.effects.enabled()).hash(&mut h);
                (self.effects.intensity.to_bits(), self.fill, self.crop, area).hash(&mut h);
                (self.renderer == Renderer::Kitty, self.fx_menu).hash(&mut h);
                self.cell_aspect.to_bits().hash(&mut h);
                // Keep rendering until the last hit has faded out, then hold still.
                ((self.drive.pulse * 20.0) as u32).hash(&mut h);
                h.finish()
            });
            if key.is_some() && key == self.paused_key && !self.frame_out.is_empty() {
                return;
            }
            self.paused_key = key;
            // Look for baked-in black bars on the clean frame (effects add grain to them).
            if self.fill && self.ticks.is_multiple_of(CROP_EVERY) {
                let found = effects::content_rect(&f.data, f.w, f.h);
                let close = |a: usize, b: usize| a.abs_diff(b) <= 2;
                let (prev, n) = self.crop_vote;
                let same = close(found.0, prev.0)
                    && close(found.1, prev.1)
                    && close(found.2, prev.2)
                    && close(found.3, prev.3);
                self.crop_vote = (found, if same { n + 1 } else { 1 });
                if self.crop_vote.1 >= CROP_VOTES {
                    self.crop = Some(found);
                }
            }
            let crop = self
                .crop
                .filter(|c| c.2 <= f.w && c.3 <= f.h)
                .unwrap_or((0, 0, f.w, f.h));
            self.effects
                .render(&f.data, f.w, f.h, self.drive, dt, &mut self.fx_out);
            match self.fill_canvas(area, f.w, f.h, crop) {
                Some((cw, ch)) => {
                    self.backdrop.compose(
                        &self.fx_out,
                        f.w,
                        f.h,
                        crop,
                        cw,
                        ch,
                        &mut self.frame_out,
                    );
                    self.frame_dims = (cw, ch);
                }
                None => {
                    std::mem::swap(&mut self.fx_out, &mut self.frame_out);
                    self.frame_dims = (f.w, f.h);
                }
            }
        }
    }

    /// Canvas size in pixels that covers the whole video area around a fw x fh frame,
    /// when the frame alone would leave empty space (and fill is on).
    fn fill_canvas(
        &self,
        area: Rect,
        fw: usize,
        fh: usize,
        crop: effects::Crop,
    ) -> Option<(usize, usize)> {
        let video = ui::video_area(area, self.fx_menu);
        if !self.fill || video.width == 0 || video.height == 0 {
            return None;
        }
        let (cw, ch) = match self.renderer {
            // One cell is one pixel wide and two tall.
            Renderer::Blocks => (video.width as usize, video.height as usize * 2),
            Renderer::Kitty => {
                // Area aspect in pixels; grow the frame to it in one direction.
                let aspect = video.width as f64 / (video.height as f64 * self.cell_aspect);
                if aspect > fw as f64 / fh as f64 {
                    ((fh as f64 * aspect) as usize, fh)
                } else {
                    (fw, (fw as f64 / aspect) as usize)
                }
            }
        };
        // Nothing worth filling, or the frame is from before a resize and doesn't fit.
        let picture = (crop.2 - crop.0) * (crop.3 - crop.1);
        let gain = (cw * ch) as f64 / picture as f64;
        (cw >= fw && ch >= fh && gain > 1.02).then_some((cw, ch))
    }

    fn apply_settings(&mut self, s: settings::Settings) {
        if let Some(i) = s
            .preset
            .and_then(|name| PRESETS.iter().position(|p| p.name == name))
        {
            self.set_preset(i);
        }
        if let Some(labels) = s.effects {
            let fxs: Vec<Fx> = labels.iter().filter_map(|l| Fx::from_label(l)).collect();
            self.effects.set_enabled(&fxs);
        }
        if let Some(i) = s.intensity.filter(|i| i.is_finite()) {
            self.effects.intensity = i.clamp(0.1, 2.0);
        }
        // --no-sync on the command line wins over the saved choice.
        self.sync = self.sync && s.sync.unwrap_or(true);
        self.fill = s.fill.unwrap_or(true);
        self.show_preview = s.preview.unwrap_or(true);
    }

    fn save_settings(&mut self) {
        let s = settings::Settings {
            effects: Some(
                self.effects
                    .enabled()
                    .iter()
                    .map(|fx| fx.label().to_string())
                    .collect(),
            ),
            preset: Some(PRESETS[self.preset].name.to_string()),
            intensity: Some((self.effects.intensity * 10.0).round() / 10.0),
            sync: Some(self.sync),
            fill: Some(self.fill),
            preview: Some(self.show_preview),
        };
        if let Err(e) = settings::save(&s) {
            self.status = Some((format!("couldn't save settings: {e:#}"), true));
        }
    }

    /// Pixel size the video decoder should produce for the current renderer.
    fn decode_size(&self, area: Rect, aspect: f64) -> (usize, usize) {
        match self.renderer {
            Renderer::Blocks => ui::fit(
                ui::video_area(area, self.fx_menu),
                aspect,
                self.cfg.max_width,
            ),
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

    // ---- keys ----

    fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.account_ui.is_some() {
            self.account_key(k);
        } else if self.show_video {
            self.player_key(k);
        } else {
            self.browse_key(k);
        }
    }

    fn browse_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if matches!(self.focus, Focus::Search) {
            match k.code {
                KeyCode::Enter => self.submit_query(),
                KeyCode::Char('t') if ctrl => self.search_kind = self.search_kind.next(),
                KeyCode::Esc | KeyCode::Tab | KeyCode::Down => self.focus = Focus::Results,
                _ => {
                    self.query.handle(k);
                }
            }
            return;
        }

        let len = if self.tab == Tab::Queue {
            self.queue.items.len()
        } else {
            self.view().entries.len()
        };
        let state = if self.tab == Tab::Queue {
            &mut self.queue_state
        } else {
            &mut self.view_mut().state
        };
        let sel = state.selected().unwrap_or(0);
        let last = len.saturating_sub(1);
        let mut select = |i: usize| state.select((len > 0).then_some(i.min(last)));
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => return select(sel.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => return select(sel + 1),
            KeyCode::PageUp => return select(sel.saturating_sub(10)),
            KeyCode::PageDown => return select(sel + 10),
            KeyCode::Home | KeyCode::Char('g') => return select(0),
            KeyCode::End | KeyCode::Char('G') => return select(last),
            _ => {}
        }

        let depth = self.tabs[self.tab as usize].len();
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') if depth > 1 => {
                self.tabs[self.tab as usize].pop();
            }
            KeyCode::Esc => self.quit = true,
            KeyCode::Char('/') | KeyCode::Char('i') => {
                self.tab = Tab::Search;
                self.focus = Focus::Search;
            }
            KeyCode::Char(c @ '1'..='6') => self.switch_tab(Tab::ALL[c as usize - '1' as usize]),
            KeyCode::Tab => self.switch_tab(Tab::ALL[(self.tab as usize + 1) % Tab::ALL.len()]),
            KeyCode::BackTab => {
                self.switch_tab(Tab::ALL[(self.tab as usize + Tab::ALL.len() - 1) % Tab::ALL.len()])
            }
            KeyCode::Char('t') if self.tab == Tab::Search => {
                self.search_kind = self.search_kind.next();
                self.submit_query();
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Char('r') => {
                if let Some(e) = self.selected_entry() {
                    self.start_radio(e, true);
                }
            }
            KeyCode::Char('R') => self.random_radio(),
            KeyCode::Char('S') => self.shuffle_play(),
            KeyCode::Char('a') => self.enqueue(),
            KeyCode::Char('x') | KeyCode::Delete if self.tab == Tab::Queue => {
                if let Some(i) = self.queue_state.selected()
                    && !self.queue.remove(i)
                {
                    self.status = Some(("can't remove the playing video".into(), false));
                }
            }
            KeyCode::Char('x') | KeyCode::Delete if self.tab == Tab::History && depth == 1 => {
                if let Some((_, e)) = self.view().selected() {
                    let id = e.target.clone();
                    self.history.remove(&id);
                    if let Err(e) = self.history.save() {
                        self.status = Some((format!("couldn't save history: {e:#}"), true));
                    }
                    self.sync_history();
                }
            }
            KeyCode::Char('u') => {
                if matches!(self.tab, Tab::Library | Tab::History) && depth == 1 {
                    return;
                }
                self.view_mut().loaded = false;
                self.load_view();
            }
            KeyCode::Char('L') => {
                self.account_ui = Some(AccountUi::Menu);
                self.account_msg = None;
            }
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('p') => {
                self.show_preview = !self.show_preview;
                self.save_settings();
            }
            // Playback continues while browsing.
            KeyCode::Char('v') if self.active => self.show_video = true,
            KeyCode::Char(' ') if self.player.is_some() => {
                if let Some(p) = &self.player {
                    p.toggle_pause()
                }
            }
            KeyCode::Char('n') => self.skip(true),
            KeyCode::Char('N') => self.skip(false),
            _ => {}
        }
    }

    fn selected_entry(&self) -> Option<Entry> {
        if self.tab == Tab::Queue {
            let i = self.queue_state.selected()?;
            return self.queue.items.get(i).cloned();
        }
        self.view().selected().map(|(_, e)| e.clone())
    }

    fn activate(&mut self) {
        if self.tab == Tab::Queue {
            if let Some(i) = self.queue_state.selected()
                && self.queue.jump(i).is_some()
            {
                self.begin_playback();
                self.play_current();
            }
            return;
        }
        // Search results and history play one at a time.
        let single = matches!(self.tab, Tab::Search | Tab::History)
            && self.tabs[self.tab as usize].len() == 1;
        let Some((i, entry)) = self.view().selected() else {
            return;
        };
        let entry = entry.clone();
        match entry.kind {
            // Feeds and playlists play through.
            Kind::Video if single => self.play_entries(&[entry], 0),
            Kind::Video => {
                let entries = self.view().entries.clone();
                self.play_entries(&entries, i);
            }
            _ => self.push_view(entry.title.clone(), entry.list_target()),
        }
    }

    fn videos_here(&self) -> Vec<Entry> {
        let entries = if self.tab == Tab::Queue {
            &self.queue.items
        } else {
            &self.view().entries
        };
        entries
            .iter()
            .filter(|e| e.kind == Kind::Video)
            .cloned()
            .collect()
    }

    /// Radio seeded from a random video of the current list.
    fn random_radio(&mut self) {
        let videos = self.videos_here();
        if videos.is_empty() {
            self.status = Some(("no videos in this list to start a radio from".into(), false));
            return;
        }
        let seed = videos[self.rng.below(videos.len())].clone();
        self.start_radio(seed, true);
    }

    fn shuffle_play(&mut self) {
        let videos = self.videos_here();
        if videos.is_empty() {
            return;
        }
        self.queue.set(&videos, 0);
        self.queue.shuffle_all(&mut self.rng);
        self.begin_playback();
        self.play_current();
    }

    fn enqueue(&mut self) {
        match self.selected_entry() {
            Some(e) if e.kind == Kind::Video && self.tab != Tab::Queue => {
                let title = e.title.clone();
                self.queue.push(e);
                self.status = Some((
                    format!("queued {title} ({} in queue)", self.queue.items.len()),
                    false,
                ));
            }
            Some(e) if e.kind != Kind::Video => {
                self.status = Some(("open it (enter) to play or queue its videos".into(), false))
            }
            _ => {}
        }
    }

    /// Keys for the open effects menu. Returns false for keys it leaves to the player.
    fn fx_menu_key(&mut self, k: KeyEvent) -> bool {
        let n = Fx::ALL.len();
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => self.fx_cursor = (self.fx_cursor + n - 1) % n,
            KeyCode::Down | KeyCode::Char('j') => self.fx_cursor = (self.fx_cursor + 1) % n,
            KeyCode::PageUp => self.fx_cursor = self.fx_cursor.saturating_sub(8),
            KeyCode::PageDown => self.fx_cursor = (self.fx_cursor + 8).min(n - 1),
            KeyCode::Home => self.fx_cursor = 0,
            KeyCode::End => self.fx_cursor = n - 1,
            KeyCode::Char(' ') | KeyCode::Enter => self.toggle_fx(self.fx_cursor),
            KeyCode::Char('c') => {
                self.effects.set_enabled(&[]);
                self.save_settings();
            }
            KeyCode::Esc | KeyCode::Char('e') => self.fx_menu = false,
            _ => return false,
        }
        true
    }

    fn toggle_fx(&mut self, i: usize) {
        if let Some(&fx) = Fx::ALL.get(i) {
            self.fx_cursor = i;
            self.effects.toggle(fx);
            self.save_settings();
        }
    }

    /// Pasted text (bracketed paste) goes into the text field on screen; in the
    /// browser it goes to the search, so a pasted YouTube link is one enter away.
    fn on_paste(&mut self, text: &str) {
        if self.show_help {
            return;
        }
        match &mut self.account_ui {
            Some(AccountUi::Path(path)) => path.insert(text),
            Some(_) => {}
            None if self.show_video => {}
            None => {
                self.tab = Tab::Search;
                self.focus = Focus::Search;
                self.query.insert(text);
            }
        }
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let pos = Position::new(m.column, m.row);
        if self.show_help {
            if matches!(m.kind, MouseEventKind::Down(_)) {
                self.show_help = false;
            }
            return;
        }
        if !self.show_video
            && self.account_ui.is_none()
            && m.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some(r) = self.search_rect
        {
            if r.contains(pos) {
                self.focus = Focus::Search;
                self.query.click(usize::from(pos.x - r.x));
            } else if matches!(self.focus, Focus::Search) {
                self.focus = Focus::Results;
            }
            return;
        }
        if !self.show_video || self.account_ui.is_some() {
            return;
        }
        let in_menu = self.fx_menu_rect.is_some_and(|r| r.contains(pos));
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.fx_button.is_some_and(|r| r.contains(pos)) {
                    self.fx_menu = !self.fx_menu;
                } else if let Some(i) = ui::fx_row_at(self, pos) {
                    self.toggle_fx(i);
                }
            }
            MouseEventKind::ScrollUp if in_menu => {
                self.fx_cursor = self.fx_cursor.saturating_sub(1);
            }
            MouseEventKind::ScrollDown if in_menu => {
                self.fx_cursor = (self.fx_cursor + 1).min(Fx::ALL.len() - 1);
            }
            _ => {}
        }
    }

    fn player_key(&mut self, k: KeyEvent) {
        if self.fx_menu && self.fx_menu_key(k) {
            return;
        }
        match k.code {
            KeyCode::Char('e') => {
                self.fx_menu = true;
                return;
            }
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => {
                return self.stop_playback(None);
            }
            KeyCode::Char('b') | KeyCode::Tab => {
                self.show_video = false;
                return;
            }
            KeyCode::Char('n') | KeyCode::Char('>') => return self.skip(true),
            KeyCode::Char('N') | KeyCode::Char('<') => return self.skip(false),
            KeyCode::Char('m') => {
                match self.queue.current().cloned() {
                    Some(e) if self.player.is_some() => self.start_radio(e, false),
                    _ => self.status = Some(("radio needs a playing YouTube video".into(), false)),
                }
                return;
            }
            KeyCode::Char('z') => {
                self.queue.shuffle_rest(&mut self.rng);
                self.status = Some(("shuffled the rest of the queue".into(), false));
                return;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                return;
            }
            KeyCode::Up => {
                return self
                    .audio
                    .shared
                    .set_volume(self.audio.shared.volume() + 0.1);
            }
            KeyCode::Down => {
                return self
                    .audio
                    .shared
                    .set_volume(self.audio.shared.volume() - 0.1);
            }
            KeyCode::Char('s') => self.sync = !self.sync,
            KeyCode::Char('f') => self.fill = !self.fill,
            KeyCode::Char('p') => self.set_preset((self.preset + 1) % PRESETS.len()),
            KeyCode::Char('P') => {
                self.set_preset((self.preset + PRESETS.len() - 1) % PRESETS.len())
            }
            KeyCode::Char('[') => self.effects.intensity = (self.effects.intensity - 0.1).max(0.1),
            KeyCode::Char(']') => self.effects.intensity = (self.effects.intensity + 0.1).min(2.0),
            KeyCode::Char('h') => return self.show_hud = !self.show_hud,
            KeyCode::Char('r') => return self.step_res(true),
            KeyCode::Char('R') => return self.step_res(false),
            KeyCode::Char('g') => return self.toggle_renderer(),
            _ => {}
        }
        if matches!(k.code, KeyCode::Char('s' | 'f' | 'p' | 'P' | '[' | ']')) {
            return self.save_settings();
        }

        let Some(p) = self.player.as_mut() else {
            return;
        };
        let seek = |p: &mut Player, d: f64| p.seek(d).err().map(|e| (format!("{e:#}"), true));
        match k.code {
            KeyCode::Char(' ') => p.toggle_pause(),
            KeyCode::Left => self.status = seek(p, -SEEK_STEP),
            KeyCode::Right => self.status = seek(p, SEEK_STEP),
            _ => {}
        }
    }

    /// Changes when what's on screen moves around: another screen, tab, list, scroll
    /// position, overlay or video. Not on things that tick, like the clock or spinner.
    fn layout_signature(&self) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut h = DefaultHasher::new();
        let view = self.view();
        (
            self.show_video,
            self.tab as u8,
            self.tabs[self.tab as usize].len(),
            view.entries.len(),
            view.state.offset(),
            view.loading,
            self.queue.items.len(),
            self.queue_state.offset(),
        )
            .hash(&mut h);
        // The preview pane's text changes with the selection.
        if self.show_preview && !self.show_video {
            (view.state.selected(), self.queue_state.selected()).hash(&mut h);
        }
        (
            self.busy.is_some(),
            self.show_help,
            self.fx_menu,
            self.account_ui.is_some(),
            &self.status,
            self.player.as_ref().map(|p| &p.title),
            matches!(self.focus, Focus::Search),
        )
            .hash(&mut h);
        h.finish()
    }

    fn set_preset(&mut self, i: usize) {
        self.preset = i;
        self.effects.apply_preset(&PRESETS[i]);
    }
}

/// The video id of a YouTube watch, youtu.be or shorts URL.
fn video_id(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.");
    let id = match host {
        "youtu.be" => path.split(['?', '&', '#', '/']).next(),
        "youtube.com" | "music.youtube.com" => {
            if let Some(q) = path.strip_prefix("watch?") {
                q.split('&').find_map(|kv| kv.strip_prefix("v="))
            } else {
                ["shorts/", "live/", "embed/"]
                    .iter()
                    .find_map(|p| path.strip_prefix(p))
                    .and_then(|p| p.split(['?', '&', '#', '/']).next())
            }
        }
        _ => None,
    }?;
    youtube::is_video_id(id).then(|| id.to_string())
}

fn run(term: &mut Term, app: &mut App) -> Result<()> {
    let mut last = Instant::now();
    let mut last_layout = 0;
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
        app.preview_want = None;
        app.preview_image = None;
        term.draw(|f| ui::draw(f, app))?;
        // ratatui only rewrites cells it thinks changed. Titles with characters whose
        // width the terminal sees differently (ambiguous-width symbols, some emoji) shift
        // the rest of their row, and those cells would never be cleaned up. Whenever the
        // layout changes, repaint everything (inside the synchronized update: no flicker).
        let layout = app.layout_signature();
        if layout != last_layout {
            last_layout = layout;
            term.clear()?;
            if let Some(k) = app.kitty.as_mut() {
                k.forget();
            }
            app.image_rect = None;
            app.preview_want = None;
            app.preview_image = None;
            term.draw(|f| ui::draw(f, app))?;
        }
        draw_image(term, app)?;
        execute!(term.backend_mut(), EndSynchronizedUpdate)?;

        let fps = if app.show_video && app.player.is_some() {
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
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => app.on_key(k),
                Event::Mouse(m) => app.on_mouse(m),
                Event::Paste(text) => app.on_paste(&text),
                _ => {}
            }
            if app.quit {
                break;
            }
        }
    }
    Ok(())
}

/// Graphics renderer: put the processed frame (or the preview thumbnail) where ui::draw
/// left room for it.
fn draw_image(term: &mut Term, app: &mut App) -> Result<()> {
    let Some(kitty) = app.kitty.as_mut() else {
        return Ok(());
    };
    let out = term.backend_mut();
    let (w, h) = app.frame_dims;
    let thumb = app
        .preview_image
        .as_ref()
        .and_then(|(rect, url)| match app.thumbs.get(url) {
            Some(preview::Slot::Ready(t)) => Some((*rect, t)),
            _ => None,
        });
    match (app.image_rect, thumb) {
        (Some(rect), _) if w > 0 && app.frame_out.len() >= w * h * 3 => {
            kitty.draw(out, rect, &app.frame_out, w, h)?
        }
        (None, Some((rect, t))) => kitty.draw_still(out, rect, &t.data, (t.w, t.h), t.id)?,
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
    execute!(
        out,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste,
        cursor::Hide
    )?;
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(
        stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        LeaveAlternateScreen,
        cursor::Show
    );
}

fn main() -> Result<()> {
    let mut cfg = parse_args()?;
    check_tool("yt-dlp", "--version")?;
    check_tool("ffmpeg", "-version")?;

    // Command-line cookies win for this session; otherwise use the saved account.
    youtube::remove_cookie_copies(false);
    let mut startup_error = None;
    let signed_in = if let Some(auth) = cfg.auth.take() {
        youtube::set_auth(Some(auth));
        Some("command-line cookies".to_string())
    } else {
        match account::load().and_then(|a| a.map(|a| Ok((a.auth()?, a))).transpose()) {
            Ok(Some((auth, account))) => {
                youtube::set_auth(Some(auth));
                Some(account.describe())
            }
            Ok(None) => None,
            Err(e) => {
                startup_error = Some((format!("sign-in: {e:#}"), true));
                None
            }
        }
    };

    let audio = AudioOut::open()?;
    audio.shared.set_volume(cfg.volume);

    let mut app = App::new(cfg, audio, signed_in);
    if startup_error.is_some() {
        app.status = startup_error;
    }
    let mut term = setup_terminal()?;
    let result = run(&mut term, &mut app);
    if let Some(k) = app.kitty.as_mut() {
        let _ = k.clear(term.backend_mut());
        let _ = std::io::Write::flush(term.backend_mut());
    }
    drop(app);
    restore_terminal();
    youtube::remove_cookie_copies(true);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids_from_urls() {
        let id = Some("dQw4w9WgXcQ".to_string());
        assert_eq!(video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), id);
        assert_eq!(
            video_id("https://youtube.com/watch?list=RD1&v=dQw4w9WgXcQ&t=3"),
            id
        );
        assert_eq!(video_id("https://youtu.be/dQw4w9WgXcQ?si=x"), id);
        assert_eq!(video_id("https://m.youtube.com/shorts/dQw4w9WgXcQ"), id);
        assert_eq!(video_id("https://vimeo.com/watch?v=dQw4w9WgXcQ"), None);
        assert_eq!(video_id("https://www.youtube.com/watch?v=short"), None);
    }
}
