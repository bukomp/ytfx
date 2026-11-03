//! YouTube access through the `yt-dlp` CLI (no API key needed).

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

/// How yt-dlp authenticates. None when signed out.
#[derive(Clone, Debug)]
pub enum Auth {
    /// A Netscape cookies.txt; each run gets a private copy (see `CookieCopy`).
    CookiesFile(PathBuf),
    /// yt-dlp reads this browser's cookie store.
    Browser(String),
}

static AUTH: Mutex<Option<Auth>> = Mutex::new(None);
static COPIES: AtomicU64 = AtomicU64::new(0);

pub fn set_auth(auth: Option<Auth>) {
    *AUTH.lock().unwrap_or_else(|e| e.into_inner()) = auth;
}

/// yt-dlp saves its cookie jar back to the `--cookies` file when it exits (truncate and
/// rewrite, no locking), so it never gets the saved file: each run reads a private copy
/// that is deleted afterwards. Cookies YouTube refreshed during the run are merged back
/// into the saved account file (see `save_back`), which keeps that session alive.
struct CookieCopy {
    path: PathBuf,
    /// The saved file the copy came from, when refreshed cookies should go back into it.
    source: Option<PathBuf>,
    /// The copy's contents before yt-dlp ran.
    before: String,
}

/// Serializes merges into the saved cookie file between concurrent yt-dlp runs.
static SAVE_BACK: Mutex<()> = Mutex::new(());

impl CookieCopy {
    fn new(src: &Path) -> Result<Self> {
        let data = fs::read(src).with_context(|| format!("can't read {}", src.display()))?;
        let n = COPIES.fetch_add(1, Relaxed);
        let path = std::env::temp_dir().join(format!("ytfx-cookies-{}-{n}.txt", std::process::id()));
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .context("can't create a temporary cookie file")?;
        // Only the account file ytfx manages gets updates; a --cookies file stays untouched.
        let ours = crate::account::cookies_path().is_ok_and(|p| p == src);
        let copy = CookieCopy {
            path,
            source: ours.then(|| src.to_path_buf()),
            before: String::from_utf8_lossy(&data).into_owned(),
        };
        f.write_all(&data)?;
        Ok(copy)
    }

    /// Merge cookies that changed during the run into the saved file.
    fn save_back(&self) -> Result<()> {
        let Some(source) = &self.source else {
            return Ok(());
        };
        let after = String::from_utf8_lossy(&fs::read(&self.path)?).into_owned();
        let _lock = SAVE_BACK.lock().unwrap_or_else(|e| e.into_inner());
        let current = fs::read_to_string(source)?;
        if let Some(merged) = merge_cookies(&current, &self.before, &after) {
            crate::account::write_private(source, &merged)?;
        }
        Ok(())
    }
}

impl Drop for CookieCopy {
    fn drop(&mut self) {
        let _ = self.save_back();
        let _ = fs::remove_file(&self.path);
    }
}

/// Delete temporary cookie copies: this process's own (call on exit, when background runs
/// may still hold some), and those of earlier ytfx processes that are gone.
pub fn remove_cookie_copies(own: bool) {
    let me = std::process::id();
    let Ok(dir) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for entry in dir.flatten() {
        let name = entry.file_name();
        let Some(pid) = name
            .to_str()
            .and_then(|n| n.strip_prefix("ytfx-cookies-"))
            .and_then(|rest| rest.split('-').next())
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        // SAFETY: kill with signal 0 only checks whether the process exists.
        let alive = pid != me && unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;
        if (pid == me && own) || (pid != me && !alive) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// (domain, path, name) of a Netscape cookie line, or None for comments and junk.
fn cookie_key(line: &str) -> Option<(&str, &str, &str)> {
    let body = line.strip_prefix("#HttpOnly_").unwrap_or(line);
    if body.starts_with('#') || line.chars().any(|c| c.is_control() && c != '\t') {
        return None;
    }
    let f: Vec<&str> = body.split('\t').collect();
    (f.len() == 7).then(|| (f[0], f[2], f[5]))
}

fn is_youtube_domain(domain: &str) -> bool {
    let d = domain.trim_start_matches('.');
    d == "youtube.com" || d.ends_with(".youtube.com")
}

/// Apply what a yt-dlp run changed (`before` -> `after`) to `current`: youtube.com
/// cookies that are new or have a new value. Removals are ignored. None if nothing changed.
fn merge_cookies(current: &str, before: &str, after: &str) -> Option<String> {
    let old: std::collections::HashSet<&str> = before.lines().collect();
    let changed: Vec<(_, &str)> = after
        .lines()
        .filter(|l| !old.contains(l))
        .filter_map(|l| cookie_key(l).map(|k| (k, l)))
        .filter(|(k, _)| is_youtube_domain(k.0))
        .collect();
    if changed.is_empty() {
        return None;
    }
    let mut lines: Vec<&str> = current.lines().collect();
    let mut out_changed = false;
    for (key, line) in changed {
        match lines.iter().position(|l| cookie_key(l) == Some(key)) {
            Some(i) if lines[i] != line => {
                lines[i] = line;
                out_changed = true;
            }
            Some(_) => {}
            None => {
                lines.push(line);
                out_changed = true;
            }
        }
    }
    out_changed.then(|| lines.join("\n") + "\n")
}

/// A yt-dlp command for `target`, signed in only when the target is YouTube itself, so
/// account cookies never go to other sites. Keep the CookieCopy alive while it runs.
fn yt_dlp(target: &str) -> Result<(Command, Option<CookieCopy>)> {
    let mut cmd = Command::new("yt-dlp");
    cmd.stdin(Stdio::null());
    let auth = AUTH.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let mut copy = None;
    if is_youtube_target(target) {
        match auth {
            Some(Auth::Browser(b)) => {
                cmd.arg("--cookies-from-browser").arg(b);
            }
            Some(Auth::CookiesFile(src)) => {
                let c = CookieCopy::new(&src)?;
                cmd.arg("--cookies").arg(&c.path);
                copy = Some(c);
            }
            None => {}
        }
    }
    Ok((cmd, copy))
}

/// A yt-dlp search or feed keyword, or an http(s) URL on a YouTube host.
pub fn is_youtube_target(target: &str) -> bool {
    if target.starts_with(":yt") || target.starts_with("ytsearch") {
        return true;
    }
    let Some(rest) = target
        .strip_prefix("https://")
        .or_else(|| target.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    matches!(
        host,
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be"
    )
}

/// Account feeds (need sign-in).
pub const HOME: &str = ":ytrec";
pub const SUBSCRIPTIONS: &str = ":ytsubs";

const SITE: &str = "https://www.youtube.com/";

/// Feeds and library pages that only exist for a signed-in account.
pub fn needs_account(target: &str) -> bool {
    target.starts_with(':') || target.starts_with("https://www.youtube.com/feed/")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Video,
    Playlist,
    Channel,
}

/// One row of a list: a video, or a playlist / channel that opens into another list.
#[derive(Debug, Clone)]
pub struct Entry {
    pub kind: Kind,
    /// Video id for videos; for lists a youtube.com URL or a yt-dlp feed keyword.
    pub target: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<f64>,
    pub views: Option<u64>,
}

impl Entry {
    pub fn url(&self) -> String {
        match self.kind {
            Kind::Video => format!("{SITE}watch?v={}", self.target),
            _ => self.target.clone(),
        }
    }

    /// What to list when this entry is opened (channels open on their uploads).
    pub fn list_target(&self) -> String {
        let t = &self.target;
        let tabbed = ["/videos", "/streams", "/shorts", "/playlists"]
            .iter()
            .any(|tab| t.ends_with(tab));
        if self.kind == Kind::Channel && !tabbed {
            format!("{}/videos", t.trim_end_matches('/'))
        } else {
            list_url(t)
        }
    }

    fn folder(title: &str, target: &str) -> Entry {
        Entry {
            kind: Kind::Playlist,
            target: target.into(),
            title: title.into(),
            channel: String::new(),
            duration: None,
            views: None,
        }
    }
}

/// YouTube won't show Mixes (list ids starting with RD, e.g. "My Mix" RDMM<video id>) as
/// playlist pages ("This playlist type is unviewable"); they open from a watch page of
/// their seed video, whose id ends the list id.
pub fn list_url(url: &str) -> String {
    let list = url
        .split(['?', '&'])
        .find_map(|kv| kv.strip_prefix("list="))
        .unwrap_or_default();
    let seed = list.get(list.len().saturating_sub(11)..).unwrap_or_default();
    if url.starts_with(SITE) && list.starts_with("RD") && list.len() > 11 && is_video_id(seed) {
        format!("{SITE}watch?v={seed}&list={list}")
    } else {
        url.to_string()
    }
}

/// The fixed entries of the Library tab.
pub fn library() -> Vec<Entry> {
    vec![
        Entry::folder("Watch later", ":ytwatchlater"),
        Entry::folder("Liked videos", ":ytfav"),
        Entry::folder("History", ":ythistory"),
        Entry::folder("Your playlists", "https://www.youtube.com/feed/playlists"),
        Entry::folder(
            "Subscribed channels",
            "https://www.youtube.com/feed/channels",
        ),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    Videos,
    Playlists,
    Channels,
}

impl SearchKind {
    pub fn label(self) -> &'static str {
        match self {
            SearchKind::Videos => "videos",
            SearchKind::Playlists => "playlists",
            SearchKind::Channels => "channels",
        }
    }

    pub fn next(self) -> Self {
        match self {
            SearchKind::Videos => SearchKind::Playlists,
            SearchKind::Playlists => SearchKind::Channels,
            SearchKind::Channels => SearchKind::Videos,
        }
    }

    /// The yt-dlp input that runs this search.
    pub fn target(self, query: &str, count: usize) -> String {
        // `sp` is YouTube's result-type filter.
        let filter = match self {
            SearchKind::Videos => return format!("ytsearch{count}:{query}"),
            SearchKind::Playlists => "EgIQAw%3D%3D",
            SearchKind::Channels => "EgIQAg%3D%3D",
        };
        format!("{SITE}results?search_query={}&sp={filter}", encode(query))
    }
}

/// Percent-encode a query string value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Stream {
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct StreamInfo {
    pub title: String,
    pub duration: Option<f64>,
    /// width / height of the video
    pub aspect: f64,
    pub video: Stream,
    pub audio: Stream,
}

#[derive(Deserialize)]
struct FlatEntry {
    id: Option<String>,
    url: Option<String>,
    ie_key: Option<String>,
    title: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    duration: Option<f64>,
    view_count: Option<f64>,
}

impl FlatEntry {
    fn into_entry(self) -> Option<Entry> {
        let (kind, target) = if self.ie_key.as_deref() == Some("Youtube") {
            (Kind::Video, self.id.filter(|id| is_video_id(id))?)
        } else {
            // Only follow plain youtube.com pages; the URL goes back into yt-dlp.
            let url = self.url?;
            let path = url.strip_prefix(SITE)?;
            if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return None;
            }
            let kind = if path.starts_with("playlist?") {
                Kind::Playlist
            } else if ["channel/", "@", "c/", "user/"]
                .iter()
                .any(|p| path.starts_with(p))
            {
                Kind::Channel
            } else {
                return None;
            };
            (kind, url)
        };
        Some(Entry {
            kind,
            target,
            title: self
                .title
                .map_or_else(|| "(untitled)".into(), |t| clean(&t)),
            channel: clean(&self.channel.or(self.uploader).unwrap_or_default()),
            duration: self.duration,
            views: self.view_count.map(|v| v as u64),
        })
    }
}

#[derive(Deserialize)]
struct Info {
    title: Option<String>,
    duration: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
    url: Option<String>,
    http_headers: Option<HashMap<String, String>>,
    requested_formats: Option<Vec<Format>>,
}

#[derive(Deserialize)]
struct Format {
    url: String,
    http_headers: Option<HashMap<String, String>>,
    vcodec: Option<String>,
}

pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Entries of a search, feed, playlist or channel, without resolving each video.
pub fn list(target: &str, limit: usize) -> Result<Vec<Entry>> {
    match list_once(target, limit) {
        // Auto-generated "Artist - Topic" channels have no Videos tab; their bare channel
        // page lists the uploads instead.
        Err(e) if e.to_string().contains("does not have a videos tab") => {
            match target.strip_suffix("/videos") {
                Some(bare) => list_once(bare, limit),
                None => Err(e),
            }
        }
        other => other,
    }
}

fn list_once(target: &str, limit: usize) -> Result<Vec<Entry>> {
    let (mut cmd, _cookies) = yt_dlp(target)?;
    let out = cmd
        .args([
            "--flat-playlist",
            "--dump-json",
            "--ignore-errors",
            "--playlist-end",
            &limit.to_string(),
            "--",
            target,
        ])
        .output()
        .context("could not run yt-dlp (is it installed and on PATH?)")?;

    let entries: Vec<Entry> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<FlatEntry>(line).ok())
        .filter_map(FlatEntry::into_entry)
        .collect();

    if entries.is_empty() {
        // Warnings are kept on for this: an empty feed is otherwise all yt-dlp says.
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("cookies are no longer valid") {
            bail!("{COOKIES_ROTATED}");
        }
        if !out.status.success() {
            let err = last_line(&out.stderr);
            if err.contains("Login details are needed") || err.contains("Sign in to") {
                bail!("this needs a YouTube account: press L to sign in (or the sign-in expired)");
            }
            bail!("yt-dlp: {err}");
        }
    }
    Ok(entries)
}

const COOKIES_ROTATED: &str = "YouTube says this sign-in is no longer valid. A browser that is \
     signed in to Google refreshes the session in the background, even with no YouTube tab \
     open, and that kills exported copies. Export from a private window instead (sign in, \
     open youtube.com/robots.txt, export, close the window), then import with L. ytfx keeps \
     that session fresh by itself.";

/// Videos for radio: YouTube's Mix for `seed` (related videos, starting with it). Some
/// videos have no Mix; then a search for the seed's channel and title stands in.
pub fn radio(seed: &Entry, limit: usize) -> Result<Vec<Entry>> {
    if seed.kind != Kind::Video || !is_video_id(&seed.target) {
        bail!("radio needs a YouTube video");
    }
    let id = &seed.target;
    let videos = |entries: Vec<Entry>| -> Vec<Entry> {
        entries
            .into_iter()
            .filter(|e| e.kind == Kind::Video)
            .collect()
    };
    let mix = list(&format!("{SITE}watch?v={id}&list=RD{id}"), limit).map(videos);
    if mix
        .as_ref()
        .is_ok_and(|m| m.iter().filter(|e| &e.target != id).count() >= 3)
    {
        return mix;
    }
    let query = format!("{} {}", seed.channel, seed.title);
    let found = list(&SearchKind::Videos.target(query.trim(), limit), limit).map(videos);
    match (mix, found) {
        (_, Ok(found)) if !found.is_empty() => Ok(found),
        (Ok(mix), _) => Ok(mix),
        (Err(e), _) => Err(e),
    }
}

/// Resolve a video page URL into direct media stream URLs that ffmpeg can read.
pub fn resolve(url: &str, max_height: usize) -> Result<StreamInfo> {
    // Best video up to `max_height` (skipping AV1, which is slow to decode) plus best audio.
    let h = max_height;
    let format = format!("bv*[height<={h}][vcodec!^=av01]+ba/bv*[height<={h}]+ba/b[height<={h}]/b");
    let (mut cmd, _cookies) = yt_dlp(url)?;
    let out = cmd
        .args(["-J", "--no-playlist", "--no-warnings", "-f", &format, "--"])
        .arg(url)
        .output()
        .context("could not run yt-dlp (is it installed and on PATH?)")?;
    if !out.status.success() {
        let err = last_line(&out.stderr);
        if err.contains("not a bot") {
            bail!(
                "YouTube bot check for this video: try `brew upgrade yt-dlp`, a different network/VPN off, or signing in (L)"
            );
        }
        bail!("yt-dlp: {err}");
    }
    let info: Info = serde_json::from_slice(&out.stdout).context("unexpected yt-dlp output")?;

    let (video, audio) = match info.requested_formats {
        Some(formats) => {
            let (mut video, mut audio) = (None, None);
            for f in formats {
                let has_video = f.vcodec.as_deref().is_some_and(|c| c != "none");
                let stream = Stream::new(f.url, f.http_headers)?;
                if has_video && video.is_none() {
                    video = Some(stream);
                } else if audio.is_none() {
                    audio = Some(stream);
                }
            }
            (video, audio)
        }
        None => {
            let url = info
                .url
                .ok_or_else(|| anyhow!("no playable stream found"))?;
            let stream = Stream::new(url, info.http_headers)?;
            (Some(stream.clone()), Some(stream))
        }
    };

    let aspect = match (info.width, info.height) {
        (Some(w), Some(h)) if w > 0.0 && h > 0.0 => w / h,
        _ => 16.0 / 9.0,
    };
    Ok(StreamInfo {
        title: info
            .title
            .map_or_else(|| "(untitled)".into(), |t| clean(&t)),
        duration: info.duration,
        aspect,
        video: video.ok_or_else(|| anyhow!("no video stream found"))?,
        audio: audio.ok_or_else(|| anyhow!("no audio stream found"))?,
    })
}

impl Stream {
    fn new(url: String, headers: Option<HashMap<String, String>>) -> Result<Self> {
        // ffmpeg understands many protocols (file:, pipe:, ...); only accept remote media.
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            bail!("refusing non-http stream url");
        }
        let headers = headers.unwrap_or_default();
        // Header lines are joined with CRLF for ffmpeg, so a CR/LF inside one would inject more.
        let bad = |s: &String| s.contains(['\r', '\n', '\0']);
        if headers.iter().any(|(k, v)| bad(k) || bad(v)) {
            bail!("refusing malformed http header from yt-dlp");
        }
        Ok(Stream {
            url,
            headers: headers.into_iter().collect(),
        })
    }
}

fn last_line(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let mut lines = text.lines().rev().filter(|l| !l.trim().is_empty());
    let line = lines
        .clone()
        .find(|l| l.starts_with("ERROR"))
        .or_else(|| lines.next())
        .unwrap_or("unknown error");
    clean(line.trim())
}

/// Strip control and bidi/zero-width characters from remote text before it reaches the
/// terminal; ratatui passes ESC through as-is, so a title could otherwise inject escape sequences.
fn clean(s: &str) -> String {
    s.chars().filter(|&c| is_safe_char(c)).collect()
}

/// Not a control (C0/C1) or bidi/zero-width character.
pub fn is_safe_char(c: char) -> bool {
    !c.is_control()
        && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_escape_sequences() {
        assert_eq!(clean("a\x1b]52;c;aGk=\x07b\u{202e}c\td"), "a]52;c;aGk=bcd");
        assert_eq!(clean("Phonk ☠ mix 日本"), "Phonk ☠ mix 日本");
    }

    #[test]
    fn stream_rejects_bad_urls_and_headers() {
        assert!(Stream::new("file:///etc/passwd".into(), None).is_err());
        let crlf = HashMap::from([("User-Agent".to_string(), "x\r\nHost: evil".to_string())]);
        assert!(Stream::new("https://a.b/v".into(), Some(crlf)).is_err());
        assert!(Stream::new("https://a.b/v".into(), None).is_ok());
    }

    fn flat(json: &str) -> Option<Entry> {
        serde_json::from_str::<FlatEntry>(json).unwrap().into_entry()
    }

    #[test]
    fn flat_entries_are_classified_and_validated() {
        let v = flat(r#"{"ie_key":"Youtube","id":"dQw4w9WgXcQ","title":"t"}"#).unwrap();
        assert_eq!((v.kind, v.target.as_str()), (Kind::Video, "dQw4w9WgXcQ"));
        assert!(flat(r#"{"ie_key":"Youtube","id":"--exec=x"}"#).is_none());

        let p = flat(r#"{"ie_key":"YoutubeTab","url":"https://www.youtube.com/playlist?list=PL1"}"#);
        assert_eq!(p.unwrap().kind, Kind::Playlist);
        let c = flat(r#"{"ie_key":"YoutubeTab","url":"https://www.youtube.com/channel/UC1"}"#)
            .unwrap();
        assert_eq!(c.kind, Kind::Channel);
        assert_eq!(c.list_target(), "https://www.youtube.com/channel/UC1/videos");

        for bad in [
            "https://evil.example/playlist?list=PL1",
            "file:///etc/passwd",
            "https://www.youtube.com/playlist?list=PL1 --exec x",
        ] {
            let json = format!(r#"{{"ie_key":"YoutubeTab","url":"{bad}"}}"#);
            assert!(flat(&json).is_none(), "{bad}");
        }
    }

    #[test]
    fn auth_only_goes_to_youtube() {
        for t in [
            ":ytrec",
            "ytsearch5:x",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://music.youtube.com/playlist?list=PL1",
        ] {
            assert!(is_youtube_target(t), "{t}");
        }
        for t in [
            "https://evil.example/youtube.com/@x",
            "https://www.youtube.com.evil.example/",
            "https://user@www.youtube.com.evil.example/",
            "https://www.youtube.com@evil.example/",
            "file:///etc/passwd",
        ] {
            assert!(!is_youtube_target(t), "{t}");
        }
    }

    #[test]
    fn cookie_copies_are_private_and_removed() {
        let src = std::env::temp_dir().join(format!("ytfx-test-src-{}.txt", std::process::id()));
        fs::write(&src, "x").unwrap();
        let copy = CookieCopy::new(&src).unwrap();
        let path = copy.path.clone();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        drop(copy);
        assert!(!path.exists());
        fs::remove_file(src).unwrap();
    }

    #[test]
    fn mixes_open_from_their_seed_video() {
        assert_eq!(
            list_url("https://www.youtube.com/playlist?list=RDMMaObqW1dRVm8"),
            "https://www.youtube.com/watch?v=aObqW1dRVm8&list=RDMMaObqW1dRVm8"
        );
        let normal = "https://www.youtube.com/playlist?list=PLGBKsNyGY-afUo7nfXwctjuJgCtqrYI6Q";
        assert_eq!(list_url(normal), normal);
    }

    #[test]
    fn refreshed_youtube_cookies_are_merged_back() {
        let line = |d: &str, n: &str, v: &str| format!("{d}\tTRUE\t/\tTRUE\t1900000000\t{n}\t{v}");
        let header = "# Netscape HTTP Cookie File\n";
        let current = format!("{header}{}\n{}\n", line(".youtube.com", "SID", "a"), line(".youtube.com", "SIDCC", "old"));
        let before = current.clone();
        let after = format!(
            "{header}{}\n{}\n{}\n{}\n",
            line(".youtube.com", "SID", "a"),
            line(".youtube.com", "SIDCC", "new"),
            line(".youtube.com", "YSC", "x"),
            line(".evil.example", "track", "y"),
        );
        let merged = merge_cookies(&current, &before, &after).unwrap();
        assert!(merged.contains("SIDCC\tnew") && !merged.contains("SIDCC\told"));
        assert!(merged.contains("YSC\tx") && !merged.contains("evil"));
        assert!(merged.starts_with(header));
        assert_eq!(merge_cookies(&current, &before, &before), None);
    }

    #[test]
    fn search_targets() {
        assert_eq!(SearchKind::Videos.target("a b", 5), "ytsearch5:a b");
        assert_eq!(
            SearchKind::Playlists.target("lo-fi & chill", 5),
            "https://www.youtube.com/results?search_query=lo-fi%20%26%20chill&sp=EgIQAw%3D%3D"
        );
    }

    /// Needs network: `cargo test topic_channel -- --ignored`
    #[test]
    #[ignore]
    fn topic_channel_lists_uploads() {
        let topic = flat(
            r#"{"ie_key":"YoutubeTab","url":"https://www.youtube.com/channel/UCRr1xG_2WIDs18a6cIiCxeA"}"#,
        )
        .unwrap();
        let entries = list(&topic.list_target(), 5).unwrap();
        assert!(!entries.is_empty() && entries.iter().all(|e| e.kind == Kind::Video));
    }
}
