use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DEFAULT_URL: &str = "https://www.youtube.com/watch?v=FtutLA63Cp8";

pub struct VideoMeta {
    pub orig_width: usize,
    pub orig_height: usize,
    pub frame_width: usize,
    pub frame_height: usize,
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

pub fn last_cache_entry() -> Option<PathBuf> {
    let key = fs::read_to_string(cache_root().join("last")).ok()?;
    let path = cache_root().join(key.trim());
    if path.exists() { Some(path) } else { None }
}

pub fn frames_path(entry: &Path) -> PathBuf {
    entry.join("frames.bin")
}

pub fn load_meta(entry: &Path) -> Result<VideoMeta, String> {
    let s = fs::read_to_string(entry.join("meta.txt"))
        .map_err(|e| format!("Failed to read meta.txt: {e}"))?;
    let parts: Vec<&str> = s.trim().split_whitespace().collect();
    if parts.len() < 4 {
        return Err("Invalid meta.txt. Re-run: bad-apple --force <URL>".to_string());
    }
    Ok(VideoMeta {
        orig_width:   parts[0].parse().map_err(|e| format!("Invalid orig_width: {e}"))?,
        orig_height:  parts[1].parse().map_err(|e| format!("Invalid orig_height: {e}"))?,
        frame_width:  parts[2].parse().map_err(|e| format!("Invalid frame_width: {e}"))?,
        frame_height: parts[3].parse().map_err(|e| format!("Invalid frame_height: {e}"))?,
    })
}

pub fn run(
    source: Option<String>,
    force: bool,
    frame_width: usize,
    cookies_from_browser: Option<String>,
    cookies: Option<String>,
    extractor_args: Option<String>,
) -> Result<PathBuf, String> {
    let source = source.unwrap_or_else(|| DEFAULT_URL.to_string());
    let entry = entry_dir(&source);

    // Cache hit: frames.bin and audio.mp3 exist at the right resolution
    let frames_ok = !force
        && entry.join("frames.bin").exists()
        && load_meta(&entry).map(|m| m.frame_width == frame_width).unwrap_or(false);
    let audio_ok = !force && entry.join("audio.mp3").exists();

    if frames_ok && audio_ok {
        println!("Using cached assets (use --force to re-process).");
        set_last(&source)?;
        return Ok(entry);
    }

    check_tool("ffmpeg", "https://ffmpeg.org/download.html")?;
    check_tool("ffprobe", "https://ffmpeg.org/download.html")?;

    fs::create_dir_all(&entry).map_err(|e| format!("Failed to create cache directory: {e}"))?;

    let is_local = Path::new(&source).is_file();
    let tmp_video = std::env::temp_dir().join("bad-apple-src.mp4");
    let cached_video = entry.join("video.mp4");

    // Use cached video.mp4 if available (avoids re-downloading for resolution changes)
    let src_path: PathBuf = if is_local {
        PathBuf::from(&source)
    } else if !force && cached_video.exists() {
        cached_video.clone()
    } else {
        check_tool("yt-dlp", "https://github.com/yt-dlp/yt-dlp")?;
        let _ = fs::remove_file(&tmp_video);
        println!("Downloading...");
        download_video(&source, &tmp_video, cookies_from_browser.as_deref(), cookies.as_deref(), extractor_args.as_deref())?;
        tmp_video.clone()
    };

    let (orig_w, orig_h) = get_video_dimensions(&src_path)?;
    let frame_h = ((frame_width as f64 * orig_h as f64 / orig_w as f64).round() as usize).max(1);

    fs::write(entry.join("meta.txt"), format!("{orig_w} {orig_h} {frame_width} {frame_h}"))
        .map_err(|e| format!("Failed to write meta.txt: {e}"))?;

    if !audio_ok {
        println!("Extracting audio...");
        extract_audio(&src_path, &entry.join("audio.mp3"))?;
    }

    if !frames_ok {
        println!("Extracting frames at {frame_width}×{frame_h}...");
        extract_frames(&src_path, &entry.join("frames.bin"), frame_width, frame_h)?;
    }

    // Keep video.mp4 in cache for future re-processing at a different resolution
    if !cached_video.exists() || force {
        if src_path == tmp_video {
            fs::rename(&tmp_video, &cached_video)
                .or_else(|_| fs::copy(&tmp_video, &cached_video).map(|_| ()).map_err(|e| e.to_string()))
                .map_err(|e| format!("Failed to store video: {e}"))?;
        } else if is_local {
            fs::copy(&src_path, &cached_video)
                .map_err(|e| format!("Failed to copy video: {e}"))?;
        }
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
        return Err(format!("Could not read video dimensions: {stdout}"));
    }
    Ok((nums[0], nums[1]))
}

fn download_video(
    url: &str,
    output: &Path,
    cookies_from_browser: Option<&str>,
    cookies: Option<&str>,
    extractor_args: Option<&str>,
) -> Result<(), String> {
    let mut cmd = Command::new("yt-dlp");
    // Cap at 480p — terminal rendering doesn't benefit from higher resolutions
    cmd.args([
        "-f", "bestvideo[height<=480]+bestaudio/best[height<=480]/best",
        "-o", output.to_str().unwrap(),
        "--merge-output-format", "mp4",
    ]);
    if let Some(browser) = cookies_from_browser {
        cmd.args(["--cookies-from-browser", browser]);
    }
    if let Some(file) = cookies {
        cmd.args(["--cookies", file]);
    }
    if let Some(args) = extractor_args {
        cmd.args(["--extractor-args", args]);
    }
    cmd.arg(url);
    let status = cmd.status().map_err(|e| format!("Failed to run yt-dlp: {e}"))?;
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

fn extract_frames(input: &Path, output: &Path, frame_w: usize, frame_h: usize) -> Result<(), String> {
    let status = Command::new("ffmpeg")
        .args([
            "-y",
            "-i", input.to_str().unwrap(),
            "-vf", &format!("scale={frame_w}:{frame_h},fps=30"),
            "-f", "rawvideo",
            "-pix_fmt", "rgb24",
            output.to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed to run ffmpeg: {e}"))?;

    if !status.success() {
        return Err("ffmpeg failed to extract frames.".to_string());
    }
    Ok(())
}
