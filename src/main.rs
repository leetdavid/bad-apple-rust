use clap::Parser;
use crossterm::{cursor, execute, terminal};
use rodio::{Decoder, OutputStream, Sink};
use std::io::{Write, stdout};
use std::time::Instant;
use unicode_width::UnicodeWidthChar;

const FRAME_SIZE: usize = 2400; // 160 * 120 / 8
const FPS: f64 = 30.0;

#[derive(Parser)]
#[command(version, about = "Bad Apple CLI player")]
struct Cli {
    /// Render mode [possible values: block, ascii, korean, classic, shading]
    #[arg(short, long, default_value = "block")]
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
        // Detect width from the widest character in the set
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

    fn block() -> Self {
        CharSet {
            empty: ' ',
            top: '▀',
            bottom: '▄',
            full: '█',
            col_width: 1,
        }
    }

    fn ascii() -> Self {
        CharSet {
            empty: '.',
            top: '^',
            bottom: 'v',
            full: '@',
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
        // Sample at 0%, 33%, 67%, 100% of the palette
        let indices = [
            0,
            (last * 1 / 3).min(last),
            (last * 2 / 3).min(last),
            last,
        ];
        let sampled = [chars[indices[0]], chars[indices[1]], chars[indices[2]], chars[indices[3]]];
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

    fn classic() -> Self {
        // " .:-=+*#%@" sampled at 0%, 33%, 67%, 100%
        CharSet { empty: ' ', top: '-', bottom: '*', full: '@', col_width: 1 }
    }

    fn shading() -> Self {
        CharSet { empty: ' ', top: '░', bottom: '▒', full: '█', col_width: 1 }
    }

    fn korean() -> Self {
        CharSet {
            empty: '시', // U+3000 '　' ideographic space (full-width, 2 columns)
            top: '뽁',
            bottom: '늙',
            full: '뾃',
            col_width: 2,
        }
    }
}

fn main() {
    let cli = Cli::parse();

    let mode = if cli.block {
        "block"
    } else if cli.ascii {
        "ascii"
    } else if cli.korean {
        "korean"
    } else if cli.classic {
        "classic"
    } else if cli.shading {
        "shading"
    } else {
        cli.mode.as_str()
    };

    let char_set = if let Some(s) = &cli.chars {
        match CharSet::from_str(s) {
            Ok(cs) => cs,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else if let Some(s) = &cli.gradient {
        match CharSet::from_gradient(s) {
            Ok(cs) => cs,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    } else if mode == "block" {
        CharSet::block()
    } else if mode == "korean" {
        CharSet::korean()
    } else if mode == "classic" {
        CharSet::classic()
    } else if mode == "shading" {
        CharSet::shading()
    } else {
        CharSet::ascii()
    };

    let audio_bytes = include_bytes!("../assets/audio.mp3");
    let frames_bytes = include_bytes!("../assets/frames.bin");

    let num_frames = frames_bytes.len() / FRAME_SIZE;

    // Audio setup
    let audio_res = OutputStream::try_default();
    let (_stream, stream_handle) = match audio_res {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Audio device error: {:?}", e);
            std::process::exit(1);
        }
    };

    let sink = match Sink::try_new(&stream_handle) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Audio sink error: {:?}", e);
            std::process::exit(1);
        }
    };
    let cursor_audio = std::io::Cursor::new(audio_bytes);
    let source = Decoder::new(cursor_audio).unwrap();

    // Terminal setup
    let mut stdout = stdout();
    let has_tty = terminal::enable_raw_mode().is_ok();
    let _ = execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide);

    sink.append(source);
    sink.play();

    let start_time = Instant::now();
    let mut current_frame = 0;

    while current_frame < num_frames {
        let elapsed = start_time.elapsed().as_secs_f64();
        let target_frame = (elapsed * FPS) as usize;

        if target_frame > current_frame {
            current_frame = target_frame;
            if current_frame >= num_frames {
                break;
            }

            let frame_data =
                &frames_bytes[current_frame * FRAME_SIZE..(current_frame + 1) * FRAME_SIZE];

            let (term_w, term_h) = terminal::size().unwrap_or((80, 24));
            render_frame(&mut stdout, frame_data, term_w, term_h, &char_set);
        }

        if let Ok(true) = crossterm::event::poll(std::time::Duration::from_millis(10)) {
            if let Ok(crossterm::event::Event::Key(key)) = crossterm::event::read() {
                if key.code == crossterm::event::KeyCode::Char('q')
                    || key.code == crossterm::event::KeyCode::Esc
                    || (key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL)
                        && key.code == crossterm::event::KeyCode::Char('c'))
                {
                    break;
                }
            }
        }
    }

    // Restore terminal
    let _ = execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen);
    if has_tty {
        let _ = terminal::disable_raw_mode();
    }
}

fn render_frame(
    stdout: &mut std::io::Stdout,
    frame: &[u8],
    term_w: u16,
    term_h: u16,
    cs: &CharSet,
) {
    let vid_aspect = 160.0_f32 / 120.0;
    let col_step = cs.col_width;

    // Wide chars (col_width == 2) are approximately square, so no /2.0 aspect correction.
    // Narrow chars are ~2x taller than wide, so divide by 2.0 to get rows from columns.
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
    // Align offset to col_step so wide chars never straddle the boundary
    let offset_x = ((term_w.saturating_sub(draw_w_cols)) / 2 / col_step) * col_step;
    let offset_y = (term_h.saturating_sub(draw_h)) / 2;

    let mut buf = String::with_capacity(term_w as usize * term_h as usize * 4);
    buf.push_str("\x1b[H"); // move cursor to home

    for y in 0..term_h {
        let mut x = 0u16;
        while x < term_w {
            let in_bounds = x >= offset_x
                && x < offset_x + draw_w_cols
                && y >= offset_y
                && y < offset_y + draw_h;

            if !in_bounds {
                for _ in 0..col_step {
                    buf.push(' ');
                }
            } else {
                let rel_x = (x - offset_x) / col_step;
                let rel_y = y - offset_y;

                let src_x = ((rel_x as f32 / draw_w as f32) * 160.0) as usize;
                let src_y_top = ((rel_y as f32 / draw_h as f32) * 120.0) as usize;
                let src_y_bottom = (((rel_y as f32 + 0.5) / draw_h as f32) * 120.0) as usize;

                let top = get_pixel(frame, src_x, src_y_top);
                let bottom = get_pixel(frame, src_x, src_y_bottom);

                match (top, bottom) {
                    (false, false) => buf.push(cs.empty),
                    (true, false) => buf.push(cs.top),
                    (false, true) => buf.push(cs.bottom),
                    (true, true) => buf.push(cs.full),
                }
            }
            x += col_step;
        }
        if y < term_h - 1 {
            buf.push_str("\r\n");
        }
    }

    stdout.write_all(buf.as_bytes()).unwrap();
    stdout.flush().unwrap();
}

fn get_pixel(frame: &[u8], src_x: usize, src_y: usize) -> bool {
    let src_x = src_x.clamp(0, 159);
    let src_y = src_y.clamp(0, 119);
    let pixel_idx = src_y * 160 + src_x;
    let byte_idx = pixel_idx / 8;
    let bit_idx = 7 - (pixel_idx % 8);
    (frame[byte_idx] & (1 << bit_idx)) != 0
}
