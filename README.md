# ytfx

A terminal YouTube browser. It searches with `yt-dlp` (no API key), plays the video in the terminal with audio (as real pixels in kitty, Ghostty and WezTerm, or as truecolor half-block characters elsewhere), and layers 37 "edit"-style effects on top (zoom punches, shake, glitches, colour grades, VHS, CRT, film and more), picked from a side menu. With SYNC on, the effects are driven by the bass of the music. Press `s` to turn SYNC off; the effects then stay at a steady level with random hits.

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
| `1` `2` `3` `4` `5` `6` | Search, home, subscriptions, library, queue, history tab |
| `tab` / `shift-tab` | Next / previous tab (in the search box, `tab` moves to the results) |
| `j` `k` / `down` `up` | Move selection |
| `PgUp` `PgDn` | Move selection by 10 |
| `g` `G` / `Home` `End` | First / last item |
| `esc` / `backspace` / `h` / `left` | Go back out of a playlist or channel |
| `r` | Start radio from the selected video |
| `R` | Start radio from a random video in this list |
| `S` | Shuffle-play this list |
| `a` | Add the selected video to the queue |
| `x` | Remove the selected item (queue and history tabs) |
| `u` | Reload this list |
| `p` | Show or hide the preview pane (thumbnail and details of the selected row; needs a list at least 90 columns wide) |
| `L` | YouTube account (sign in / out) |
| `v` | Back to the playing video |
| `space` | Pause / resume while playing |
| `n` / `N` | Next / previous video while playing |
| `?` | Help |
| `q` / `esc` | Quit |

In the search box, `esc` (or `tab` / `down`) moves to the results. Pasting a URL that starts with `http://` or `https://` plays it directly. Pasting a playlist or channel URL opens it as a list.

The search box (and the cookies.txt path field) edits like a normal text field:

| Key | Action |
| --- | --- |
| `left` / `right` (or `ctrl-b` / `ctrl-f`) | Move one character |
| `option`/`alt`/`ctrl` + `left` / `right` (or `alt-b` / `alt-f`) | Move one word |
| `home` / `end`, `ctrl-a` / `ctrl-e`, `cmd-left` / `cmd-right` | Go to the start / end |
| `backspace` / `delete` (or `ctrl-h` / `ctrl-d`) | Delete one character back / forward |
| `option`/`alt`/`ctrl` + `backspace`, `ctrl-w` | Delete the word before the cursor |
| `option`/`alt`/`ctrl` + `delete`, `alt-d` | Delete the word after the cursor |
| `ctrl-u` / `cmd-backspace`, `ctrl-k` | Delete to the start / to the end |
| mouse click | Put the cursor there |

Pasted text goes into the search box as one piece, even when the box isn't focused, so a pasted link doesn't trigger key commands.

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
| `e` | Open / close the effects menu (also the `◂ FX n` button in the HUD) |
| `r` / `R` | Resolution up / down (graphics renderer) |
| `g` | Switch between pixel graphics and text blocks |
| `f` | Blurred background around narrow videos on / off |
| `h` | Hide / show HUD |
| `?` | Help |
| `q` / `esc` / `backspace` | Stop playback and go back |
| `ctrl-c` | Quit |

Effects menu:

`e`, or a click on the `◂ FX n` button at the right end of the HUD's bottom line (`n` is the number of enabled effects), opens a side menu on the right edge of the player. The video shrinks to make room. Effects are listed under group headers, and the description of the selected effect shows at the bottom.

| Key | Action |
| --- | --- |
| `up` `down` / `j` `k` | Move |
| `PgUp` `PgDn` `Home` `End` | Jump |
| `space` / `enter` | Toggle the selected effect |
| `c` | Clear all effects |
| `esc` / `e` | Close the menu |

Mouse: click an effect to toggle it, use the wheel to scroll. While the menu is open, `up` `down` and `space` go to the menu, not to volume and pause. `p` / `P` and `[` `]` still work.

| Group | Effects |
| --- | --- |
| Motion | zoom (punch in on hits), shake, bounce, spin (alternating kick), wave (rows sway), bulge (fisheye pop), mirror (left half to right), kaleido (four-way mirror) |
| Time | trails (motion smear), echo (colour channels lag), tunnel (zooming feedback), stutter (freeze on big hits) |
| Glitch | rgb (chromatic aberration), glitch (datamosh), slice (bands slide apart), static (tv snow), swap (colour channels swap) |
| Color | red (blood-red duotone), noir (black and white), thermal (heat camera), vapor (purple / cyan), hue (drifts, jumps on hits), poster (flat bands), solar (solarized highlights), invert (negative flash) |
| Light | strobe (brightness pumps), flash (white flash), glow (bloom), vignette (dark edges pump), cinema (black bars slam in) |
| Texture | blur (soft focus), pixel (chunky pixels), dither (ordered dither), edges (neon outlines), crt (scanlines, phosphor mask), film (sepia, grain, scratches), vhs (tape wobble, tracking, grain) |

ytfx remembers the enabled effects, preset, intensity, bass sync and background fill across runs, in `~/.config/ytfx/settings.json`. `--no-sync` still turns sync off for one run.

Presets (`p` / `P`; applying one replaces the current effect set):

| Preset | Effects |
| --- | --- |
| edit (default) | zoom, shake, rgb, glitch, vhs |
| phonk red | zoom, shake, red, invert, strobe, blur |
| glitchcore | rgb, glitch, pixel, trails, vhs, shake |
| dream | blur, trails, zoom, mirror |
| vaporwave | vapor, echo, wave, glow, crt |
| noir | noir, film, vignette, cinema, stutter |
| rave | kaleido, tunnel, hue, flash, spin, zoom |
| heat | thermal, bulge, rgb, static, shake |
| broken tv | crt, slice, swap, static, bounce |
| neon | edges, glow, hue, trails, zoom |
| clean | none |

## Tabs

The browser has six tabs. Press `1` to `6`, or `tab` / `shift-tab`, to switch.

1. Search: search results.
2. Home: your recommendations.
3. Subscriptions: the latest videos from channels you subscribe to.
4. Library: Watch later, Liked videos, History, Your playlists and Subscribed channels.
5. Queue: the play queue.
6. History: the videos you played in ytfx, newest first. A video you play again moves to the top.

Home, subscriptions and library need sign-in. History does not: ytfx keeps it on your computer, in `~/.config/ytfx/history.json` (last 500 videos). Enter plays one video from it. `x` removes the selected video from it. To clear it, delete the file. YouTube's own watch history is in Library.

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

Enter on a video in a feed, playlist or channel plays through that list. A video from plain search results or history plays alone. `a` adds the selected video to the queue, `S` shuffle-plays the current list, and `x` removes an item on the queue tab.

When a video ends, the next one starts. ytfx resolves the next video in the background a few seconds into the current one, so it starts without waiting. If 3 videos in a row fail, the queue stops.

Radio: `r` in the browser starts radio from the selected video. `R` starts it from a random video in the current list. `m` in the player starts it from the playing video. Radio uses YouTube's Mix for that video (related videos), shuffled, and never repeats a video. When fewer than 3 videos are left, it fetches a Mix seeded by a random recently played video. If a video has no Mix, radio falls back to a search for its channel and title.

## Browse while playing

`b` or `tab` in the player goes to the browser while playback continues. A now-playing line shows at the bottom. In the browser, `v` returns to the video, `space` pauses, and `n` / `N` skip to the next / previous video. In the player, `q`, `esc` or `backspace` stops playback and goes back. The HUD shows the queue position (for example `[3/25]`) and a RADIO badge.

## Resolution

There are two renderers:

- **Graphics (kitty protocol):** used automatically in kitty, Ghostty and WezTerm. The video is sent as a real image at the resolution you choose (`--res`, or `r` / `R` while playing), and the terminal scales it to fit the window. kitty receives frames through shared memory, so even 720p costs almost nothing to send. Other terminals receive them inline as base64.
- **Blocks:** used everywhere else, and inside tmux. Each cell shows two pixels with the `▀` character, so the resolution is the terminal grid. For example, 200 columns gives a 200x112 picture. To get more detail, make the font smaller.

The source stream is fetched at 720p or at the chosen resolution, whichever is higher. At 720p, each preset takes about 0.7 to 3.2 ms per frame, and no single effect takes more than about 1.5 ms. With all 37 effects on at once, it takes about 3.7 ms at 360p, 14 ms at 720p and 32 ms at 1080p. Blur and glow run at reduced resolution, per-channel curves (poster, solar, strobe, flash, invert) are combined into one pass, and history buffers exist only while their effect is on. While paused, the effects stop running once the last hit has faded, until something changes (a seek, another effect, a resize). With no effects on, it costs almost nothing.

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
- `src/textinput.rs`: one-line text field (cursor, word jumps, editing keys, scrolling)
- `src/youtube.rs`: `yt-dlp` search and stream URL resolution
- `src/account.rs`: sign-in (cookie import, saved account)
- `src/queue.rs`: play queue and radio
- `src/player.rs`: ffmpeg video and audio decoders, A/V sync, seek, resize
- `src/audio.rs`: cpal output, volume, and the tap of played samples
- `src/analysis.rs`: FFT, bass level, beat detection
- `src/effects.rs`: effect pipeline, presets, effect catalogue (groups, descriptions)
- `src/kitty.rs`: kitty graphics protocol output (shared memory or inline base64)
- `src/ui.rs`: ratatui rendering (browser, HUD, effects menu, help, account dialog) and the half-block video widget

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

Three tests are ignored by default because they need input files:

- `real_audio` (`src/analysis.rs`): runs the analyzer on real audio.
  `YTFX_PCM=file.f32 cargo test real_audio -- --ignored --nocapture`, where the file is 48 kHz stereo f32le (for example from `ffmpeg ... -ac 2 -ar 48000 -f f32le`).
- `preview` (`src/effects.rs`): writes one PPM per preset, mid-beat.
  `YTFX_FRAME=frame.rgb YTFX_OUT=dir cargo test preview -- --ignored`, where `frame.rgb` is a raw 200x112 RGB24 frame and `dir` is the output directory.
- `preview_each` (`src/effects.rs`): writes one PPM per effect, on a hit.
  `YTFX_FRAME=frame.rgb YTFX_OUT=dir cargo test preview_each -- --ignored`, with the same inputs as `preview`. Set `YTFX_SIZE=1280x720` for a frame of another size.
