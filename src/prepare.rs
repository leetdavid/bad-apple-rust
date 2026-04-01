use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DEFAULT_URL: &str = "https://www.youtube.com/watch?v=FtutLA63Cp8";
const FRAME_WIDTH: usize = 160;

pub struct VideoMeta {
    pub frame_width: usize,
    pub frame_height: usize,
}

impl VideoMeta {
    pub fn frame_size(&self) -> usize {
        self.frame_width * self.frame_height * 3 // RGB24: 3 bytes per pixel
    }
}

pub fn cache_root() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("bad-apple")
}

fn cache_key(source: &str) -> String {
    let mut h = DefaultHasher::new();
    source.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn entry_dir(source: &str) -> PathBuf {
    cache_root().join(cache_key(source))
}

/// Returns the most recently used cache entry, if any.
pub fn last_cache_entry() -> Option<PathBuf> {
    let key = fs::read_to_string(cache_root().join("last")).ok()?;
    let path = cache_root().join(key.trim());
    if path.exists() { Some(path) } else { None }
}

pub fn load_meta(entry: &Path) -> Result<VideoMeta, String> {
    let s = fs::read_to_string(entry.join("meta.txt"))
        .map_err(|e| format!("Failed to read meta.txt: {e}"))?;
    let parts: Vec<&str> = s.trim().split_whitespace().collect();
    if parts.len() < 3 || parts[2] != "rgb" {
        return Err(
            "Cached assets use an old format. Re-run: bad-apple --force <URL>".to_string(),
        );
    }
    Ok(VideoMeta {
        frame_width: parts[0].parse().map_err(|e| format!("Invalid width: {e}"))?,
        frame_height: parts[1].parse().map_err(|e| format!("Invalid height: {e}"))?,
    })
}

pub fn run(source: Option<String>, force: bool) -> Result<PathBuf, String> {
    let source = source.unwrap_or_else(|| DEFAULT_URL.to_string());
    let entry = entry_dir(&source);

    if !force && entry.join("frames.bin").exists() && entry.join("audio.mp3").exists() {
        println!("Using cached assets (use --force to re-process).");
        set_last(&source)?;
        return Ok(entry);
    }

    check_tool("ffmpeg", "https://ffmpeg.org/download.html")?;

    fs::create_dir_all(&entry).map_err(|e| format!("Failed to create cache directory: {e}"))?;

    let is_local = Path::new(&source).is_file();

    let video_path: PathBuf;
    let tmp_video = std::env::temp_dir().join("bad-apple-src.mp4");

    if is_local {
        video_path = PathBuf::from(&source);
    } else {
        check_tool("yt-dlp", "https://github.com/yt-dlp/yt-dlp")?;
        let _ = fs::remove_file(&tmp_video);
        println!("Downloading...");
        download_video(&source, &tmp_video)?;
        video_path = tmp_video.clone();
    }

    let (orig_w, orig_h) = get_video_dimensions(&video_path)?;
    let frame_height = ((FRAME_WIDTH as f64 * orig_h as f64 / orig_w as f64).round() as usize).max(1);

    fs::write(entry.join("meta.txt"), format!("{FRAME_WIDTH} {frame_height} rgb"))
        .map_err(|e| format!("Failed to write meta.txt: {e}"))?;

    println!("Extracting audio...");
    extract_audio(&video_path, &entry.join("audio.mp3"))?;

    println!("Processing frames ({FRAME_WIDTH}x{frame_height}, 1-bit, 30 fps)...");
    process_frames(&video_path, &entry.join("frames.bin"), FRAME_WIDTH, frame_height)?;

    if !is_local {
        let _ = fs::remove_file(&tmp_video);
    }

    set_last(&source)?;
    println!("Done. Cached at {}", entry.display());
    Ok(entry)
}

fn set_last(source: &str) -> Result<(), String> {
    let root = cache_root();
    fs::create_dir_all(&root).map_err(|e| format!("Failed to create cache root: {e}"))?;
    fs::write(root.join("last"), cache_key(source))
        .map_err(|e| format!("Failed to write last pointer: {e}"))
}

fn check_tool(name: &str, url: &str) -> Result<(), String> {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| format!("`{name}` not found in PATH. Install it from {url}"))?;
    Ok(())
}

fn get_video_dimensions(input: &Path) -> Result<(usize, usize), String> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height",
            "-of", "default=noprint_wrappers=1:nokey=1",
            input.to_str().unwrap(),
        ])
        .output()
        .map_err(|e| format!("Failed to run ffprobe: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let nums: Vec<usize> = stdout
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();

    if nums.len() < 2 {
        return Err(format!("Could not read video dimensions from ffprobe output: {stdout}"));
    }
    Ok((nums[0], nums[1]))
}

fn download_video(url: &str, output: &Path) -> Result<(), String> {
    let status = Command::new("yt-dlp")
        .args(["-o", output.to_str().unwrap(), "--merge-output-format", "mp4", url])
        .status()
        .map_err(|e| format!("Failed to run yt-dlp: {e}"))?;

    if !status.success() {
        return Err("yt-dlp failed to download the video.".to_string());
    }
    Ok(())
}

fn extract_audio(input: &Path, output: &Path) -> Result<(), String> {
    let status = Command::new("ffmpeg")
        .args(["-y", "-i", input.to_str().unwrap(), "-q:a", "0", "-map", "a",
               output.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !status.success() {
        return Err("ffmpeg failed to extract audio.".to_string());
    }
    Ok(())
}

fn process_frames(input: &Path, output: &Path, width: usize, height: usize) -> Result<(), String> {
    let mut child = Command::new("ffmpeg")
        .args([
            "-i", input.to_str().unwrap(),
            "-vf", &format!("scale={width}:{height}"),
            "-r", "30",
            "-f", "rawvideo",
            "-pix_fmt", "rgb24",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn ffmpeg: {e}"))?;

    let mut stdout = child.stdout.take().unwrap();
    let mut out_file =
        fs::File::create(output).map_err(|e| format!("Failed to create frames.bin: {e}"))?;

    let frame_bytes = width * height * 3; // RGB24: 3 bytes per pixel
    let mut raw_frame = vec![0u8; frame_bytes];
    let mut frame_count = 0usize;

    loop {
        match stdout.read_exact(&mut raw_frame) {
            Ok(()) => {
                out_file
                    .write_all(&raw_frame)
                    .map_err(|e| format!("Write error: {e}"))?;
                frame_count += 1;
                if frame_count % 300 == 0 {
                    print!("\r  {frame_count} frames...");
                    let _ = io::stdout().flush();
                }
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Error reading frames from ffmpeg: {e}")),
        }
    }

    let _ = child.wait();
    println!("\r  {frame_count} frames processed.    ");
    Ok(())
}
