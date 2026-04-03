mod prepare;

use clap::Parser;
use ratatui::{
    buffer::Buffer,
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    layout::Rect,
    style::{Color, Modifier, Style},
};
use rodio::{Decoder, OutputStream, Sink};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

const FPS: f64 = 30.0;

/// Placement of the video within the terminal.
/// All coordinates are 0-indexed terminal columns/rows.
#[derive(Debug, PartialEq)]
struct VideoLayout {
    /// Number of character-slot columns (draw_w * col_step = terminal columns used).
    draw_w: u16,
    /// Number of terminal rows used by the video.
    draw_h: u16,
    /// Leftmost terminal column of the video.
    offset_x: u16,
    /// Topmost terminal row of the video.
    offset_y: u16,
    /// The terminal row that the controls bar occupies (always the last row).
    bar_row: u16,
}

/// Compute how the video should be laid out in the terminal.
///
/// The last row is always reserved for the controls bar.
/// The video is scaled to fill as much of the remaining area as possible while
/// preserving aspect ratio, then centered.
fn compute_layout(term_w: u16, term_h: u16, vid_aspect: f32, col_step: u16) -> VideoLayout {
    // Reserve the last terminal row for the controls bar.
    let bar_row = term_h.saturating_sub(1);
    let video_h = bar_row; // rows available to the video: 0 .. bar_row-1

    let aspect_correction = if col_step == 1 { 2.0_f32 } else { 1.0_f32 };
    let max_chars = term_w / col_step.max(1);

    // Start by trying to fill the full width.
    let mut draw_w = max_chars;
    let mut draw_h = (draw_w as f32 / vid_aspect / aspect_correction).round() as u16;

    // If that overflows the video rows, clamp to the available height instead.
    if draw_h > video_h {
        draw_h = video_h;
        draw_w = (video_h as f32 * aspect_correction * vid_aspect).round() as u16;
    }

    let draw_w = draw_w.max(1);
    let draw_h = draw_h.max(1);

    let draw_w_cols = draw_w * col_step;
    // Center horizontally; keep aligned to col_step boundaries for wide chars.
    let offset_x = (term_w.saturating_sub(draw_w_cols) / 2 / col_step) * col_step;
    // Center vertically within the video area (rows 0..bar_row).
    let offset_y = video_h.saturating_sub(draw_h) / 2;

    VideoLayout { draw_w, draw_h, offset_x, offset_y, bar_row }
}

#[derive(Parser)]
#[command(version, about = "Bad Apple CLI player")]
struct Cli {
    /// URL or local file path of a video to download, preprocess, and play.
    /// Accepts any URL supported by yt-dlp (YouTube, direct links, etc.).
    /// Omit to replay the last prepared video.
    url: Option<String>,

    /// Force re-download of the video (also triggers re-processing).
    #[arg(long)]
    download: bool,

    /// Force re-processing from the cached video without re-downloading.
    #[arg(long)]
    process: bool,

    /// Render mode [possible values: full-color, ascii-color, ascii, braille, korean, shading]
    #[arg(short, long, default_value = "full-color")]
    mode: String,

    /// ▀ with 24-bit fg/bg color per pixel
    #[arg(long = "full-color", conflicts_with_all = ["ascii_color", "ascii", "braille", "korean", "shading", "monochrome"])]
    full_color: bool,

    /// .:-=+*#%@ gradient with color
    #[arg(long = "ascii-color", conflicts_with_all = ["full_color", "ascii", "braille", "korean", "shading"])]
    ascii_color: bool,

    /// .:-=+*#%@ gradient, monochrome
    #[arg(long, conflicts_with_all = ["full_color", "ascii_color", "braille", "korean", "shading"])]
    ascii: bool,

    /// ⠁⠃⠇⠧⠷⠿ braille gradient with color
    #[arg(long, conflicts_with_all = ["full_color", "ascii_color", "ascii", "korean", "shading"])]
    braille: bool,

    /// Korean wide-character gradient with color
    #[arg(long, conflicts_with_all = ["full_color", "ascii_color", "ascii", "braille", "shading"])]
    korean: bool,

    /// ░▒█ block shading, monochrome
    #[arg(long, conflicts_with_all = ["full_color", "ascii_color", "ascii", "braille", "korean"])]
    shading: bool,

    /// Disable ANSI colors (overrides mode default).
    #[arg(long, conflicts_with_all = ["full_color"])]
    monochrome: bool,

    /// Custom characters for the four pixel states, in order: empty top-only bottom-only full.
    /// Example: --chars " 가을뿔"
    /// Width (narrow/wide) is auto-detected from the characters.
    #[arg(short = 'c', long)]
    chars: Option<String>,

    /// Gradient palette string (2+ characters, darkest to brightest).
    /// Four characters are sampled evenly from it for the four pixel states.
    /// Example: --gradient " .:-=+*#%@"
    /// Width (narrow/wide) is auto-detected from the characters.
    #[arg(short = 'g', long)]
    gradient: Option<String>,

    /// Frame width to use when preprocessing (default: 320).
    /// Smaller values are faster to prepare and use less disk space.
    /// Use --process to re-process at a different width.
    #[arg(long, default_value = "320")]
    width: u32,

    /// Pass cookies from a browser to yt-dlp to bypass bot detection.
    /// Example: --cookies-from-browser chrome  (or firefox, safari, edge)
    #[arg(long = "cookies-from-browser")]
    cookies_from_browser: Option<String>,

    /// Pass a Netscape-format cookies file to yt-dlp.
    /// Example: --cookies ~/cookies.txt
    #[arg(long)]
    cookies: Option<String>,

    /// Pass --extractor-args to yt-dlp.
    /// Example: --extractor-args "youtube:player_client=ios"
    #[arg(long)]
    extractor_args: Option<String>,

    /// Brightness adjustment from -1.0 (darkest) to 1.0 (brightest), default 0.0.
    /// Up/down arrows adjust brightness at runtime.
    #[arg(short = 'b', long, default_value = "0", allow_hyphen_values = true)]
    brightness: f32,
}

struct CharSet {
    empty: char,
    top: char,
    bottom: char,
    full: char,
    /// Terminal columns each character occupies (1 = narrow, 2 = wide)
    col_width: u16,
}

impl CharSet {
    fn from_str(s: &str) -> Result<Self, String> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != 4 {
            return Err(format!(
                "--chars requires exactly 4 characters, got {}",
                chars.len()
            ));
        }
        let col_width = chars
            .iter()
            .map(|c| c.width().unwrap_or(1) as u16)
            .max()
            .unwrap_or(1)
            .max(1);
        Ok(CharSet {
            empty: chars[0],
            top: chars[1],
            bottom: chars[2],
            full: chars[3],
            col_width,
        })
    }

    fn ascii() -> Self {
        // Sampled from ".:-=+*#%@": positions 0, 2, 5, 8
        CharSet {
            empty: '.',
            top: '-',
            bottom: '*',
            full: '@',
            col_width: 1,
        }
    }

    fn braille() -> Self {
        // Sampled from "⠁⠃⠇⠧⠷⠿": positions 0, 1, 3, 5
        CharSet {
            empty: '⠁',
            top: '⠃',
            bottom: '⠧',
            full: '⠿',
            col_width: 1,
        }
    }

    fn from_gradient(s: &str) -> Result<Self, String> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() < 2 {
            return Err(format!(
                "--gradient requires at least 2 characters, got {}",
                chars.len()
            ));
        }
        let last = chars.len() - 1;
        let indices = [0, (last * 1 / 3).min(last), (last * 2 / 3).min(last), last];
        let sampled = [
            chars[indices[0]],
            chars[indices[1]],
            chars[indices[2]],
            chars[indices[3]],
        ];
        let col_width = sampled
            .iter()
            .map(|c| c.width().unwrap_or(1) as u16)
            .max()
            .unwrap_or(1)
            .max(1);
        Ok(CharSet {
            empty: sampled[0],
            top: sampled[1],
            bottom: sampled[2],
            full: sampled[3],
            col_width,
        })
    }

    fn shading() -> Self {
        CharSet {
            empty: ' ',
            top: '░',
            bottom: '▒',
            full: '█',
            col_width: 1,
        }
    }

    fn korean() -> Self {
        CharSet {
            empty: 'ㅇ',
            top: '시',
            bottom: '늙',
            full: '뾃',
            col_width: 2,
        }
    }
}

enum RenderMode {
    Charset(CharSet),
    FullColor,
}

fn main() {
    let cli = Cli::parse();

    let frame_width = cli.width as usize;

    let cache_entry = if cli.url.is_some() || cli.download || cli.process {
        match prepare::run(
            cli.url,
            cli.download,
            cli.process,
            frame_width,
            cli.cookies_from_browser,
            cli.cookies,
            cli.extractor_args,
        ) {
            Ok(dir) => dir,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        match prepare::last_cache_entry() {
            Some(dir) => dir,
            None => {
                eprintln!("No cached video found. Run: bad-apple <URL>");
                std::process::exit(1);
            }
        }
    };

    let meta = match prepare::load_meta(&cache_entry) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("Error loading video metadata: {e}");
            std::process::exit(1);
        }
    };

    let audio_bytes = std::fs::read(cache_entry.join("audio.mp3")).unwrap_or_else(|_| {
        eprintln!("Audio not found. Re-run: bad-apple --force <URL>");
        std::process::exit(1);
    });

    let monochrome = cli.monochrome;

    let mode = if cli.full_color {
        "full-color"
    } else if cli.ascii_color {
        "ascii-color"
    } else if cli.ascii {
        "ascii"
    } else if cli.braille {
        "braille"
    } else if cli.korean {
        "korean"
    } else if cli.shading {
        "shading"
    } else {
        cli.mode.as_str()
    };

    // Each builtin mode carries its default colorize setting.
    const BUILTIN_MODES: &[(&str, bool)] = &[
        ("full-color", true),
        ("ascii-color", true),
        ("ascii", false),
        ("braille", true),
        ("korean", true),
        ("shading", false),
    ];
    let mut mode_idx = BUILTIN_MODES
        .iter()
        .position(|&(m, _)| m == mode)
        .unwrap_or(0);

    let mut render_mode = if let Some(s) = &cli.chars {
        match CharSet::from_str(s) {
            Ok(cs) => RenderMode::Charset(cs),
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else if let Some(s) = &cli.gradient {
        match CharSet::from_gradient(s) {
            Ok(cs) => RenderMode::Charset(cs),
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else {
        render_mode_for_builtin(mode)
    };

    let mode_default_colorize = BUILTIN_MODES[mode_idx].1;
    let mut colorize = if monochrome {
        false
    } else {
        mode_default_colorize
    };
    let mut brightness: f32 = cli.brightness.clamp(-1.0, 1.0);

    let mut mode_label: String = if cli.chars.is_some() || cli.gradient.is_some() {
        match &render_mode {
            RenderMode::Charset(cs) => {
                let prefix = if cli.chars.is_some() { "custom" } else { "gradient" };
                format!("{} {}{}{}{}", prefix, cs.empty, cs.top, cs.bottom, cs.full)
            }
            RenderMode::FullColor => "full-color".to_string(),
        }
    } else {
        mode.to_string()
    };

    let frame_w = meta.frame_width;
    let frame_h = meta.frame_height;
    let frame_size = frame_w * frame_h * 3;

    let frames_path = prepare::frames_path(&cache_entry);
    let frames_file = std::fs::File::open(&frames_path).unwrap_or_else(|e| {
        eprintln!("Frames not found ({e}). Re-run: bad-apple --force <URL>");
        std::process::exit(1);
    });
    let mut frames_reader = BufReader::new(frames_file);
    let mut frame_buf = vec![0u8; frame_size];

    // Audio setup
    let (_stream, stream_handle) = match OutputStream::try_default() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Audio device error: {e:?}");
            std::process::exit(1);
        }
    };
    let sink = match Sink::try_new(&stream_handle) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Audio sink error: {e:?}");
            std::process::exit(1);
        }
    };
    let source = Decoder::new(std::io::Cursor::new(audio_bytes)).unwrap();

    let mut terminal = ratatui::init();

    sink.append(source);
    sink.play();

    // base_offset + elapsed since base_time = current playback position in seconds.
    // Resets on seek; base_time resets on unpause.
    let mut base_offset: f64 = 0.0;
    let mut base_time = Instant::now();
    let mut paused = false;
    let mut show_controls = true;
    let mut current_frame = 0usize;
    let mut done = false;
    let mut fps_frames: u32 = 0;
    let mut fps_timer = Instant::now();
    let mut fps: f32 = 0.0;

    while !done {
        let elapsed = if paused {
            base_offset
        } else {
            base_offset + base_time.elapsed().as_secs_f64()
        };
        let target_frame = (elapsed * FPS) as usize;

        if !paused && target_frame > current_frame {
            // Seek directly to target frame instead of reading through skipped frames.
            if target_frame > current_frame + 1 {
                let byte_offset = target_frame as u64 * frame_size as u64;
                let _ = frames_reader.seek(SeekFrom::Start(byte_offset));
                current_frame = target_frame;
            }
            match frames_reader.read_exact(&mut frame_buf) {
                Ok(()) => {
                    current_frame += 1;
                    fps_frames += 1;
                    let fps_elapsed = fps_timer.elapsed().as_secs_f32();
                    if fps_elapsed >= 1.0 {
                        fps = fps_frames as f32 / fps_elapsed;
                        fps_frames = 0;
                        fps_timer = Instant::now();
                    }
                    terminal
                        .draw(|frame| {
                            let area = frame.area();
                            let buf = frame.buffer_mut();
                            render_to_buf(
                                buf, area, &frame_buf, frame_w, frame_h, &render_mode,
                                colorize, brightness, &meta, &mode_label, show_controls, paused, fps,
                            );
                        })
                        .unwrap();
                }
                Err(_) => {
                    done = true;
                }
            }
        }

        if let Ok(true) = event::poll(Duration::from_millis(10)) {
            if let Ok(Event::Key(key)) = event::read() {
                // Ignore key-repeat and key-release; only act on the initial press.
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('c')
                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        break;
                    }
                    KeyCode::Char('c') => {
                        colorize = !colorize;
                    }
                    KeyCode::Char('h') => {
                        show_controls = !show_controls;
                    }
                    KeyCode::Char('k') | KeyCode::Char(' ') => {
                        if paused {
                            paused = false;
                            base_time = Instant::now();
                            sink.play();
                        } else {
                            base_offset += base_time.elapsed().as_secs_f64();
                            paused = true;
                            sink.pause();
                        }
                        // Re-render immediately so the controls bar reflects the new state.
                        terminal
                            .draw(|frame| {
                                let area = frame.area();
                                let buf = frame.buffer_mut();
                                render_to_buf(
                                    buf, area, &frame_buf, frame_w, frame_h, &render_mode,
                                    colorize, brightness, &meta, &mode_label, show_controls, paused, fps,
                                );
                            })
                            .unwrap();
                    }
                    KeyCode::Char('j') | KeyCode::Char('l') => {
                        let current_pos = if paused {
                            base_offset
                        } else {
                            base_offset + base_time.elapsed().as_secs_f64()
                        };
                        let delta =
                            if key.code == KeyCode::Char('j') { -10.0 } else { 10.0 };
                        let new_pos = (current_pos + delta).max(0.0);
                        let new_frame = (new_pos * FPS) as usize;
                        let byte_offset = new_frame as u64 * frame_size as u64;
                        let _ = frames_reader.seek(SeekFrom::Start(byte_offset));
                        current_frame = new_frame;
                        base_offset = new_pos;
                        base_time = Instant::now();
                        let _ = sink.try_seek(Duration::from_secs_f64(new_pos));
                        if paused {
                            sink.pause();
                        }
                        // Read and render one frame so the display updates immediately.
                        if frames_reader.read_exact(&mut frame_buf).is_ok() {
                            current_frame += 1;
                            terminal
                                .draw(|frame| {
                                    let area = frame.area();
                                    let buf = frame.buffer_mut();
                                    render_to_buf(
                                        buf, area, &frame_buf, frame_w, frame_h, &render_mode,
                                        colorize, brightness, &meta, &mode_label, show_controls, paused, fps,
                                    );
                                })
                                .unwrap();
                        }
                    }
                    KeyCode::Up => {
                        brightness = (brightness + 0.1).min(1.0);
                    }
                    KeyCode::Down => {
                        brightness = (brightness - 0.1).max(-1.0);
                    }
                    KeyCode::Left => {
                        mode_idx = (mode_idx + BUILTIN_MODES.len() - 1) % BUILTIN_MODES.len();
                        render_mode = render_mode_for_builtin(BUILTIN_MODES[mode_idx].0);
                        mode_label = BUILTIN_MODES[mode_idx].0.to_string();
                        if !monochrome {
                            colorize = BUILTIN_MODES[mode_idx].1;
                        }
                    }
                    KeyCode::Right => {
                        mode_idx = (mode_idx + 1) % BUILTIN_MODES.len();
                        render_mode = render_mode_for_builtin(BUILTIN_MODES[mode_idx].0);
                        mode_label = BUILTIN_MODES[mode_idx].0.to_string();
                        if !monochrome {
                            colorize = BUILTIN_MODES[mode_idx].1;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    ratatui::restore();
}

fn render_mode_for_builtin(mode: &str) -> RenderMode {
    match mode {
        "full-color" => RenderMode::FullColor,
        "ascii-color" => RenderMode::Charset(CharSet::ascii()),
        "ascii" => RenderMode::Charset(CharSet::ascii()),
        "braille" => RenderMode::Charset(CharSet::braille()),
        "korean" => RenderMode::Charset(CharSet::korean()),
        _ => RenderMode::Charset(CharSet::shading()),
    }
}

fn render_to_buf(
    buf: &mut Buffer,
    area: Rect,
    frame: &[u8],
    frame_w: usize,
    frame_h: usize,
    render_mode: &RenderMode,
    colorize: bool,
    brightness: f32,
    meta: &prepare::VideoMeta,
    mode_label: &str,
    show_controls: bool,
    paused: bool,
    fps: f32,
) {
    let term_w = area.width;
    let term_h = area.height;

    let vid_aspect = meta.orig_width as f32 / meta.orig_height as f32;
    let col_step = match render_mode {
        RenderMode::Charset(cs) => cs.col_width,
        RenderMode::FullColor => 1,
    };

    let layout = compute_layout(term_w, term_h, vid_aspect, col_step);
    let VideoLayout { draw_w, draw_h, offset_x, offset_y, bar_row } = layout;

    // Ratatui clears the buffer to default cells before each draw, so letterboxed
    // areas are automatically blank — we only need to fill the video region.

    match render_mode {
        RenderMode::FullColor => {
            for y in 0..draw_h {
                for x in 0..draw_w {
                    let cell_x = offset_x + x;
                    let cell_y = offset_y + y;
                    let src_x = ((x as f32 / draw_w as f32) * frame_w as f32) as usize;
                    let src_y_top =
                        ((y as f32 / draw_h as f32) * frame_h as f32) as usize;
                    let src_y_bot =
                        (((y as f32 + 0.5) / draw_h as f32) * frame_h as f32) as usize;
                    let fg = adjust_rgb(get_rgb(frame, src_x, src_y_top, frame_w, frame_h), brightness);
                    let bg = adjust_rgb(get_rgb(frame, src_x, src_y_bot, frame_w, frame_h), brightness);
                    buf.set_string(
                        cell_x,
                        cell_y,
                        "▀",
                        Style::new()
                            .fg(Color::Rgb(fg.0, fg.1, fg.2))
                            .bg(Color::Rgb(bg.0, bg.1, bg.2)),
                    );
                }
            }
        }
        RenderMode::Charset(cs) => {
            let mut ch_buf = [0u8; 4];
            for y in 0..draw_h {
                for x in 0..draw_w {
                    let cell_x = offset_x + x * cs.col_width;
                    let cell_y = offset_y + y;
                    let src_x = ((x as f32 / draw_w as f32) * frame_w as f32) as usize;
                    let src_y_top =
                        ((y as f32 / draw_h as f32) * frame_h as f32) as usize;
                    let src_y_bot =
                        (((y as f32 + 0.5) / draw_h as f32) * frame_h as f32) as usize;
                    let top = get_pixel(frame, src_x, src_y_top, frame_w, frame_h, brightness);
                    let bottom = get_pixel(frame, src_x, src_y_bot, frame_w, frame_h, brightness);
                    let ch = match (top, bottom) {
                        (false, false) => cs.empty,
                        (true, false) => cs.top,
                        (false, true) => cs.bottom,
                        (true, true) => cs.full,
                    };
                    let style = if colorize {
                        let rgb = adjust_rgb(get_rgb(frame, src_x, src_y_top, frame_w, frame_h), brightness);
                        Style::new().fg(Color::Rgb(rgb.0, rgb.1, rgb.2))
                    } else {
                        Style::default()
                    };
                    let symbol = ch.encode_utf8(&mut ch_buf);
                    buf.set_string(cell_x, cell_y, &*symbol, style);
                }
            }
        }
    }

    // Controls bar: full-width reverse-video strip on bar_row (the last terminal row).
    let bar_text = if show_controls {
        let pause_label = if paused { "resume" } else { "pause" };
        format!(
            "{:.1}fps  {}x{}  [{}]  [←/→] mode  [↑/↓] brt:{:+.1}  [k] {pause_label}  [j] -10s  [l] +10s  [c] color  [q] quit  [h] hide",
            fps, term_w, term_h, mode_label, brightness,
        )
    } else {
        format!(" [{}]  [h] show controls", mode_label)
    };
    let tw = term_w as usize;
    // Pad/truncate by char count (not bytes) so multi-byte chars like ← → don't misalign.
    let char_count = bar_text.chars().count();
    let padded = if char_count < tw {
        format!("{:<width$}", bar_text, width = tw)
    } else {
        bar_text.chars().take(tw).collect::<String>()
    };
    buf.set_string(
        area.x,
        area.y + bar_row,
        &padded,
        Style::new().add_modifier(Modifier::REVERSED),
    );
}

fn get_rgb(
    frame: &[u8],
    src_x: usize,
    src_y: usize,
    frame_w: usize,
    frame_h: usize,
) -> (u8, u8, u8) {
    let src_x = src_x.clamp(0, frame_w - 1);
    let src_y = src_y.clamp(0, frame_h - 1);
    let base = (src_y * frame_w + src_x) * 3;
    (frame[base], frame[base + 1], frame[base + 2])
}

/// Shift each channel by `brightness` (-1.0 = black, +1.0 = white).
fn adjust_rgb((r, g, b): (u8, u8, u8), brightness: f32) -> (u8, u8, u8) {
    let adj = brightness * 255.0;
    let clamp = |v: f32| v.clamp(0.0, 255.0) as u8;
    (clamp(r as f32 + adj), clamp(g as f32 + adj), clamp(b as f32 + adj))
}

/// Returns true if the pixel is "lit", after applying brightness to the luminance threshold.
/// brightness > 0 makes more pixels appear lit; brightness < 0 requires brighter pixels.
fn get_pixel(frame: &[u8], src_x: usize, src_y: usize, frame_w: usize, frame_h: usize, brightness: f32) -> bool {
    let (r, g, b) = get_rgb(frame, src_x, src_y, frame_w, frame_h);
    let luma = r as f32 * 299.0 + g as f32 * 587.0 + b as f32 * 114.0;
    luma + brightness * 128_000.0 > 128_000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{layout::Position, style::Modifier};

    fn make_meta(orig_w: usize, orig_h: usize) -> prepare::VideoMeta {
        prepare::VideoMeta {
            orig_width: orig_w,
            orig_height: orig_h,
            frame_width: orig_w,
            frame_height: orig_h,
        }
    }

    // ── compute_layout invariants ──────────────────────────────────────────────

    /// The video must never spill into the bar row.
    #[test]
    fn layout_video_stays_above_bar() {
        let cases = [
            (80u16, 24u16, 16.0_f32 / 9.0, 1u16),
            (160, 50, 16.0 / 9.0, 1),
            (120, 40, 4.0 / 3.0, 1),
            (80, 24, 4.0 / 3.0, 2), // wide chars
            (200, 60, 21.0 / 9.0, 1), // ultra-wide
            (80, 24, 1.0 / 3.0, 1),  // very tall video
        ];
        for (tw, th, aspect, col_step) in cases {
            let l = compute_layout(tw, th, aspect, col_step);
            let last_video_row = l.offset_y + l.draw_h - 1;
            assert!(
                last_video_row < l.bar_row,
                "tw={tw} th={th}: video last row {last_video_row} >= bar row {}",
                l.bar_row
            );
        }
    }

    /// The bar row is always the last terminal row.
    #[test]
    fn layout_bar_row_is_last_row() {
        for th in [1u16, 2, 10, 24, 50, 100] {
            let l = compute_layout(80, th, 16.0 / 9.0, 1);
            assert_eq!(l.bar_row, th - 1, "th={th}: bar_row should be {}", th - 1);
        }
    }

    /// The video should fill the full terminal width when the aspect ratio allows.
    #[test]
    fn layout_maximizes_width_for_wide_video() {
        // 16:9 at 160 wide: draw_h will be within video_h so width won't be clipped.
        let l = compute_layout(160, 50, 16.0 / 9.0, 1);
        assert_eq!(l.draw_w, 160, "wide video should fill full width");
        assert_eq!(l.offset_x, 0);
    }

    /// The video should be horizontally centered (left/right padding equal ±1).
    #[test]
    fn layout_horizontal_centering() {
        // 4:3 in 160-wide terminal forces height clamping → narrower video → side padding.
        let l = compute_layout(160, 50, 4.0 / 3.0, 1);
        let used_cols = l.draw_w * 1;
        let left = l.offset_x;
        let right = 160u16.saturating_sub(left + used_cols);
        assert!(
            (left as i32 - right as i32).abs() <= 1,
            "not centered: left={left} right={right}"
        );
    }

    /// The video should be vertically centered within the video area (±1 for rounding).
    #[test]
    fn layout_vertical_centering() {
        let l = compute_layout(160, 50, 16.0 / 9.0, 1);
        let top = l.offset_y;
        // Space below video but above the bar row.
        let bottom = l.bar_row.saturating_sub(l.offset_y + l.draw_h);
        assert!(
            (top as i32 - bottom as i32).abs() <= 1,
            "not vertically centered: top={top} bottom={bottom} bar={}",
            l.bar_row
        );
    }

    /// Video must stay within terminal width.
    #[test]
    fn layout_video_within_width() {
        for col_step in [1u16, 2] {
            let l = compute_layout(80, 24, 16.0 / 9.0, col_step);
            assert!(
                l.offset_x + l.draw_w * col_step <= 80,
                "video overflows terminal width (col_step={col_step})"
            );
        }
    }

    // ── render_to_buf invariants ───────────────────────────────────────────────

    fn solid_frame(w: usize, h: usize) -> Vec<u8> {
        // Non-black/non-default so FullColor video cells are identifiable.
        vec![128u8; w * h * 3]
    }

    fn render_test_buf(tw: u16, th: u16, meta: &prepare::VideoMeta, show_controls: bool) -> Buffer {
        let area = Rect::new(0, 0, tw, th);
        let mut buf = Buffer::empty(area);
        let frame = solid_frame(meta.frame_width, meta.frame_height);
        render_to_buf(
            &mut buf, area, &frame,
            meta.frame_width, meta.frame_height,
            &RenderMode::FullColor, false, meta,
            "full-color", show_controls, false, 30.0,
        );
        buf
    }

    /// REVERSED must appear on EVERY column of the last row and NOWHERE ELSE.
    /// If the bar is at the wrong row (e.g. row 0 instead of row th-1), this fails.
    #[test]
    fn bar_is_exclusively_on_last_row() {
        // Includes (543, 130) = the 4K terminal size (height-constrained path).
        for (tw, th) in [(80u16, 24u16), (160, 50), (120, 40), (40, 10), (543, 130)] {
            let meta = make_meta(16, 9);
            let buf = render_test_buf(tw, th, &meta, true);

            for row in 0..th {
                for col in 0..tw {
                    let has_rev = buf
                        .cell(Position::new(col, row))
                        .unwrap()
                        .modifier
                        .contains(Modifier::REVERSED);
                    if row == th - 1 {
                        assert!(
                            has_rev,
                            "tw={tw} th={th}: bar MISSING at row={row} col={col}"
                        );
                    } else {
                        assert!(
                            !has_rev,
                            "tw={tw} th={th}: REVERSED spuriously at row={row} col={col} — bar should only be on row {}",
                            th - 1
                        );
                    }
                }
            }
        }
    }

    /// Same contract when controls are hidden.
    #[test]
    fn bar_is_on_last_row_when_hidden() {
        let (tw, th) = (80u16, 24u16);
        let meta = make_meta(16, 9);
        let buf = render_test_buf(tw, th, &meta, false);

        for row in 0..th {
            for col in 0..tw {
                let has_rev = buf
                    .cell(Position::new(col, row))
                    .unwrap()
                    .modifier
                    .contains(Modifier::REVERSED);
                if row == th - 1 {
                    assert!(has_rev, "hidden bar MISSING at row={row} col={col}");
                } else {
                    assert!(!has_rev, "REVERSED spuriously at row={row} col={col}");
                }
            }
        }
    }

    /// Video cells and letterbox placement for a width-constrained terminal
    /// (video fills full width, top/bottom letterbox present).
    #[test]
    fn video_placement_width_constrained() {
        // 16:9 at 160×50: draw_w=160, draw_h=45, offset_y=2 → rows 0-1 and 47-48 letterbox.
        let (tw, th) = (160u16, 50u16);
        let meta = make_meta(16, 9);
        let buf = render_test_buf(tw, th, &meta, true);
        let layout = compute_layout(tw, th, 16.0 / 9.0, 1);

        assert!(layout.offset_y > 0, "expected top letterbox for this terminal");
        assert!(layout.offset_y + layout.draw_h < layout.bar_row, "expected bottom letterbox");

        check_video_and_letterbox(&buf, tw, th, &layout);
    }

    /// Video cells and letterbox placement for a height-constrained terminal
    /// (video fills full height of video area, left/right letterbox present).
    /// This is the 4K case: 543×130.
    #[test]
    fn video_placement_height_constrained_4k() {
        let (tw, th) = (543u16, 130u16);
        let meta = make_meta(16, 9);
        let buf = render_test_buf(tw, th, &meta, true);
        let layout = compute_layout(tw, th, 16.0 / 9.0, 1);

        // Height-constrained: video fills all video_h rows, so no top/bottom letterbox.
        assert_eq!(layout.draw_h, th - 1, "video should fill full video height");
        assert_eq!(layout.offset_y, 0, "no top letterbox expected");
        // But there should be left/right letterbox.
        assert!(layout.offset_x > 0, "expected left/right letterbox for 16:9 in wide terminal");

        check_video_and_letterbox(&buf, tw, th, &layout);
    }

    /// Shared checker: scans every cell and asserts correct placement.
    fn check_video_and_letterbox(buf: &Buffer, tw: u16, th: u16, layout: &VideoLayout) {
        let video_top = layout.offset_y;
        let video_bot = layout.offset_y + layout.draw_h; // exclusive
        let vid_left = layout.offset_x;
        let vid_right = layout.offset_x + layout.draw_w; // exclusive

        for row in 0..th {
            for col in 0..tw {
                let cell = buf.cell(Position::new(col, row)).unwrap();
                let is_video = cell.symbol() == "▀"
                    && cell.fg != ratatui::style::Color::Reset;
                let is_bar = cell.modifier.contains(Modifier::REVERSED);

                if row == layout.bar_row {
                    assert!(is_bar, "row={row} col={col}: expected bar cell");
                } else if row >= video_top && row < video_bot
                    && col >= vid_left && col < vid_right
                {
                    assert!(
                        is_video,
                        "row={row} col={col}: expected ▀ inside video bounds"
                    );
                } else {
                    // Letterbox or out-of-video-bounds: must be empty.
                    assert!(
                        !is_video && !is_bar,
                        "row={row} col={col}: expected empty cell in letterbox region"
                    );
                }
            }
        }
    }
}
