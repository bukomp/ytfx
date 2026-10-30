//! Optional YouTube sign-in. ytfx never reads browser data on its own: the user either
//! imports a cookies.txt they exported, or picks a browser for yt-dlp to read cookies from.
//! The choice is saved in the config directory; imported cookies are trimmed to
//! youtube.com and kept in a file only the user can read.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::youtube::Auth;

/// Browsers yt-dlp can read cookies from.
pub const BROWSERS: [&str; 8] = [
    "firefox", "chrome", "chromium", "edge", "brave", "safari", "vivaldi", "opera",
];
const MAX_COOKIES_FILE: u64 = 16 << 20;
/// Cookies that only exist for a signed-in Google account.
const LOGIN_COOKIES: [&str; 4] = [
    "SAPISID",
    "__Secure-3PAPISID",
    "__Secure-1PAPISID",
    "LOGIN_INFO",
];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Account {
    /// Cookies imported into the config directory.
    CookiesFile,
    /// yt-dlp reads this browser's cookie store each time it runs.
    Browser(String),
}

impl Account {
    pub fn auth(&self) -> Result<Auth> {
        Ok(match self {
            Account::CookiesFile => Auth::CookiesFile(cookies_path()?),
            Account::Browser(b) => Auth::Browser(b.clone()),
        })
    }

    pub fn describe(&self) -> String {
        match self {
            Account::CookiesFile => "imported cookies".into(),
            Account::Browser(b) => format!("{b} cookies"),
        }
    }
}

fn config_dir() -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?).join(".config"),
    };
    Ok(base.join("ytfx"))
}

fn account_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("account.json"))
}

pub fn cookies_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cookies.txt"))
}

/// The account to start with. A browser chosen in the account dialog wins; otherwise
/// `cookies.txt` in the config directory is used whenever it exists, also when it was
/// copied there by hand. It's trimmed to youtube.com and made private on the way.
pub fn load() -> Result<Option<Account>> {
    let saved = fs::read_to_string(account_path()?)
        .ok()
        .and_then(|text| serde_json::from_str::<Account>(&text).ok());
    if let Some(Account::Browser(b)) = &saved
        && BROWSERS.contains(&b.as_str())
    {
        return Ok(saved);
    }
    let path = cookies_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&fs::read(&path)?).into_owned();
    let (kept, count) =
        youtube_cookies(&text).with_context(|| format!("not using {}", path.display()))?;
    if count < cookie_lines(&text) {
        write_private(&path, &kept)?;
    } else if fs::metadata(&path)?.permissions().mode() & 0o077 != 0 {
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(Some(Account::CookiesFile))
}

/// Number of cookie lines in a Netscape cookie file.
fn cookie_lines(text: &str) -> usize {
    text.lines()
        .map(|l| l.strip_prefix("#HttpOnly_").unwrap_or(l))
        .filter(|l| !l.starts_with('#') && l.split('\t').count() == 7)
        .count()
}

pub fn save(account: &Account) -> Result<()> {
    if let Account::Browser(b) = account
        && !BROWSERS.contains(&b.as_str())
    {
        bail!("unsupported browser `{b}`");
    }
    // Switching to a browser drops the imported cookies; they are no longer used.
    if matches!(account, Account::Browser(_)) {
        remove(&cookies_path()?)?;
    }
    write_private(&account_path()?, &serde_json::to_string(account)?)
}

pub fn sign_out() -> Result<()> {
    remove(&account_path()?)?;
    remove(&cookies_path()?)
}

/// Copy the youtube.com cookies of a Netscape cookies.txt into the config directory.
/// Returns how many cookies were kept.
pub fn import_cookies(src: &Path) -> Result<usize> {
    let meta = fs::metadata(src).with_context(|| format!("can't read {}", src.display()))?;
    if !meta.is_file() || meta.len() > MAX_COOKIES_FILE {
        bail!("{} is not a cookies.txt file", src.display());
    }
    let text = String::from_utf8_lossy(&fs::read(src)?).into_owned();
    let (kept, count) = youtube_cookies(&text)?;
    write_private(&cookies_path()?, &kept)?;
    save(&Account::CookiesFile)?;
    Ok(count)
}

/// Expand a leading `~/` in a path typed by the user.
pub fn expand_home(path: &str) -> PathBuf {
    let path = path.trim();
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// Keep only youtube.com lines of a Netscape cookie file, and check there is a sign-in.
fn youtube_cookies(text: &str) -> Result<(String, usize)> {
    let mut out = String::from("# Netscape HTTP Cookie File\n# youtube.com cookies imported by ytfx\n\n");
    let (mut count, mut signed_in) = (0, false);
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        // `#HttpOnly_` marks an HttpOnly cookie; any other `#` line is a comment.
        let body = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        if body.starts_with('#') || line.chars().any(|c| c.is_control() && c != '\t') {
            continue;
        }
        let fields: Vec<&str> = body.split('\t').collect();
        if fields.len() != 7 {
            continue;
        }
        let domain = fields[0].trim_start_matches('.');
        if domain != "youtube.com" && !domain.ends_with(".youtube.com") {
            continue;
        }
        signed_in |= LOGIN_COOKIES.contains(&fields[5]);
        out.push_str(line);
        out.push('\n');
        count += 1;
    }
    if count == 0 {
        bail!("no youtube.com cookies in that file: export it from a youtube.com tab");
    }
    if !signed_in {
        bail!("those youtube.com cookies are not signed in: sign in on youtube.com and export again");
    }
    Ok((out, count))
}

/// Write `data` to `path` with mode 0600, via a fresh temp file so a symlink or an
/// existing file with looser permissions is never written through.
fn write_private(path: &Path, data: &str) -> Result<()> {
    let dir = path.parent().context("bad config path")?;
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;

    let tmp = path.with_extension("tmp");
    remove(&tmp)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("can't write {}", tmp.display()))?;
    f.write_all(data.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    Ok(())
}

fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_signed_in_youtube_cookies() {
        let text = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tabc\n\
            #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1900000000\tLOGIN_INFO\txyz\r\n\
            .google.com\tTRUE\t/\tTRUE\t1900000000\tSID\tsecret\n\
            .bank.example\tTRUE\t/\tTRUE\t1900000000\tsession\tsecret\n\
            .evilyoutube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tx\n\
            music.youtube.com\tFALSE\t/\tTRUE\t1900000000\tPREF\tf=1\n";
        let (out, count) = youtube_cookies(text).unwrap();
        assert_eq!(count, 3);
        assert!(!out.contains("secret") && !out.contains("evil"));
        assert!(out.contains("#HttpOnly_.youtube.com"));
    }

    #[test]
    fn counts_cookie_lines() {
        let text = "# Netscape HTTP Cookie File\n\
            .youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\ta\n\
            #HttpOnly_.google.com\tTRUE\t/\tTRUE\t0\tSID\tb\n\n";
        assert_eq!(cookie_lines(text), 2);
        assert_eq!(youtube_cookies(text).unwrap().1, 1);
    }

    #[test]
    fn rejects_files_without_a_youtube_sign_in() {
        assert!(youtube_cookies("not a cookie file").is_err());
        let anon = ".youtube.com\tTRUE\t/\tTRUE\t1900000000\tPREF\tf=1\n";
        assert!(youtube_cookies(anon).is_err());
    }
}
