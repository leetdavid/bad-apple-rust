# Bad Crab Apple 🦀🍏

A standalone, single-binary CLI player for "Bad Apple!!" written in Rust. Audio and video are compiled directly into the binary — no external files or dependencies needed at runtime.

## Building

```bash
cargo build --release
```

This takes a moment: it embeds ~15MB of compressed audio and video frames into the binary.

## Running

```bash
./target/release/badcrabapple
```

This plays the video in the default **block** mode. The video scales automatically to your terminal size, maintaining the original 4:3 aspect ratio. Press `q`, `Esc`, or `Ctrl+C` to quit.

## Render Modes

The renderer maps each 1-bit pixel to a terminal character. Because terminal characters are taller than they are wide, each row samples **two video rows** (top half and bottom half), giving four possible states per character: empty, top-lit, bottom-lit, and fully-lit.

There are five built-in modes:

| Mode | Flag | Characters | Notes |
|------|------|------------|-------|
| `classic` | `--classic` | `' '` `-` `*` `@` | **Default.** Sampled from `" .:-=+*#%@"`. |
| `block` | `--block` | `' '` `▀` `▄` `█` | Highest visual quality. |
| `ascii` | `--ascii` | `'.'` `^` `v` `@` | Narrow ASCII characters. |
| `shading` | `--shading` | `' '` `░` `▒` `█` | Unicode block shading. |
| `korean` | `--korean` | `시` `뽁` `늙` `뾃` | Wide (2-column) Korean glyphs. Aspect ratio is corrected automatically. |

Use either the flag shorthand or `--mode <name>`:

```bash
./target/release/badcrabapple --shading
./target/release/badcrabapple --mode shading
```

## Custom Characters

### `--chars` — specify all four states directly

Provide exactly 4 characters in the order: **empty · top-lit · bottom-lit · fully-lit**.

```bash
./target/release/badcrabapple --chars " .':"
./target/release/badcrabapple --chars " 가을뿔"
```

### `--gradient` / `-g` — sample from a palette

Provide a string of 2 or more characters from darkest to brightest. Four characters are sampled evenly from it (at 0%, 33%, 67%, 100%) and used for the four states.

```bash
./target/release/badcrabapple -g " .:-=+*#%@"
./target/release/badcrabapple -g " ░▒▓█"
```

For both options, character display width is detected automatically — wide/CJK characters (2 columns) get a different aspect ratio correction than narrow characters (1 column).

## How it works

The original video was thresholded to 1-bit per pixel (pure black and white) and scaled to 160×120. At 1 bit per pixel, 8 pixels pack into a single byte. The resulting `frames.bin` contains 6,572 frames at 30 fps and is ~15MB. Audio is stored as a compressed MP3. Both are embedded into the binary at compile time via `include_bytes!`.
