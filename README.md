# ytfx

A terminal YouTube browser. It searches with `yt-dlp` (no API key), plays the video in the terminal with audio (as real pixels in kitty, Ghostty and WezTerm, or as truecolor half-block characters elsewhere), and layers "edit"-style effects on top: zoom punch, shake, RGB split, blur, red grade, glitch, trails, invert flash, VHS, pixelate, mirror and strobe. With SYNC on, the effects are driven by the bass of the music. Press `s` to turn SYNC off; the effects then stay at a steady level with random hits.

## Requirements

- Rust toolchain
- `yt-dlp` and `ffmpeg` on `PATH` (`brew install yt-dlp ffmpeg`)
- A truecolor terminal (kitty, WezTerm, iTerm2, Ghostty, ...)

## Run

```
cargo run --release
cargo run --release -- "search terms"
cargo run --release -- https://www.youtube.com/watch?v=...
```

Options (`ytfx [options] [search terms | youtube url]`):

```
--fps <n>          video/render frame rate (default 30, clamped to 5-60)
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
```

The two cookie options are not saved. To stay signed in, press `L` in the app (see Sign-in).

## Keys

Press `?` for an in-app key list.

Browser:

| Key | Action |
| --- | --- |
| `/` or `i` | Focus the search box |
| `enter` | Search (in the box) / play video / open playlist or channel |
| `ctrl-t` | In the search box: cycle videos / playlists / channels |
| `t` | In results on the search tab: cycle videos / playlists / channels |
| `1` `2` `3` `4` `5` | Search, home, subscriptions, library, queue tab |
| `tab` / `shift-tab` | Next / previous tab (in the search box, `tab` moves to the results) |
| `j` `k` / `down` `up` | Move selection |
| `PgUp` `PgDn` | Move selection by 10 |
| `g` `G` / `Home` `End` | First / last item |
| `esc` / `backspace` / `h` / `left` | Go back out of a playlist or channel |
| `r` | Start radio from the selected video |
| `R` | Start radio from a random video in this list |
| `S` | Shuffle-play this list |
| `a` | Add the selected video to the queue |
| `x` | Remove the selected item (queue tab) |
| `u` | Reload this list |
| `p` | Show or hide the preview pane (thumbnail and details of the selected row; needs a list at least 90 columns wide) |
| `L` | YouTube account (sign in / out) |
| `v` | Back to the playing video |
| `space` | Pause / resume while playing |
| `n` / `N` | Next / previous video while playing |
| `?` | Help |
| `q` / `esc` | Quit |

In the search box, `esc` (or `tab` / `down`) moves to the results. Pasting a URL that starts with `http://` or `https://` plays it directly. Pasting a playlist or channel URL opens it as a list.

Player:

| Key | Action |
| --- | --- |
| `space` | Pause / resume |
| `n` / `N` (or `>` / `<`) | Next / previous video in the queue |
| `m` | Start radio from the playing video |
| `z` | Shuffle the rest of the queue |
| `b` / `tab` | Browse while the video keeps playing |
| `left` `right` | Seek -10 s / +10 s |
| `up` `down` | Volume up / down |
| `s` | Toggle bass sync |
| `p` / `P` | Next / previous preset |
| `[` `]` | Effect intensity down / up (0.1 to 2.0) |
| `1-9` `0` `-` `=` | Toggle individual effects (map below) |
| `r` / `R` | Resolution up / down (graphics renderer) |
| `g` | Switch between pixel graphics and text blocks |
| `f` | Blurred background around narrow videos on / off |
| `h` | Hide / show HUD |
| `?` | Help |
| `q` / `esc` / `backspace` | Stop playback and go back |
| `ctrl-c` | Quit |

Effect keys:

| Key | Effect | Key | Effect | Key | Effect |
| --- | --- | --- | --- | --- | --- |
| `1` | zoom | `5` | red | `9` | vhs |
| `2` | shake | `6` | glitch | `0` | pixel |
| `3` | rgb | `7` | trails | `-` | mirror |
| `4` | blur | `8` | invert | `=` | strobe |

ytfx remembers the enabled effects, preset, intensity, bass sync and background fill across runs, in `~/.config/ytfx/settings.json`. `--no-sync` still turns sync off for one run.

Presets (`p` / `P`; applying one replaces the current effect set):

| Preset | Effects |
| --- | --- |
| edit (default) | zoom, shake, rgb, glitch, vhs |
| phonk red | zoom, shake, red, invert, strobe, blur |
| glitchcore | rgb, glitch, pixel, trails, vhs, shake |
| dream | blur, trails, zoom, mirror |
| clean | none |

## Tabs

The browser has five tabs. Press `1` to `5`, or `tab` / `shift-tab`, to switch.

1. Search: search results.
2. Home: your recommendations.
3. Subscriptions: the latest videos from channels you subscribe to.
4. Library: Watch later, Liked videos, History, Your playlists and Subscribed channels.
5. Queue: the play queue.

Home, subscriptions and library need sign-in.

## Sign-in

Press `L` to open the account dialog. Sign-in persists across runs.

A `cookies.txt` in `~/.config/ytfx/` (or `$XDG_CONFIG_HOME/ytfx/`) is used automatically at startup, also when you copy it there by hand. ytfx removes the lines for sites other than youtube.com and sets the mode to 0600. If the file has no youtube.com sign-in, ytfx starts signed out and shows the reason. A browser chosen with `b` takes priority over the file.

- `f`: import a `cookies.txt` file. Export it from a youtube.com tab with a browser extension (for example "Get cookies.txt LOCALLY"). ytfx keeps only the youtube.com cookies, in `~/.config/ytfx/cookies.txt` (or `$XDG_CONFIG_HOME/ytfx`) with mode 0600. You can delete the exported file afterwards.
- `b`: let yt-dlp read a browser's cookie store (firefox, chrome, chromium, edge, brave, safari, vivaldi, opera). On macOS, Chromium browsers ask for Keychain access. Safari needs Full Disk Access for the terminal. Firefox needs neither.
- `o`: sign out and delete the imported cookies.

ytfx never reads browser data unless you pick `b`.

YouTube can rate-limit or flag accounts that tools like yt-dlp use. A secondary account is safer.

`--cookies <file>` and `--cookies-from-browser <name>` apply to one session only. They are not saved.

## Search kinds, queue and radio

Search finds videos, playlists or channels. Cycle the kind with `ctrl-t` in the search box, or `t` in the results. Enter on a playlist or channel opens it as a list. `esc`, `backspace`, `h` or `left` goes back.

Enter on a video in a feed, playlist or channel plays through that list. A video from plain search results plays alone. `a` adds the selected video to the queue, `S` shuffle-plays the current list, and `x` removes an item on the queue tab.

When a video ends, the next one starts. ytfx resolves the next video in the background a few seconds into the current one, so it starts without waiting. If 3 videos in a row fail, the queue stops.

Radio: `r` in the browser starts radio from the selected video. `R` starts it from a random video in the current list. `m` in the player starts it from the playing video. Radio uses YouTube's Mix for that video (related videos), shuffled, and never repeats a video. When fewer than 3 videos are left, it fetches a Mix seeded by a random recently played video. If a video has no Mix, radio falls back to a search for its channel and title.

## Browse while playing

`b` or `tab` in the player goes to the browser while playback continues. A now-playing line shows at the bottom. In the browser, `v` returns to the video, `space` pauses, and `n` / `N` skip to the next / previous video. In the player, `q`, `esc` or `backspace` stops playback and goes back. The HUD shows the queue position (for example `[3/25]`) and a RADIO badge.

## Resolution

There are two renderers:

- **Graphics (kitty protocol):** used automatically in kitty, Ghostty and WezTerm. The video is sent as a real image at the resolution you choose (`--res`, or `r` / `R` while playing), and the terminal scales it to fit the window. kitty receives frames through shared memory, so even 720p costs almost nothing to send. Other terminals receive them inline as base64.
- **Blocks:** used everywhere else, and inside tmux. Each cell shows two pixels with the `▀` character, so the resolution is the terminal grid. For example, 200 columns gives a 200x112 picture. To get more detail, make the font smaller.

The source stream is fetched at 720p or at the chosen resolution, whichever is higher. With every effect on, processing takes about 7.5 ms per frame at 720p and about 17 ms at 1080p. With no effects on, it costs almost nothing.

When the video doesn't fill its area (another aspect ratio, or smaller than the window), ytfx fills the empty space with a zoomed, blurred and darkened copy of the video. `f` turns this off.

## How the bass sync works

ytfx decodes the audio itself with ffmpeg and plays it through cpal. That means it can tap the exact samples it sends to the speakers. No system audio capture, loopback device or OS permission is needed.

Each frame the analyzer:

1. Runs a 2048-point FFT on the latest played audio.
2. Sums the 30-150 Hz band energy, averaged over two frames, to get the bass energy.
3. Scales it with an auto-gain (a slow peak follower with a floor) to get a "level".
4. Fires a beat only on a spike: the level must come near the recent peak, the energy must have at least doubled from its lowest point in the last 0.3 s, and after a beat the level must drop back before the next one can fire. Held or swelling bass lines, such as orchestral low strings or long 808s, wobble but don't do all three, so they don't trigger effects.
5. Sets a beat pulse that jumps on a beat and decays exponentially. The pulse scales every effect.

Video frames are scheduled against the audio clock (samples actually played), so audio and video stay in sync.

## Architecture

- `src/main.rs`: argument parsing, app state, main loop, key handling
- `src/youtube.rs`: `yt-dlp` search and stream URL resolution
- `src/account.rs`: sign-in (cookie import, saved account)
- `src/queue.rs`: play queue and radio
- `src/player.rs`: ffmpeg video and audio decoders, A/V sync, seek, resize
- `src/audio.rs`: cpal output, volume, and the tap of played samples
- `src/analysis.rs`: FFT, bass level, beat detection
- `src/effects.rs`: effect pipeline, presets, effect key map
- `src/kitty.rs`: kitty graphics protocol output (shared memory or inline base64)
- `src/ui.rs`: ratatui rendering (browser, HUD, help, account dialog) and the half-block video widget

## Troubleshooting

- "Sign in to confirm you're not a bot": YouTube flags some IP addresses (VPNs, datacenters, shared networks). Try these in order:
  1. Update yt-dlp (`brew upgrade yt-dlp`). YouTube changes often and yt-dlp ships fixes often.
  2. Turn off your VPN or try another network.
  3. Press `L` and choose `f` to import a `cookies.txt` you exported yourself (ideally from a secondary Google account). Only that file is shared, and no browser or Keychain access is needed.
  4. Press `L` and choose `b` to let yt-dlp read your browser's cookie store. On macOS, Chromium browsers ask for Keychain access to decrypt it. Use this only if you are comfortable with that.
- "YouTube says this sign-in is no longer valid" or empty Home / Subscriptions: a browser that is signed in to Google refreshes the whole Google session in the background, even with no YouTube tab open, and that invalidates exported copies. Export from a private window instead: sign in to YouTube, open `https://www.youtube.com/robots.txt` in the same tab, export the youtube.com cookies, and close the private window right away. Then import the file with `L` → `f`, or copy it to `~/.config/ytfx/cookies.txt`. That session belongs to ytfx alone, and ytfx keeps it fresh by saving the cookies YouTube updates after each request.
- Low resolution: in kitty, Ghostty or WezTerm, press `r`. In other terminals, the blocks renderer is limited by the cell grid (see Resolution).
- Choppy video: lower the resolution (`R`), `--max-width` or `--fps`.
- Wrong or banded colors: the terminal probably lacks truecolor support. Use one that has it.

## Tests

```
cargo test --release
```

Two tests are ignored by default because they need input files:

- `real_audio` (`src/analysis.rs`): runs the analyzer on real audio.
  `YTFX_PCM=file.f32 cargo test real_audio -- --ignored --nocapture`, where the file is 48 kHz stereo f32le (for example from `ffmpeg ... -ac 2 -ar 48000 -f f32le`).
- `preview` (`src/effects.rs`): writes one PPM per preset, mid-beat.
  `YTFX_FRAME=frame.rgb YTFX_OUT=dir cargo test preview -- --ignored`, where `frame.rgb` is a raw 200x112 RGB24 frame and `dir` is the output directory.
