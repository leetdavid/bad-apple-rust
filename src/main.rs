use clap::Parser;
use crossterm::{cursor, execute, terminal};
use rodio::{Decoder, OutputStream, Sink};
use std::io::{stdout, Write};
use std::time::Instant;

const FRAME_SIZE: usize = 2400; // 160 * 120 / 8
const FPS: f64 = 30.0;

#[derive(Parser)]
#[command(version, about = "Bad Apple CLI player")]
struct Cli {
    #[arg(short, long, default_value = "block")]
    mode: String,
}

fn main() {
    let cli = Cli::parse();
    let is_block = cli.mode == "block";

    let audio_bytes = include_bytes!("../assets/audio.mp3");
    let frames_bytes = include_bytes!("../assets/frames.bin");

    let num_frames = frames_bytes.len() / FRAME_SIZE;

    // Audio setup
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let sink = Sink::try_new(&stream_handle).unwrap();
    let cursor_audio = std::io::Cursor::new(audio_bytes);
    let source = Decoder::new(cursor_audio).unwrap();

    // Terminal setup
    let mut stdout = stdout();
    terminal::enable_raw_mode().unwrap();
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide).unwrap();

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

            let frame_data = &frames_bytes[current_frame * FRAME_SIZE..(current_frame + 1) * FRAME_SIZE];
            
            let (term_w, term_h) = terminal::size().unwrap_or((80, 24));
            render_frame(&mut stdout, frame_data, term_w, term_h, is_block);
        }

        if crossterm::event::poll(std::time::Duration::from_millis(10)).unwrap() {
            if let crossterm::event::Event::Key(key) = crossterm::event::read().unwrap() {
                if key.code == crossterm::event::KeyCode::Char('q') || key.code == crossterm::event::KeyCode::Esc || (key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) && key.code == crossterm::event::KeyCode::Char('c')) {
                    break;
                }
            }
        }
    }

    // Restore terminal
    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen).unwrap();
    terminal::disable_raw_mode().unwrap();
}

fn render_frame(stdout: &mut std::io::Stdout, frame: &[u8], term_w: u16, term_h: u16, is_block: bool) {
    let vid_aspect = 160.0 / 120.0;
    let mut draw_w = term_w;
    let mut draw_h = (term_w as f32 / vid_aspect / 2.0).round() as u16;
    if draw_h > term_h {
        draw_h = term_h;
        draw_w = (term_h as f32 * 2.0 * vid_aspect).round() as u16;
    }
    
    let draw_w = draw_w.max(1);
    let draw_h = draw_h.max(1);

    let offset_x = (term_w.saturating_sub(draw_w)) / 2;
    let offset_y = (term_h.saturating_sub(draw_h)) / 2;

    let mut buf = String::with_capacity((term_w as usize * term_h as usize) + term_h as usize * 10);
    
    buf.push_str("\x1b[H"); // Move cursor to home

    for y in 0..term_h {
        for x in 0..term_w {
            if x < offset_x || x >= offset_x + draw_w || y < offset_y || y >= offset_y + draw_h {
                buf.push(' ');
            } else {
                let rel_x = x - offset_x;
                let rel_y = y - offset_y;
                
                let src_x = ((rel_x as f32 / draw_w as f32) * 160.0) as usize;
                
                if is_block {
                    let src_y_top = ((rel_y as f32 / draw_h as f32) * 120.0) as usize;
                    let src_y_bottom = (((rel_y as f32 + 0.5) / draw_h as f32) * 120.0) as usize;
                    
                    let top = get_pixel(frame, src_x, src_y_top);
                    let bottom = get_pixel(frame, src_x, src_y_bottom);
                    
                    match (top, bottom) {
                        (false, false) => buf.push(' '),
                        (true, false) => buf.push('▀'),
                        (false, true) => buf.push('▄'),
                        (true, true) => buf.push('█'),
                    }
                } else {
                    let src_y = ((rel_y as f32 / draw_h as f32) * 120.0) as usize;
                    let p = get_pixel(frame, src_x, src_y);
                    buf.push(if p { '@' } else { ' ' });
                }
            }
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
