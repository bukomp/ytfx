//! YouTube access through the `yt-dlp` CLI (no API key needed).

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

/// Extra yt-dlp arguments for authentication (cookies), set once from the command line.
static AUTH_ARGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn add_auth_arg(flag: &str, value: &str) {
    let mut args = AUTH_ARGS.lock().unwrap_or_else(|e| e.into_inner());
    args.push(flag.to_string());
    args.push(value.to_string());
}

fn yt_dlp() -> Command {
    let mut cmd = Command::new("yt-dlp");
    cmd.args(AUTH_ARGS.lock().unwrap_or_else(|e| e.into_inner()).iter());
    cmd.stdin(Stdio::null());
    cmd
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<f64>,
    pub views: Option<u64>,
}

impl SearchResult {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }
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
    title: Option<String>,
    channel: Option<String>,
    uploader: Option<String>,
    duration: Option<f64>,
    view_count: Option<f64>,
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

pub fn search(query: &str, count: usize) -> Result<Vec<SearchResult>> {
    let out = yt_dlp()
        .args([
            "--flat-playlist",
            "--dump-json",
            "--no-warnings",
            "--ignore-errors",
        ])
        .arg(format!("ytsearch{count}:{query}"))
        .output()
        .context("could not run yt-dlp (is it installed and on PATH?)")?;

    let results: Vec<SearchResult> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<FlatEntry>(line).ok())
        .filter_map(|e| {
            Some(SearchResult {
                id: e.id?,
                title: e.title.map_or_else(|| "(untitled)".into(), |t| clean(&t)),
                channel: clean(&e.channel.or(e.uploader).unwrap_or_default()),
                duration: e.duration,
                views: e.view_count.map(|v| v as u64),
            })
        })
        .collect();

    if results.is_empty() && !out.status.success() {
        bail!("search failed: {}", last_line(&out.stderr));
    }
    Ok(results)
}

/// Resolve a video page URL into direct media stream URLs that ffmpeg can read.
pub fn resolve(url: &str, max_height: usize) -> Result<StreamInfo> {
    // Best video up to `max_height` (skipping AV1, which is slow to decode) plus best audio.
    let h = max_height;
    let format = format!("bv*[height<={h}][vcodec!^=av01]+ba/bv*[height<={h}]+ba/b[height<={h}]/b");
    let out = yt_dlp()
        .args(["-J", "--no-playlist", "--no-warnings", "-f", &format, "--"])
        .arg(url)
        .output()
        .context("could not run yt-dlp (is it installed and on PATH?)")?;
    if !out.status.success() {
        let err = last_line(&out.stderr);
        if err.contains("not a bot") {
            bail!(
                "YouTube bot check for this video: try `brew upgrade yt-dlp`, a different network/VPN off, or --cookies <file>"
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
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("unknown error");
    clean(line.trim())
}

/// Strip control and bidi/zero-width characters from remote text before it reaches the
/// terminal; ratatui passes ESC through as-is, so a title could otherwise inject escape sequences.
fn clean(s: &str) -> String {
    s.chars()
        .filter(|&c| !c.is_control() && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .collect()
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
}
