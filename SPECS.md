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
--force    Force re-download and re-processing even if cached assets exist
```

# Video Processing

When a new source is prepared, the following steps run:

1. **Download** (remote URLs only): `yt-dlp` downloads the video to a temp file as MP4.
2. **Dimensions**: `ffprobe` reads the original width and height, stored in `meta.txt`.
3. **Audio**: `ffmpeg` extracts audio as MP3.
4. **Frames**: `ffmpeg` scales the video to 160px wide (height derived from original aspect ratio) at 30 fps and pipes raw RGB24 pixels (3 bytes per pixel) into `frames.bin`.

Cached files per source: `audio.mp3`, `frames.bin`, `meta.txt`.

# Configuring the Renderer

```bash
bad-apple --block "https://youtube.com/watch?v=..."
bad-apple --ascii "file.mp4"
bad-apple -g " .:-=+*#%@" "https://youtube.com/watch?v=..."
```

## Rendering Technique

Each terminal character cell maps to two video rows (top half and bottom half), producing four possible states per cell: empty, top-lit, bottom-lit, and fully-lit. This doubles the effective vertical resolution.

Narrow characters (1 column wide) receive a ÷2 vertical aspect correction, since terminal cells are approximately twice as tall as they are wide. Wide/CJK characters (2 columns wide) do not receive this correction.

The video scales to fill the terminal while preserving the original aspect ratio.

## Color

All charset modes render in **ANSI 24-bit color by default**. The foreground color of each character is set to the top source pixel's RGB value. The background is left as the terminal default.

Use `--monochrome` to disable color output and render using the terminal's default foreground and background only.

## Built-in Modes

| Mode | Flag | Characters (empty · top · bottom · full) |
|------|------|------------------------------------------|
| `classic` | `--classic` | `' '` `-` `*` `@` |
| `block` | `--block` | `' '` `▀` `▄` `█` |
| `ascii` | `--ascii` | `'.'` `^` `v` `@` |
| `shading` | `--shading` | `' '` `░` `▒` `█` |
| `korean` | `--korean` | `시` `뽁` `늙` `뾃` |
| `full-color` | `--full-color` | Always `▀`; fg = top pixel, bg = bottom pixel |

The default mode is `full-color`.

`full-color` maximizes color fidelity by always using `▀` (upper half block) with independent 24-bit fg/bg per cell — two pixels of color per terminal character, similar to `mpv --vo=tct`.

## Options

```
-m, --mode <name>        Select a built-in render mode by name
--classic                Shorthand for --mode classic
--block                  Shorthand for --mode block
--ascii                  Shorthand for --mode ascii
--shading                Shorthand for --mode shading
--korean                 Shorthand for --mode korean (wide 2-column characters)
--full-color             Shorthand for --mode full-color (default; conflicts with --monochrome)
--monochrome             Disable ANSI colors; use terminal default fg/bg only (conflicts with --full-color)
-c, --chars <chars>      Exactly 4 characters: empty, top-lit, bottom-lit, fully-lit
-g, --gradient <chars>   2+ character palette sampled at 0%, 33%, 67%, 100%
```

Character display width is auto-detected; wide/CJK characters trigger the 2-column aspect correction automatically.

# Controls

| Key | Action |
|-----|--------|
| `left` | Cycle backward between built-in render modes |
| `right` | Cycle forward between built-in render modes |
| `q` | Quit |
| `Esc` | Quit |
| `Ctrl+C` | Quit |
