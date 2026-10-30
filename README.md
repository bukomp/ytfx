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
--cookies-from-browser <name>
                   pass browser cookies to yt-dlp (chrome, firefox, safari, ...)
--cookies <file>   pass a Netscape cookies.txt file to yt-dlp
-h, --help         show help
```

## Keys

Press `?` for an in-app key list.

Browser:

| Key | Action |
| --- | --- |
| `/` or `i` | Focus the search box |
| `enter` | Search (in the box) / play selected result |
| `j` `k` / `down` `up` | Move selection |
| `PgUp` `PgDn` | Move selection by 10 |
| `g` `G` / `Home` `End` | First / last result |
| `tab` | Switch between search box and results |
| `?` | Help |
| `q` / `esc` | Quit |

In the search box, `esc` (or `tab` / `down`) moves to the results when there are any; `esc` quits when there are none. Pasting a URL that starts with `http://` or `https://` plays it directly.

Player:

| Key | Action |
| --- | --- |
| `space` | Pause / resume |
| `left` `right` | Seek -10 s / +10 s |
| `up` `down` | Volume up / down |
| `s` | Toggle bass sync |
| `p` / `P` | Next / previous preset |
| `[` `]` | Effect intensity down / up (0.1 to 2.0) |
| `1-9` `0` `-` `=` | Toggle individual effects (map below) |
| `r` / `R` | Resolution up / down (graphics renderer) |
| `g` | Switch between pixel graphics and text blocks |
| `h` | Hide / show HUD |
| `?` | Help |
| `q` / `esc` / `backspace` | Back to results |
| `ctrl-c` | Quit |

Effect keys:

| Key | Effect | Key | Effect | Key | Effect |
| --- | --- | --- | --- | --- | --- |
| `1` | zoom | `5` | red | `9` | vhs |
| `2` | shake | `6` | glitch | `0` | pixel |
| `3` | rgb | `7` | trails | `-` | mirror |
| `4` | blur | `8` | invert | `=` | strobe |

Presets (`p` / `P`; applying one replaces the current effect set):

| Preset | Effects |
| --- | --- |
| edit (default) | zoom, shake, rgb, glitch, vhs |
| phonk red | zoom, shake, red, invert, strobe, blur |
| glitchcore | rgb, glitch, pixel, trails, vhs, shake |
| dream | blur, trails, zoom, mirror |
| clean | none |

## Resolution

There are two renderers:

- **Graphics (kitty protocol):** used automatically in kitty, Ghostty and WezTerm. The video is sent as a real image at the resolution you choose (`--res`, or `r` / `R` while playing), and the terminal scales it to fit the window. kitty receives frames through shared memory, so even 720p costs almost nothing to send. Other terminals receive them inline as base64.
- **Blocks:** used everywhere else, and inside tmux. Each cell shows two pixels with the `▀` character, so the resolution is the terminal grid. For example, 200 columns gives a 200x112 picture. To get more detail, make the font smaller.

The source stream is fetched at 720p or at the chosen resolution, whichever is higher. With every effect on, processing takes about 15 ms per frame at 720p and about 30 ms at 1080p, so 1080p is near the limit at 30 fps.

## How the bass sync works

ytfx decodes the audio itself with ffmpeg and plays it through cpal. That means it can tap the exact samples it sends to the speakers. No system audio capture, loopback device or OS permission is needed.

Each frame the analyzer:

1. Runs a 2048-point FFT on the latest played audio.
2. Sums the 30-150 Hz band energy, with auto-gain, to get a "level".
3. Computes spectral flux in that band and compares it to an adaptive threshold (mean + 1.5 sigma over about 1.5 s of history). A flux above the threshold is a beat.
4. Sets a beat pulse that jumps on a beat and decays exponentially. The pulse scales every effect.

Video frames are scheduled against the audio clock (samples actually played), so audio and video stay in sync.

## Architecture

- `src/main.rs`: argument parsing, app state, main loop, key handling
- `src/youtube.rs`: `yt-dlp` search and stream URL resolution
- `src/player.rs`: ffmpeg video and audio decoders, A/V sync, seek, resize
- `src/audio.rs`: cpal output, volume, and the tap of played samples
- `src/analysis.rs`: FFT, bass level, beat detection
- `src/effects.rs`: effect pipeline, presets, effect key map
- `src/kitty.rs`: kitty graphics protocol output (shared memory or inline base64)
- `src/ui.rs`: ratatui rendering (browser, HUD, help) and the half-block video widget

## Troubleshooting

- "Sign in to confirm you're not a bot": YouTube flags some IP addresses (VPNs, datacenters, shared networks). Try these in order:
  1. Update yt-dlp (`brew upgrade yt-dlp`). YouTube changes often and yt-dlp ships fixes often.
  2. Turn off your VPN or try another network.
  3. Export a `cookies.txt` yourself (ideally from a throwaway Google account) and pass `--cookies cookies.txt`. Only that file is shared, and no browser or Keychain access is needed.
  4. `--cookies-from-browser <name>` reads your browser's cookie store directly. On macOS, Chromium browsers ask for Keychain access to decrypt it. Use this only if you are comfortable with that.
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
