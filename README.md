# Bad Crab Apple 🦀🍏

A standalone, completely cross-platform, single-binary CLI player for "Bad Apple!!" written in Rust.

All assets (audio and video frames) are aggressively compressed and **compiled directly into the binary as raw bits**, allowing it to play perfectly synchronized video and audio in your terminal without any external dependencies at runtime.

## Features
- **Zero Dependencies:** No need for `ffmpeg`, `vlc`, or external video/audio files at runtime. Everything is inside the executable.
- **Dynamic Resizing:** The video will automatically scale to fit any terminal window size while maintaining its 4:3 aspect ratio.
- **Two Render Modes:**
  - **Block Mode (Default):** Uses Unicode half-block characters (`▀`, `▄`, `█`) to double the vertical resolution of your terminal, providing smooth, high-quality playback.
  - **ASCII Mode:** Uses standard ASCII characters to render the frames.

## Usage

Build the release binary (this will take a moment as it embeds the ~15MB highly compressed asset files):
```bash
cargo build --release
```

Run the application:
```bash
# Run with the default high-resolution Block mode
./target/release/badcrabapple

# Run with standard ASCII mode
./target/release/badcrabapple --mode ascii
```

## Controls
- `q`, `Esc`, or `Ctrl+C` to quit the player early.

## How it works (The Asset Pipeline)
The original video was processed into a 160x120 raw binary format. To achieve maximum compression and allow embedding into the Rust binary, it was thresholded to 1-bit per pixel (pure black and white), allowing 8 pixels to be packed into a single byte. The resulting `frames.bin` file contains exactly 6572 frames and is ~15MB.
