//! Sprachnotizen und Untertitel: Audio- und Videodateien lokal mit Whisper (whisper.cpp) in Text umwandeln.
//! Audio → `.txt` neben der Datei, Video → `.srt` (der Player zeigt die Untertitel und blendet sie ein/aus).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

/// Ton als 16-kHz-Mono-WAV (für Whisper): mit ffmpeg, sonst selbst dekodiert (MP3, FLAC, WAV, OGG, M4A)
fn to_wav(src: &Path) -> Result<PathBuf, String> {
    let out = std::env::temp_dir().join(format!("foldex-stt-{}-{}.wav", std::process::id(), now_ms()));
    if let Some(ff) = crate::convert::ffmpeg() {
        let mut c = Command::new(ff);
        c.args(["-y", "-v", "error", "-i"]).arg(src).args(["-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"]).arg(&out).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        hide(&mut c);
        let o = c.output().map_err(|e| format!("ffmpeg: {e}"))?;
        if o.status.success() && out.is_file() {
            return Ok(out);
        }
        let err = String::from_utf8_lossy(&o.stderr);
        if crate::convert::class_of(src) == crate::convert::Class::Video {
            return Err(format!("Ton nicht lesbar: {}", err.lines().last().unwrap_or("")));
        }
    } else if crate::convert::class_of(src) == crate::convert::Class::Video {
        return Err("Für Untertitel aus Videos wird ffmpeg gebraucht (Einstellungen → Werkzeuge)".into());
    }
    // ohne ffmpeg: rodio/symphonia
    use rodio::Source;
    let f = std::fs::File::open(src).map_err(|e| e.to_string())?;
    let dec = rodio::Decoder::new(BufReader::new(f)).map_err(|e| format!("Audio nicht lesbar: {e}"))?;
    let (ch, rate) = (dec.channels().max(1) as usize, dec.sample_rate());
    let all: Vec<f32> = dec.convert_samples::<f32>().collect();
    let mono: Vec<f32> = all.chunks(ch).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    let n = (mono.len() as u64 * 16_000 / rate.max(1) as u64) as usize;
    let k = rate as f64 / 16_000.0;
    let mut b = Vec::with_capacity(44 + n * 2);
    let data_len = (n * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 1u16.to_le_bytes().to_vec(), 16_000u32.to_le_bytes().to_vec(), 32_000u32.to_le_bytes().to_vec(), 2u16.to_le_bytes().to_vec(), 16u16.to_le_bytes().to_vec()] {
        b.extend_from_slice(&v);
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for i in 0..n {
        let v = mono.get((i as f64 * k) as usize).copied().unwrap_or(0.0);
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(&out, b).map_err(|e| e.to_string())?;
    Ok(out)
}

fn now_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis())
}

fn hide(c: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let _ = c;
}

/// Zieldatei: `name.txt` bzw. `name.srt` neben der Quelle (vorhandene werden nicht überschrieben)
pub fn target(src: &Path, srt: bool) -> PathBuf {
    let dir = src.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Transkript".into());
    let ext = if srt { "srt" } else { "txt" };
    let p = dir.join(format!("{stem}.{ext}"));
    if !p.exists() {
        return p;
    }
    (2..).map(|i| dir.join(format!("{stem} ({i}).{ext}"))).find(|p| !p.exists()).unwrap_or(p)
}

/// Datei umwandeln; `prog` in Promille (0–1000). Ergebnis: geschriebene Datei.
pub fn run(src: &Path, srt: bool, prog: &AtomicU64, cancel: &AtomicBool) -> Result<PathBuf, String> {
    let model = crate::localllm::stt_ready().ok_or("Whisper ist noch nicht eingerichtet: Einstellungen → Lokale Modelle → Spracheingabe")?;
    let exe = crate::localllm::whisper_exe().ok_or("whisper.cpp fehlt")?;
    prog.store(20, Relaxed);
    let wav = to_wav(src)?;
    prog.store(60, Relaxed);
    let dest = target(src, srt);
    let base = std::env::temp_dir().join(format!("foldex-stt-out-{}", now_ms()));
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    let mut c = Command::new(&exe);
    crate::localllm::whisper_env(&mut c, &exe);
    c.current_dir(exe.parent().unwrap_or(Path::new(".")))
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(&wav)
        .args(["-l", "auto", "-pp", "-t", &threads.to_string()])
        .arg(if srt { "-osrt" } else { "-otxt" })
        .arg("-of")
        .arg(&base)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide(&mut c);
    let mut child = c.spawn().map_err(|e| format!("whisper.cpp starten: {e}"))?;
    let mut last = String::new();
    if let Some(err) = child.stderr.take() {
        for l in BufReader::new(err).lines().map_while(Result::ok) {
            if cancel.load(Relaxed) {
                let _ = child.kill();
                break;
            }
            // „whisper_print_progress_callback: progress =  40%“
            if let Some(p) = l.split("progress =").nth(1).and_then(|r| r.trim().trim_end_matches('%').trim().parse::<u64>().ok()) {
                prog.store(60 + p.min(100) * 930 / 100, Relaxed);
            } else if !l.trim().is_empty() {
                last = l;
            }
        }
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&wav);
    if cancel.load(Relaxed) {
        return Err("Abgebrochen".into());
    }
    let produced = base.with_extension(if srt { "srt" } else { "txt" });
    if !ok || !produced.is_file() {
        return Err(format!("Spracherkennung fehlgeschlagen: {last}"));
    }
    let mut text = std::fs::read_to_string(&produced).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&produced);
    if !srt {
        // Zeilen zusammenfassen, Platzhalter wie [Musik] fallen weg
        let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !(l.starts_with('[') && l.ends_with(']'))).collect();
        text = lines.join("\n") + "\n";
    }
    std::fs::write(&dest, text).map_err(|e| e.to_string())?;
    prog.store(1000, Relaxed);
    Ok(dest)
}

// ───────────────────────── Untertitel lesen ─────────────────────────

/// ein Untertitel: von, bis (Sekunden), Text
pub type Cue = (f64, f64, String);

fn ts(s: &str) -> Option<f64> {
    // 00:01:02,345
    let s = s.trim().replace('.', ",");
    let (hms, ms) = s.split_once(',').unwrap_or((&s, "0"));
    let p: Vec<f64> = hms.split(':').filter_map(|x| x.trim().parse().ok()).collect();
    let base = match p.as_slice() {
        [h, m, s] => h * 3600.0 + m * 60.0 + s,
        [m, s] => m * 60.0 + s,
        _ => return None,
    };
    Some(base + ms.trim().parse::<f64>().unwrap_or(0.0) / 10f64.powi(ms.trim().len() as i32))
}

/// SRT (und einfaches WebVTT) einlesen
pub fn parse_srt(t: &str) -> Vec<Cue> {
    let t = t.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut v = Vec::new();
    for block in t.split("\n\n") {
        let mut lines = block.lines().skip_while(|l| !l.contains("-->"));
        let Some(time) = lines.next() else { continue };
        let Some((a, b)) = time.split_once("-->") else { continue };
        let b = b.split_whitespace().next().unwrap_or("");
        if let (Some(a), Some(b)) = (ts(a), ts(b)) {
            let text: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
            if !text.is_empty() {
                v.push((a, b, text.join("\n")));
            }
        }
    }
    v
}

/// Untertitel-Datei zum Video (gleicher Name, .srt oder .vtt)
pub fn subs_for(video: &Path) -> Option<PathBuf> {
    ["srt", "vtt"].iter().map(|e| video.with_extension(e)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt() {
        let s = "1\n00:00:01,000 --> 00:00:02,500\nHallo\n\n2\n00:00:03,000 --> 00:00:04,000\nZwei\nZeilen\n";
        let c = parse_srt(s);
        assert_eq!(c.len(), 2);
        assert!((c[0].1 - 2.5).abs() < 1e-9);
        assert_eq!(c[1].2, "Zwei\nZeilen");
    }
}
