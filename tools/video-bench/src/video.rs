use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use breader::ImageView;
use serde_json::Value;

/// Frame size after ffmpeg's automatic rotation.
fn probe(path: &Path) -> Result<(usize, usize), String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_streams",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|e| format!("ffprobe: {e}"))?;
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("ffprobe json: {e}"))?;
    let s = v["streams"]
        .get(0)
        .ok_or_else(|| format!("{}: no video stream", path.display()))?;
    let w = s["width"].as_u64().unwrap_or(0) as usize;
    let h = s["height"].as_u64().unwrap_or(0) as usize;
    let rot = s["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|d| d["rotation"].as_f64())
        .unwrap_or(0.0);
    if w == 0 || h == 0 {
        return Err(format!("{}: unknown frame size", path.display()));
    }
    let quarter = (rot / 90.0).round() as i64;
    Ok(if quarter.rem_euclid(2) == 1 {
        (h, w)
    } else {
        (w, h)
    })
}

/// Pipes the decoded gray frames of `path` to `f`; returns the frame count.
pub fn for_each_frame(
    path: &Path,
    max_frames: Option<u64>,
    mut f: impl FnMut(u64, ImageView<'_>),
) -> Result<u64, String> {
    let (w, h) = probe(path)?;
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-fps_mode",
            "passthrough",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut stdout = child.stdout.take().ok_or("ffmpeg: no stdout")?;
    let mut buf = vec![0u8; w * h];
    let mut n = 0u64;
    while max_frames.is_none_or(|m| n < m) {
        if stdout.read_exact(&mut buf).is_err() {
            break;
        }
        let view = ImageView::from_packed(&buf, w, h).map_err(|e| e.to_string())?;
        f(n, view);
        n += 1;
    }
    drop(stdout);
    let _ = child.kill();
    let _ = child.wait();
    if n == 0 {
        return Err(format!("{}: no frames decoded", path.display()));
    }
    Ok(n)
}
