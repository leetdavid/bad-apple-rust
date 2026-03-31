# Bad Apple CLI (badcrabapple) - Implementation Plan

## Goal
Create a standalone, compiled Rust CLI application that plays the "Bad Apple!!" video in the terminal, synchronized with audio. The binary must be entirely self-contained (no external video/audio files required at runtime), dynamically resize to fit the terminal, and support both ASCII and Block character rendering modes.

## Phase 1: Asset Preparation Pipeline (Bash Script)
*   **Objective:** Download the source video and convert it into highly optimized, raw 1-bit binary data and a compressed audio file to embed in the Rust binary.
*   **Steps:**
    1.  Install `yt-dlp` to download the video.
    2.  Extract the audio to a compressed format (e.g., `audio.mp3`) using `ffmpeg`.
    3.  Use `ffmpeg` to process the video:
        *   Scale to a fixed "base" resolution (e.g., 128x96 or 160x120) to balance detail and binary size.
        *   Convert to pure black and white (1-bit / thresholding).
        *   Extract raw frames at exactly 30 FPS.
    4.  Write a small Rust or Python utility (as part of the build process) to pack the extracted 1-bit frames into a dense byte array (`frames.bin`).
        *   *Calculation:* 128x96 = 12,288 pixels = 1,536 bytes per frame.
        *   6570 frames (3m39s at 30fps) * 1536 bytes = ~10MB raw binary file.

## Phase 2: Rust Implementation (`badcrabapple`)
*   **Objective:** Build the high-performance terminal player.
*   **Dependencies:**
    *   `crossterm`: Terminal manipulation (size, cursor control, input handling).
    *   `rodio`: Cross-platform audio playback.
    *   `clap`: Command-line argument parsing (`--mode`).
*   **Embedding Assets:**
    *   Use `include_bytes!("audio.mp3")` and `include_bytes!("frames.bin")` to compile the assets directly into the executable.
*   **The Render Engine (Dynamic Resizing):**
    *   Read current terminal dimensions `(term_width, term_height)` via `crossterm`.
    *   Implement a nearest-neighbor sampling algorithm. For each terminal character cell `(x, y)`, calculate the corresponding pixel coordinate in the 128x96 base frame to determine if it's a 1 (white) or 0 (black).
    *   Account for terminal character aspect ratio (characters are roughly 2x as tall as they are wide).
*   **Rendering Modes:**
    *   **ASCII Mode (`--mode ascii`)**: Map 1 to a bright character (e.g., `#` or `@`) and 0 to a space (` `).
    *   **Block Mode (`--mode block`)**: Sample two vertical pixels from the base frame for every single terminal character to double vertical resolution.
        *   Top 0, Bottom 0 -> ` ` (Space)
        *   Top 1, Bottom 0 -> `▀` (Upper half block)
        *   Top 0, Bottom 1 -> `▄` (Lower half block)
        *   Top 1, Bottom 1 -> `█` (Full block)
*   **Playback Synchronization:**
    *   Start audio playback on a background thread.
    *   Use `std::time::Instant` to track elapsed time.
    *   Calculate current frame index: `elapsed_seconds * 30.0`.
    *   Draw the corresponding frame from the embedded `frames.bin`. This automatically handles frame dropping if the terminal is slow.
    *   Listen for `Ctrl+C` or terminal resize events to redraw or exit cleanly.

## Phase 3: Build & Execution
*   **Steps:**
    1.  Ensure the asset preparation script runs successfully.
    2.  Run `cargo build --release` to compile the optimized binary.
    3.  Test execution with `cargo run --release -- --mode block`.
