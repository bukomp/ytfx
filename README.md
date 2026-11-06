# ytfx

A terminal YouTube browser. It searches with `yt-dlp` (no API key), plays the video in the terminal with audio (as real pixels in kitty, Ghostty and WezTerm, as sixel images in foot and other sixel terminals, or as truecolor half-block characters elsewhere), and layers 37 "edit"-style effects on top (zoom punches, shake, glitches, colour grades, VHS, CRT, film and more), picked from a side menu. With SYNC on, the effects are driven by the frequency slices that are on (by default the bass, 0-150 Hz). Press `s` to turn SYNC off; the effects then stay at a steady level with random hits.

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
--renderer <name>  auto (default), kitty or sixel (pixel graphics), blocks (text cells)
--max-width <n>    max video width in columns for the blocks renderer (default 200)
--no-sync          start with sync off
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
| `s` | Toggle sync (beats from the frequency slices that are on; off: steady effects and random hits) |
| `p` / `P` | Next / previous preset |
| `[` `]` | Effect intensity down / up (0.1 to 2.0) |
| `,` `.` | Audio delay 10 ms shorter / longer (see "Audio delay") |
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

The menu has two pages, `effects` and `frequencies`. Press `tab` or click a page name at the top to switch.

| Key | Action |
| --- | --- |
| `tab` | Switch between the effects and frequencies pages |
| `up` `down` / `j` `k` | Move |
| `PgUp` `PgDn` `Home` `End` | Jump |
| `space` / `enter` | Toggle the selected effect |
| `c` | Clear all effects |
| `esc` / `e` | Close the menu |

Mouse: click an effect to toggle it, use the wheel to scroll. While the menu is open, `up` `down` and `space` go to the menu, not to volume and pause, and `tab` switches the page (`b` still goes to the browser). `p` / `P` and `[` `]` still work.

Frequencies page:

The frequencies page is a panel across the full width of the player, above the HUD. The video gets shorter instead of narrower while it is open. The panel shows the spectrum from 0 to 12 kHz with one column per 50 Hz slice:

- The tall upper part is a live level meter for each slice, so you can see which slices react to the kick, the snare or the hats. A column flashes white when its slice hits. The meters show the same magnitudes that the detection reads, from the same FFTs.
- The axis under it shows frequency labels (`100`, `250`, `500`, `1k`, ...), `┼` at the band edges, `▲` under the cursor and `━` under the other selected slices. Selected columns are shaded.
- The lower part shows each slice's hit sensitivity, from `off` (`·`) at the bottom to 2.0 at the top. 1.0 is half height.
- The header lists the slices that are on as frequency ranges (for example `on 0-150 1000-1050 Hz`), colored by band. A `beat` dot after them lights up on each beat. The bottom line shows the selected slices: their frequency range, how many there are, their bands and their values.

When the terminal is narrower than 240 columns, the panel scrolls to keep the cursor in view.

Every slice that is on watches for its own onsets, with its own sensitivity: a higher value lets softer hits through. With sync on, a hit in any slice that is on is a beat, whatever its frequency. It drives the effects fully and fires the one-shot effects (glitch, invert, stutter, swap). Thus, to make a snare fire the effects, turn on the slices it shows up in. The level meters show which slices those are. Each slice that is on detects onsets in its own level only. There is no band check. A held bass note that moves into another slice that is on (a legato note change, for example 49 Hz to 73 Hz, which moves from the 0-50 Hz slice to the 50-100 Hz slice) is a real rise in that slice, and it can fire. To stop this, turn that slice down or off.

By default, only 0-150 Hz (kicks) is on at 1.0. Everything above it is `off`, because every slice that is on fires beats.

The slices are grouped into three bands, colored in the panel: `bass` (0-250 Hz, red), `mid` (250-4000 Hz, orange) and `high` (4000-12000 Hz, blue). The bands only group and color the slices. They do not change detection. The level that drives the steady part of the effects comes from all slices that are on together.

| Key | Action |
| --- | --- |
| `left` `right` | Select the previous / next slice |
| `option` (or `ctrl`) + `left` `right` | Jump 10 slices (add `shift` to grow the selection by 10) |
| `shift` + `left` `right` (or `PgUp` `PgDn` `Home` `End`) | Grow or shrink a selection of slices |
| `PgUp` `PgDn` `Home` `End` | Jump 20 slices / to the first or last slice |
| `a` | Select all slices of the selected slice's band (again: select one slice) |
| `up` `down` / `+` `-` | Set every selected slice 0.1 higher / lower, down to `off` |
| `0` | Set the selected slices to `off` |
| `space` / `enter` | Set the selected slices to their defaults |
| `c` | Set all slices to their defaults |
| `esc` | Drop the selection (with no selection, close the menu) |
| `tab` | Back to the effects page |

To turn a whole band off, press `a` and then `0`. Each selected slice moves by the same step, so the shape of a curve stays until slices reach `off` or 2.0.

While two or more slices are selected, you hear only their frequencies (solo, shown as `♪ solo` on the bottom line). Use this to find the slices of a sound: select a range and listen. The beat detection and the level meters still get the full audio. Select one slice (`esc`, or `left` / `right` without `shift`), close the menu or change the page to hear everything again.

Mouse: click a column in the level meters to select it, or drag across them to select a range. Click in the sensitivity part to set a slice to 0, 0.5, 1.0, 1.5 or 2.0 (bottom to top). A click inside a selection sets all of the selected slices. Drag across the sensitivity part to set each slice on the way, like drawing a curve. Use the wheel to scroll.

| Group | Effects |
| --- | --- |
| Motion | zoom (punch in on hits), shake, bounce, spin (alternating kick), wave (rows sway), bulge (fisheye pop), mirror (left half to right), kaleido (four-way mirror) |
| Time | trails (motion smear), echo (colour channels lag), tunnel (zooming feedback), stutter (freeze on big hits) |
| Glitch | rgb (chromatic aberration), glitch (datamosh), slice (bands slide apart), static (tv snow), swap (colour channels swap) |
| Color | red (blood-red duotone), noir (black and white), thermal (heat camera), vapor (purple / cyan), hue (drifts, jumps on hits), poster (flat bands), solar (solarized highlights), invert (negative flash) |
| Light | strobe (brightness pumps), flash (white flash), glow (bloom), vignette (dark edges pump), cinema (black bars slam in) |
| Texture | blur (soft focus), pixel (chunky pixels), dither (ordered dither), edges (neon outlines), crt (scanlines, phosphor mask), film (sepia, grain, scratches), vhs (tape wobble, tracking, grain) |

ytfx remembers the enabled effects, preset, intensity, frequency sensitivity, audio delay, sync and background fill across runs, in `~/.config/ytfx/settings.json`. `--no-sync` still turns sync off for one run.

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

The browser has seven tabs. Press `1` to `7`, or `tab` / `shift-tab`, to switch.

1. Search: search results.
2. Home: your recommendations.
3. Subscriptions: the latest videos from channels you subscribe to.
4. Library: Watch later, Liked videos, History, Your playlists and Subscribed channels.
5. Queue: the play queue.
6. History: the videos you played in ytfx, newest first. A video you play again moves to the top.
7. Settings: app-wide settings (see "Audio delay").

Home, subscriptions and library need sign-in. History does not: ytfx keeps it on your computer, in `~/.config/ytfx/history.json` (last 500 videos). Enter plays one video from it. `x` removes the selected video from it. To clear it, delete the file. YouTube's own watch history is in Library.

## Audio delay

Some audio outputs play the sound well after ytfx sends it. Bluetooth headphones usually take about 150-300 ms, while wired output and built-in speakers take almost none. With such an output, the video and the effects come before the sound you hear. The audio delay setting holds back the video and the effects by a set time, so they line up with the sound again. The sound itself is not delayed.

Set it in the settings tab (`7`):

| Key | Action |
| --- | --- |
| `left` `right` / `-` `+` | Delay 10 ms shorter / longer |
| `shift` + `left` `right` | 50 ms shorter / longer |
| `0` | No delay |
| `t` / `enter` | Start / stop the sync test |

The range is 0 to 1000 ms.

The sync test is the easiest way to set the delay. It plays a click every 0.6 s (every fourth click is higher) and flashes a block on the page for each click, at the delayed time. Every fourth flash is bigger and white, for the high clicks. Change the delay until each flash comes exactly with its click. The big flashes help when the delay is longer than the time between two clicks. While the test runs, a playing video holds still, and it goes on where it was when the test stops. Leaving the settings tab stops the test.

While something plays, the settings tab also shows a beat light that flashes on each beat at the delayed time. Play a track with a clear kick, and change the delay until the light flashes with each kick you hear. In the player, `,` and `.` also change the delay by 10 ms, so you can fine-tune it against the video. ytfx saves the delay with the other settings.

## Sign-in

Press `L` to open the account dialog. Sign-in persists across runs.

A `cookies.txt` in `~/.config/ytfx/` (or `$XDG_CONFIG_HOME/ytfx/`) is used automatically at startup, also when you copy it there by hand. ytfx removes the lines for sites other than youtube.com and sets the mode to 0600. If the file has no youtube.com sign-in, ytfx starts signed out and shows the reason. A browser chosen with `b` takes priority over the file.

- `f`: import a `cookies.txt` file. Export it from a youtube.com tab with a browser extension (for example "Get cookies.txt LOCALLY"). ytfx keeps only the youtube.com cookies, in `~/.config/ytfx/cookies.txt` (or `$XDG_CONFIG_HOME/ytfx`) with mode 0600. You can delete the exported file afterwards.
- `b`: let yt-dlp read a browser's cookie store (firefox, chrome, chromium, edge, brave, safari, vivaldi, opera). On macOS, Chromium browsers ask for Keychain access. Safari needs Full Disk Access for the terminal. Firefox needs neither. On Linux, Chromium browsers keep the cookie key in the GNOME keyring or KWallet. ytfx reads `--password-store=` from the browser's flags file (for example `~/.config/chromium-flags.conf`) and passes that keyring to yt-dlp. The GNOME keyring needs the Python `secretstorage` module (`python-secretstorage` on Arch).
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

There are three renderers:

- **Graphics (kitty protocol):** used automatically in kitty, Ghostty and WezTerm. The video is sent as a real image at the resolution you choose (`--res`, or `r` / `R` while playing), and the terminal scales it to fit the window. kitty receives frames through shared memory, so even 720p costs almost nothing to send. Other terminals receive them inline as base64.
- **Graphics (sixel):** used automatically in terminals that report sixel support (foot, mlterm, xterm with sixel, Windows Terminal...). ytfx asks the terminal at startup, so it works whatever `TERM` says. The terminal doesn't scale sixel images, so ytfx resamples each frame to the pixel size of the video area, maps it to 252 colours with ordered dithering, and sends it as one image (about 0.7 MB and 10 ms per frame for a 1280x720 area). `r` / `R` set the source resolution, as with kitty. A paused frame is not sent again.
- **Blocks:** used everywhere else, and inside tmux. Each cell shows two pixels with the `▀` character, so the resolution is the terminal grid. For example, 200 columns gives a 200x112 picture. To get more detail, make the font smaller.

The source stream is fetched at 720p or at the chosen resolution, whichever is higher. At 720p, each preset takes about 0.7 to 3.2 ms per frame, and no single effect takes more than about 1.5 ms. With all 37 effects on at once, it takes about 3.7 ms at 360p, 14 ms at 720p and 32 ms at 1080p. Blur and glow run at reduced resolution, per-channel curves (poster, solar, strobe, flash, invert) are combined into one pass, and history buffers exist only while their effect is on. While paused, the effects stop running once the last hit has faded, until something changes (a seek, another effect, a resize). With no effects on, it costs almost nothing.

When the video doesn't fill its area (another aspect ratio, or smaller than the window), ytfx fills the empty space with a zoomed, blurred and darkened copy of the video. `f` turns this off.

## How the sync works

ytfx decodes the audio itself with ffmpeg and plays it through cpal. That means it can tap the exact samples it sends to the speakers. No system audio capture, loopback device or OS permission is needed.

The analyzer works in fixed steps. It does not depend on the frame rate. Each step:

1. Takes the next 512 audio samples that were heard (about 94 steps a second at 48 kHz). Steps are counted from the audio, not from drawn frames, so a late or uneven frame (slow drawing) does not change what fires. It runs two FFTs: 2048 samples (43 ms, 23 Hz bins) for most slices, and 4096 samples (85 ms, 12 Hz bins) for the slices below 100 Hz. At 23 Hz, a 50 Hz slice is only two bins wide, and a loud held sub bass leaks into it. The longer window keeps a kick or stomp on top of a sub bass visible.
2. Cuts the spectrum into 50 Hz slices and sums the slices that are on (by default 0-150 Hz, weighted by each slice's sensitivity) to get the energy. When the lowest slice that is on is a bass slice, the energy is averaged over two steps.
3. Scales the energy with an auto-gain (a slow peak follower with a floor) to get a "level". The level drives the steady part of the effects.
4. Fires a beat on an onset in any slice that is on: the slice must rise by much more than usual (its rise in log terms over the quietest of its last 4 steps must be above the mean plus 2 standard deviations of its last second, and at least a minimum jump, both lower at a higher sensitivity). 4 steps are about the length of the FFT window, so a hit counts in full wherever it falls between steps. The slice must also be at least 40% of its own recent peak. Each slice's peak has a floor that is higher for bass, so hiss and the leak of a louder sound elsewhere (a snare leaks a little into the bass slices) do not fire. This also works when the bass never gets quiet between kicks, as in dense mixes with a loud sub bass. Held or swelling bass lines, such as orchestral low strings or long 808s, change too slowly to trigger effects. A held note does not fire while it stays in one slice.
5. Sets a beat pulse that jumps on a beat and decays exponentially. The pulse scales every effect.

Video frames are scheduled against the audio clock (samples actually played, minus the audio delay), so audio and video stay in sync. The analyzer also reads the audio from the audio delay ago, so the effects fire when you hear the beat.

## Architecture

- `src/main.rs`: argument parsing, app state, main loop, key handling
- `src/textinput.rs`: one-line text field (cursor, word jumps, editing keys, scrolling)
- `src/youtube.rs`: `yt-dlp` search and stream URL resolution
- `src/account.rs`: sign-in (cookie import, saved account)
- `src/queue.rs`: play queue and radio
- `src/player.rs`: ffmpeg video and audio decoders, A/V sync, seek, resize
- `src/audio.rs`: cpal output, volume, and the tap of played samples
- `src/analysis.rs`: FFT, slice levels, per-slice beat detection
- `src/effects.rs`: effect pipeline, presets, effect catalogue (groups, descriptions)
- `src/graphics.rs`: picks between the kitty and sixel output
- `src/kitty.rs`: kitty graphics protocol output (shared memory or inline base64)
- `src/sixel.rs`: sixel output (terminal probe, resampling, dithering, encoder)
- `src/ui.rs`: ratatui rendering (browser, HUD, effects menu, help, account dialog) and the half-block video widget

## Troubleshooting

- "Sign in to confirm you're not a bot": YouTube flags some IP addresses (VPNs, datacenters, shared networks). Try these in order:
  1. Update yt-dlp (`brew upgrade yt-dlp`). YouTube changes often and yt-dlp ships fixes often.
  2. Turn off your VPN or try another network.
  3. Press `L` and choose `f` to import a `cookies.txt` you exported yourself (ideally from a secondary Google account). Only that file is shared, and no browser or Keychain access is needed.
  4. Press `L` and choose `b` to let yt-dlp read your browser's cookie store. On macOS, Chromium browsers ask for Keychain access to decrypt it. Use this only if you are comfortable with that.
- "YouTube says this sign-in is no longer valid" or empty Home / Subscriptions: a browser that is signed in to Google refreshes the whole Google session in the background, even with no YouTube tab open, and that invalidates exported copies. Export from a private window instead: sign in to YouTube, open `https://www.youtube.com/robots.txt` in the same tab, export the youtube.com cookies, and close the private window right away. Then import the file with `L` → `f`, or copy it to `~/.config/ytfx/cookies.txt`. That session belongs to ytfx alone, and ytfx keeps it fresh by saving the cookies YouTube updates after each request.
- Low resolution: in kitty, Ghostty, WezTerm or a sixel terminal such as foot, press `r`. If `g` says the terminal shows no pixel graphics, try `--renderer sixel`. In other terminals, the blocks renderer is limited by the cell grid (see Resolution).
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
