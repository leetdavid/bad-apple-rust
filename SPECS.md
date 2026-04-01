# Dependencies

`bad-apple` requires the following tools to be installed and available in `PATH`:

- **`ffmpeg`** — video frame extraction and audio extraction
- **`ffprobe`** — reads original video dimensions (bundled with ffmpeg)
- **`yt-dlp`** — downloads remote URLs (YouTube and others); not required for local files

# Selecting a Video Source

```bash
bad-apple "https://youtube.com/watch?v=..." # Download and play a YouTube video
bad-apple "file.mp4"                        # Play a local video file
bad-apple                                   # Replay the last prepared video
```

Parsed assets are cached in `~/.cache/bad-apple/`, keyed by a hash of the source URL or file path. If cached assets for the given source already exist, they are loaded directly without re-processing. Run with no arguments to replay the most recently prepared video.

## Options

```
--force                          Force re-download and re-processing even if cached assets exist
--width <N>                      Frame width for preprocessing (default: 160); use --force to change
--cookies-from-browser <browser> Pass browser cookies to yt-dlp (e.g. chrome, firefox)
--cookies <file>                 Pass a Netscape cookies file to yt-dlp
--extractor-args <args>          Pass --extractor-args to yt-dlp (e.g. "youtube:player_client=ios")
```

# Video Processing

When a new source is prepared, the following steps run:

1. **Download** (remote URLs only): `yt-dlp` downloads the video to a temp file as MP4, capped at 480p.
2. **Dimensions**: `ffprobe` reads the original width and height.
3. **Audio**: `ffmpeg` extracts audio as `audio.mp3`.
4. **Frames**: `ffmpeg` extracts all frames as raw RGB24 into `frames.bin` at the configured `--width` and the proportional height.
5. **Video**: `video.mp4` is kept in cache for future re-processing at a different `--width`.

`meta.txt` stores: `orig_w orig_h frame_w frame_h`.

If `frames.bin` already exists at the requested `--width`, the extraction step is skipped. Use `--force` to re-process; `video.mp4` is reused so no re-download is needed.

# Configuring the Renderer

```bash
bad-apple --full-color "https://youtube.com/watch?v=..."
bad-apple --ascii "file.mp4"
bad-apple -g " .:-=+*#%@" "https://youtube.com/watch?v=..."
```

## Rendering Technique

Each terminal character cell maps to two video rows (top half and bottom half), producing four possible states per cell: empty, top-lit, bottom-lit, and fully-lit. This doubles the effective vertical resolution.

Narrow characters (1 column wide) receive a ÷2 vertical aspect correction, since terminal cells are approximately twice as tall as they are wide. Wide/CJK characters (2 columns wide) do not receive this correction.

The video scales to fill the terminal while preserving the original aspect ratio. Frames are read directly from `frames.bin` at playback time — no runtime ffmpeg dependency.

## Color

Each mode has a default color setting baked in (see table below). Switching modes with `left`/`right` also switches the color setting to that mode's default.

Use `--monochrome` to force monochrome output regardless of mode (conflicts with `--full-color`).

Press `c` during playback to toggle color on/off.

## Built-in Modes

| Mode          | Flag            | Color   | Characters                                    |
|---------------|-----------------|---------|-----------------------------------------------|
| `full-color`  | `--full-color`  | yes     | Always `▀`; fg = top pixel, bg = bottom pixel |
| `ascii-color` | `--ascii-color` | yes     | `.:-=+*#%@` gradient                          |
| `ascii`       | `--ascii`       | no      | `.:-=+*#%@` gradient                          |
| `braille`     | `--braille`     | yes     | `⠁⠃⠇⠧⠷⠿` gradient                             |
| `korean`      | `--korean`      | yes     | `ㅇㅎ시늙뀪뾃` wide-character gradient           |
| `shading`     | `--shading`     | no      | ` ░▒█` block shading                          |

The default mode is `full-color`.

`full-color` maximizes color fidelity by always using `▀` (upper half block) with independent 24-bit fg/bg per cell — two pixels of color per terminal character, similar to `mpv --vo=tct`.

## Options

```
-m, --mode <name>        Select a built-in render mode by name
--full-color             ▀ with 24-bit fg/bg color (default; conflicts with --monochrome)
--ascii-color            .:-=+*#%@ gradient with color
--ascii                  .:-=+*#%@ gradient, monochrome
--braille                ⠁⠃⠇⠧⠷⠿ braille gradient with color
--korean                 Korean wide-character gradient with color
--shading                ░▒█ block shading, monochrome
--monochrome             Force monochrome regardless of mode (conflicts with --full-color)
-c, --chars <chars>      Exactly 4 characters: empty, top-lit, bottom-lit, fully-lit
-g, --gradient <chars>   2+ character palette sampled at 0%, 33%, 67%, 100%
```

Character display width is auto-detected; wide/CJK characters trigger the 2-column aspect correction automatically.

# Controls

| Key         | Action                                    |
|-------------|-------------------------------------------|
| `left`      | Cycle backward through built-in modes     |
| `right`     | Cycle forward through built-in modes      |
| `k` / space | Pause / unpause                           |
| `j`         | Rewind 10 seconds                         |
| `l`         | Fast-forward 10 seconds                   |
| `c`         | Toggle color / monochrome                 |
| `q`         | Quit                                      |
| `Esc`       | Quit                                      |
| `Ctrl+C`    | Quit                                      |
