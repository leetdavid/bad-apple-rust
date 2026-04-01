mod prepare;

use clap::Parser;
use crossterm::{cursor, execute, terminal};
use rodio::{Decoder, OutputStream, Sink};
use std::io::{BufReader, Read, Seek, SeekFrom, Write, stdout};
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthChar;

const FPS: f64 = 30.0;

#[derive(Parser)]
#[command(version, about = "Bad Apple CLI player")]
struct Cli {
    /// URL or local file path of a video to download, preprocess, and play.
    /// Accepts any URL supported by yt-dlp (YouTube, direct links, etc.).
    /// Omit to replay the last prepared video.
    url: Option<String>,

    /// Force re-download and re-processing even if cached assets exist.
    #[arg(long)]
    force: bool,

    /// Render mode [possible values: classic, block, ascii, shading, korean, full-color]
    #[arg(short, long, default_value = "full-color")]
    mode: String,

    /// Use block mode (shorthand for --mode block)
    #[arg(long, conflicts_with_all = ["ascii", "korean", "classic", "shading"])]
    block: bool,

    /// Use ASCII mode (shorthand for --mode ascii)
    #[arg(long, conflicts_with_all = ["block", "korean", "classic", "shading"])]
    ascii: bool,

    /// Use Korean wide-character mode (shorthand for --mode korean)
    #[arg(long, conflicts_with_all = ["block", "ascii", "classic", "shading"])]
    korean: bool,

    /// Use classic ASCII gradient mode: " .:-=+*#%@" (shorthand for --mode classic)
    #[arg(long, conflicts_with_all = ["block", "ascii", "korean", "shading"])]
    classic: bool,

    /// Use Unicode block shading mode: " ░▒▓█" (shorthand for --mode shading)
    #[arg(long, conflicts_with_all = ["block", "ascii", "korean", "classic"])]
    shading: bool,

    /// Use full-color mode: ▀ with ANSI 24-bit fg/bg per pixel (shorthand for --mode full-color)
    #[arg(long = "full-color", conflicts_with_all = ["block", "ascii", "korean", "classic", "shading", "monochrome"])]
    full_color: bool,

    /// Disable ANSI colors; render in terminal default foreground/background only.
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

    /// Frame width to use when preprocessing (default: 160).
    /// Smaller values are faster to prepare and use less disk space.
    /// Use --force to re-process at a different width.
    #[arg(long, default_value = "160")]
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
        Ok(CharSet { empty: chars[0], top: chars[1], bottom: chars[2], full: chars[3], col_width })
    }

    fn block() -> Self {
        CharSet { empty: ' ', top: '▀', bottom: '▄', full: '█', col_width: 1 }
    }

    fn ascii() -> Self {
        CharSet { empty: '.', top: '^', bottom: 'v', full: '@', col_width: 1 }
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
        let sampled = [chars[indices[0]], chars[indices[1]], chars[indices[2]], chars[indices[3]]];
        let col_width = sampled
            .iter()
            .map(|c| c.width().unwrap_or(1) as u16)
            .max()
            .unwrap_or(1)
            .max(1);
        Ok(CharSet { empty: sampled[0], top: sampled[1], bottom: sampled[2], full: sampled[3], col_width })
    }

    fn classic() -> Self {
        CharSet { empty: ' ', top: '-', bottom: '*', full: '@', col_width: 1 }
    }

    fn shading() -> Self {
        CharSet { empty: ' ', top: '░', bottom: '▒', full: '█', col_width: 1 }
    }

    fn korean() -> Self {
        CharSet { empty: '시', top: '뽁', bottom: '늙', full: '뾃', col_width: 2 }
    }
}

enum RenderMode {
    Charset(CharSet),
    FullColor,
}

fn main() {
    let cli = Cli::parse();

    let frame_width = cli.width as usize;

    let cache_entry = if cli.url.is_some() || cli.force {
        match prepare::run(cli.url, cli.force, frame_width, cli.cookies_from_browser, cli.cookies, cli.extractor_args) {
            Ok(dir) => dir,
            Err(e) => { eprintln!("Error: {e}"); std::process::exit(1); }
        }
    } else {
        match prepare::last_cache_entry() {
            Some(dir) => dir,
            None => { eprintln!("No cached video found. Run: bad-apple <URL>"); std::process::exit(1); }
        }
    };

    let meta = match prepare::load_meta(&cache_entry) {
        Ok(m) => m,
        Err(e) => { eprintln!("Error loading video metadata: {e}"); std::process::exit(1); }
    };

    let audio_bytes = std::fs::read(cache_entry.join("audio.mp3")).unwrap_or_else(|_| {
        eprintln!("Audio not found. Re-run: bad-apple --force <URL>");
        std::process::exit(1);
    });

    let mut colorize = !cli.monochrome;

    let mode = if cli.full_color { "full-color" }
        else if cli.block   { "block"   }
        else if cli.ascii   { "ascii"   }
        else if cli.korean  { "korean"  }
        else if cli.classic { "classic" }
        else if cli.shading { "shading" }
        else                { cli.mode.as_str() };

    const BUILTIN_MODES: &[&str] = &["classic", "block", "ascii", "shading", "korean", "full-color"];
    let mut mode_idx = BUILTIN_MODES.iter().position(|&m| m == mode).unwrap_or(0);

    let mut render_mode = if let Some(s) = &cli.chars {
        match CharSet::from_str(s) {
            Ok(cs) => RenderMode::Charset(cs),
            Err(e) => { eprintln!("Error: {e}"); std::process::exit(1); }
        }
    } else if let Some(s) = &cli.gradient {
        match CharSet::from_gradient(s) {
            Ok(cs) => RenderMode::Charset(cs),
            Err(e) => { eprintln!("Error: {e}"); std::process::exit(1); }
        }
    } else {
        render_mode_for_builtin(mode)
    };

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
    let (term_w, term_h) = terminal::size().unwrap_or((80, 24));
    let mut frame_buf = vec![0u8; frame_size];

    // Audio setup
    let (_stream, stream_handle) = match OutputStream::try_default() {
        Ok(s) => s,
        Err(e) => { eprintln!("Audio device error: {e:?}"); std::process::exit(1); }
    };
    let sink = match Sink::try_new(&stream_handle) {
        Ok(s) => s,
        Err(e) => { eprintln!("Audio sink error: {e:?}"); std::process::exit(1); }
    };
    let source = Decoder::new(std::io::Cursor::new(audio_bytes)).unwrap();

    // Terminal setup
    let mut stdout = stdout();
    let has_tty = terminal::enable_raw_mode().is_ok();
    let _ = execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide);

    sink.append(source);
    sink.play();

    // base_offset + elapsed since base_time = current playback position in seconds.
    // Resets on seek; base_time resets on unpause.
    let mut base_offset: f64 = 0.0;
    let mut base_time = Instant::now();
    let mut paused = false;
    let mut current_frame = 0usize;
    let mut done = false;

    while !done {
        let elapsed = if paused { base_offset } else { base_offset + base_time.elapsed().as_secs_f64() };
        let target_frame = (elapsed * FPS) as usize;

        if !paused && target_frame > current_frame {
            // Skip ahead in the file to the target frame, then read it
            while current_frame < target_frame {
                match frames_reader.read_exact(&mut frame_buf) {
                    Ok(()) => current_frame += 1,
                    Err(_) => { done = true; break; }
                }
            }
            if !done {
                let (tw, th) = terminal::size().unwrap_or((term_w, term_h));
                render_frame(&mut stdout, &frame_buf, frame_w, frame_h, tw, th, &render_mode, colorize, &meta, &mode_label);
            }
        }

        if let Ok(true) = crossterm::event::poll(Duration::from_millis(10)) {
            if let Ok(crossterm::event::Event::Key(key)) = crossterm::event::read() {
                match key.code {
                    crossterm::event::KeyCode::Char('q') | crossterm::event::KeyCode::Esc => break,
                    crossterm::event::KeyCode::Char('c')
                        if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => break,
                    crossterm::event::KeyCode::Char('c') => { colorize = !colorize; }
                    crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Char(' ') => {
                        if paused {
                            paused = false;
                            base_time = Instant::now();
                            sink.play();
                        } else {
                            base_offset += base_time.elapsed().as_secs_f64();
                            paused = true;
                            sink.pause();
                        }
                    }
                    crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Char('l') => {
                        let current_pos = if paused { base_offset } else { base_offset + base_time.elapsed().as_secs_f64() };
                        let delta = if key.code == crossterm::event::KeyCode::Char('j') { -10.0 } else { 10.0 };
                        let new_pos = (current_pos + delta).max(0.0);
                        let new_frame = (new_pos * FPS) as usize;
                        let byte_offset = new_frame as u64 * frame_size as u64;
                        let _ = frames_reader.seek(SeekFrom::Start(byte_offset));
                        current_frame = new_frame;
                        base_offset = new_pos;
                        base_time = Instant::now();
                        let _ = sink.try_seek(Duration::from_secs_f64(new_pos));
                        if paused { sink.pause(); }
                        // Read and render one frame so the display updates immediately
                        if frames_reader.read_exact(&mut frame_buf).is_ok() {
                            current_frame += 1;
                            let (tw, th) = terminal::size().unwrap_or((term_w, term_h));
                            render_frame(&mut stdout, &frame_buf, frame_w, frame_h, tw, th, &render_mode, colorize, &meta, &mode_label);
                        }
                    }
                    crossterm::event::KeyCode::Left => {
                        mode_idx = (mode_idx + BUILTIN_MODES.len() - 1) % BUILTIN_MODES.len();
                        render_mode = render_mode_for_builtin(BUILTIN_MODES[mode_idx]);
                        mode_label = BUILTIN_MODES[mode_idx].to_string();
                    }
                    crossterm::event::KeyCode::Right => {
                        mode_idx = (mode_idx + 1) % BUILTIN_MODES.len();
                        render_mode = render_mode_for_builtin(BUILTIN_MODES[mode_idx]);
                        mode_label = BUILTIN_MODES[mode_idx].to_string();
                    }
                    _ => {}
                }
            }
        }
    }

    let _ = execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen);
    if has_tty { let _ = terminal::disable_raw_mode(); }
}


fn render_mode_for_builtin(mode: &str) -> RenderMode {
    match mode {
        "full-color" => RenderMode::FullColor,
        "block"   => RenderMode::Charset(CharSet::block()),
        "ascii"   => RenderMode::Charset(CharSet::ascii()),
        "shading" => RenderMode::Charset(CharSet::shading()),
        "korean"  => RenderMode::Charset(CharSet::korean()),
        _         => RenderMode::Charset(CharSet::classic()),
    }
}

fn render_frame(
    stdout: &mut std::io::Stdout,
    frame: &[u8],
    frame_w: usize,
    frame_h: usize,
    term_w: u16,
    term_h: u16,
    render_mode: &RenderMode,
    colorize: bool,
    meta: &prepare::VideoMeta,
    mode_label: &str,
) {
    let vid_aspect = meta.orig_width as f32 / meta.orig_height as f32;
    let col_step = match render_mode {
        RenderMode::Charset(cs) => cs.col_width,
        RenderMode::FullColor => 1,
    };
    let aspect_correction = if col_step == 1 { 2.0_f32 } else { 1.0_f32 };

    let max_chars = term_w / col_step;
    let mut draw_w = max_chars;
    let mut draw_h = (draw_w as f32 / vid_aspect / aspect_correction).round() as u16;
    if draw_h > term_h {
        draw_h = term_h;
        draw_w = (term_h as f32 * aspect_correction * vid_aspect).round() as u16;
    }
    let draw_w = draw_w.max(1);
    let draw_h = draw_h.max(1);

    let draw_w_cols = draw_w * col_step;
    let offset_x = ((term_w.saturating_sub(draw_w_cols)) / 2 / col_step) * col_step;
    let offset_y = (term_h.saturating_sub(draw_h)) / 2;

    let mut buf = String::with_capacity(term_w as usize * term_h as usize * 8);
    buf.push_str("\x1b[H");

    match render_mode {
        RenderMode::Charset(cs) => {
            use std::fmt::Write as _;
            let sentinel = (255u8, 255u8, 255u8);
            let mut last_fg = sentinel;

            for y in 0..term_h {
                let mut x = 0u16;
                while x < term_w {
                    let in_bounds = x >= offset_x && x < offset_x + draw_w_cols
                        && y >= offset_y && y < offset_y + draw_h;

                    if !in_bounds {
                        if colorize && last_fg != sentinel {
                            buf.push_str("\x1b[0m");
                            last_fg = sentinel;
                        }
                        for _ in 0..col_step { buf.push(' '); }
                    } else {
                        let rel_x = (x - offset_x) / col_step;
                        let rel_y = y - offset_y;
                        let src_x = ((rel_x as f32 / draw_w as f32) * frame_w as f32) as usize;
                        let src_y_top = ((rel_y as f32 / draw_h as f32) * frame_h as f32) as usize;
                        let src_y_bot = (((rel_y as f32 + 0.5) / draw_h as f32) * frame_h as f32) as usize;
                        let top = get_pixel(frame, src_x, src_y_top, frame_w, frame_h);
                        let bottom = get_pixel(frame, src_x, src_y_bot, frame_w, frame_h);
                        if colorize {
                            let fg = get_rgb(frame, src_x, src_y_top, frame_w, frame_h);
                            if fg != last_fg {
                                write!(buf, "\x1b[38;2;{};{};{}m", fg.0, fg.1, fg.2).unwrap();
                                last_fg = fg;
                            }
                        }
                        match (top, bottom) {
                            (false, false) => buf.push(cs.empty),
                            (true,  false) => buf.push(cs.top),
                            (false, true)  => buf.push(cs.bottom),
                            (true,  true)  => buf.push(cs.full),
                        }
                    }
                    x += col_step;
                }
                if colorize && last_fg != sentinel {
                    buf.push_str("\x1b[0m");
                    last_fg = sentinel;
                }
                if y < term_h - 1 { buf.push_str("\r\n"); }
            }
        }
        RenderMode::FullColor => {
            use std::fmt::Write as _;
            let sentinel = (255u8, 255u8, 255u8);
            let mut last_fg = sentinel;
            let mut last_bg = sentinel;

            for y in 0..term_h {
                for x in 0..term_w {
                    let in_bounds = x >= offset_x && x < offset_x + draw_w_cols
                        && y >= offset_y && y < offset_y + draw_h;

                    if !in_bounds {
                        if last_fg != sentinel || last_bg != sentinel {
                            buf.push_str("\x1b[0m");
                            last_fg = sentinel;
                            last_bg = sentinel;
                        }
                        buf.push(' ');
                    } else {
                        let rel_x = x - offset_x;
                        let rel_y = y - offset_y;
                        let src_x = ((rel_x as f32 / draw_w as f32) * frame_w as f32) as usize;
                        let src_y_top = ((rel_y as f32 / draw_h as f32) * frame_h as f32) as usize;
                        let src_y_bot = (((rel_y as f32 + 0.5) / draw_h as f32) * frame_h as f32) as usize;
                        let fg = get_rgb(frame, src_x, src_y_top, frame_w, frame_h);
                        let bg = get_rgb(frame, src_x, src_y_bot, frame_w, frame_h);
                        if fg != last_fg {
                            write!(buf, "\x1b[38;2;{};{};{}m", fg.0, fg.1, fg.2).unwrap();
                            last_fg = fg;
                        }
                        if bg != last_bg {
                            write!(buf, "\x1b[48;2;{};{};{}m", bg.0, bg.1, bg.2).unwrap();
                            last_bg = bg;
                        }
                        buf.push('▀');
                    }
                }
                if last_fg != sentinel || last_bg != sentinel {
                    buf.push_str("\x1b[0m");
                    last_fg = sentinel;
                    last_bg = sentinel;
                }
                if y < term_h - 1 { buf.push_str("\r\n"); }
            }
        }
    }

    // Overlay mode label at bottom-left in reverse video
    {
        use std::fmt::Write as _;
        write!(buf, "\x1b[{};1H\x1b[7m[{}]\x1b[0m", term_h, mode_label).unwrap();
    }

    stdout.write_all(buf.as_bytes()).unwrap();
    stdout.flush().unwrap();
}

fn get_rgb(frame: &[u8], src_x: usize, src_y: usize, frame_w: usize, frame_h: usize) -> (u8, u8, u8) {
    let src_x = src_x.clamp(0, frame_w - 1);
    let src_y = src_y.clamp(0, frame_h - 1);
    let base = (src_y * frame_w + src_x) * 3;
    (frame[base], frame[base + 1], frame[base + 2])
}

fn get_pixel(frame: &[u8], src_x: usize, src_y: usize, frame_w: usize, frame_h: usize) -> bool {
    let (r, g, b) = get_rgb(frame, src_x, src_y, frame_w, frame_h);
    (r as u32 * 299 + g as u32 * 587 + b as u32 * 114) > 128_000
}
