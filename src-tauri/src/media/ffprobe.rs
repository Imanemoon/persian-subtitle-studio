//! اجرای FFprobe و استخراج مشخصات Subtitle Streamها از خروجی JSON.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::models::SubtitleStream;
use crate::scanner::naming::text_suggests_fa;

use super::codec::classify_codec;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

// ---------- ساختار خام JSON خروجی ffprobe ----------

#[derive(Debug, Deserialize)]
struct FfprobeOutput {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    index: Option<u32>,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    codec_type: Option<String>,
    #[serde(default)]
    disposition: Option<FfprobeDisposition>,
    #[serde(default)]
    tags: Option<FfprobeTags>,
}

#[derive(Debug, Deserialize)]
struct FfprobeDisposition {
    #[serde(default)]
    default: Option<i64>,
    #[serde(default)]
    forced: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct FfprobeTags {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    language_ietf: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

// ---------- اجرا و تبدیل ----------

/// خطای پروب — لغو از شکست واقعی FFprobe تفکیک می‌شود.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// کاربر عملیات را لغو کرد؛ فرایند kill و wait شده است
    Cancelled,
    Failed(String),
}

impl ProbeError {
    pub fn message(&self) -> String {
        match self {
            Self::Cancelled => "عملیات تحلیل توسط کاربر لغو شد".into(),
            Self::Failed(m) => m.clone(),
        }
    }
}

/// مشخصات همهٔ Subtitle Streamهای ویدیو را برمی‌گرداند.
///
/// `cancel` در طول انتظار برای FFprobe بررسی می‌شود؛ در صورت لغو، فرایند
/// kill و wait شده و Threadهای خواندن pipe هم Join می‌شوند. فایل ویدیو فقط
/// خوانده می‌شود (Read-only).
pub fn probe_subtitle_streams(
    ffprobe: &Path,
    video: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<SubtitleStream>, ProbeError> {
    let mut cmd = Command::new(ffprobe);
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        "s",
        "-show_entries",
        "stream=index,codec_name,codec_type:stream_disposition=default,forced:stream_tags=language,language_ietf,title",
        "-of",
        "json",
    ]);
    cmd.arg(video);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| ProbeError::Failed(format!("اجرای FFprobe: {e}")))?;

    let mut stdout_pipe = child.stdout.take().expect("stdout piped");
    let mut stderr_pipe = child.stderr.take().expect("stderr piped");
    let t_out = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = stdout_pipe.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = stderr_pipe.read_to_string(&mut s);
        s
    });

    let deadline = Instant::now() + PROBE_TIMEOUT;
    let status = loop {
        // لغو در طول انتظار: فرایند فرزند kill و wait می‌شود (جمع‌آوری کامل)
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            // Threadهای خواندن pipe: برای ffprobe واقعی لوله با مرگ فرایند
            // بسته می‌شود و این joinها بلافاصله برمی‌گردند؛ join مستقیم اینجا
            // انجام نمی‌شود چون اگر فرایندِ نوه‌ای هندل لوله را به ارث برده
            // باشد (فقط در فرایندهای جعلی تست رخ می‌دهد)، join تا خروج نوه
            // مسدود می‌شد. Threadها detached می‌شوند و به‌هرحال با بسته‌شدن
            // لوله تمام می‌شوند — هیچ Thread یا Process بی‌صاحبی نمی‌ماند.
            drop(t_out);
            drop(t_err);
            return Err(ProbeError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(ProbeError::Failed(format!("انتظار برای FFprobe: {e}")));
            }
        }
    };

    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();

    let Some(st) = status else {
        return Err(ProbeError::Failed(
            "FFprobe در زمان مشخص پاسخ نداد (Timeout)".into(),
        ));
    };
    if !st.success() {
        let detail = stderr
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        return Err(ProbeError::Failed(format!(
            "FFprobe با کد خروج {} شکست خورد{}",
            st.code().unwrap_or(-1),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        )));
    }

    parse_ffprobe_json(&stdout).map_err(ProbeError::Failed)
}

/// تبدیل JSON خام FFprobe به مدل برنامه — مستقل از فرایند، قابل تست.
pub fn parse_ffprobe_json(json: &str) -> Result<Vec<SubtitleStream>, String> {
    let parsed: FfprobeOutput =
        serde_json::from_str(json).map_err(|e| format!("پارس JSON خروجی FFprobe: {e}"))?;

    let mut out = Vec::new();
    for s in parsed.streams {
        // فقط زیرنویس‌ها (_ffprobe خودش select s می‌کند اما محافظت اضافه منطقی است)
        if s.codec_type.as_deref() != Some("subtitle") {
            continue;
        }
        let Some(index) = s.index else {
            continue;
        };
        let codec_name = s.codec_name.unwrap_or_else(|| "unknown".to_string());
        let (language, language_ietf, title) = s
            .tags
            .map(|t| (t.language, t.language_ietf, t.title))
            .unwrap_or((None, None, None));
        let (default, forced) = s
            .disposition
            .map(|d| (d.default.unwrap_or(0) != 0, d.forced.unwrap_or(0) != 0))
            .unwrap_or((false, false));

        let persian_candidate = [
            language.as_deref(),
            language_ietf.as_deref(),
            title.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(text_suggests_fa);

        out.push(SubtitleStream {
            stream_index: index,
            kind: classify_codec(&codec_name),
            codec_name,
            language,
            language_ietf,
            title,
            default_disposition: default,
            forced_disposition: forced,
            persian_candidate,
            persian_confirmed: false,
        });
    }
    out.sort_by_key(|s| s.stream_index);
    Ok(out)
}

/// ابزار تست: «ffprobe» جعلی به‌شکل batch — به‌اندازهٔ مشخص صبر می‌کند و
/// JSON می‌دهد. برای تست لغو بدون وابستگی به سرعت واقعی ffprobe.
#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};

    pub fn write_fake_probe(dir: &Path, tag: &str, sleep_secs: u32, json: &str) -> PathBuf {
        let path = dir.join(format!("fake-probe-{tag}.cmd"));
        // ping هر ثانیه یک بار تیک می‌خورد → n = sleep+1 ≈ sleep ثانیه
        std::fs::write(
            &path,
            format!(
                "@echo off\r\nping -n {} 127.0.0.1 > nul\r\necho {}\r\n",
                sleep_secs + 1,
                json
            ),
        )
        .unwrap();
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SubtitleKind;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;
    use std::time::Instant;
    use test_support::write_fake_probe;

    /// JSON معتبر با یک Track متنی فارسی — برای ffprobe جعلی
    const FAKE_JSON_ONE_FA: &str = r#"{"streams": [{"index": 2, "codec_name": "subrip", "codec_type": "subtitle", "tags": {"language": "fas", "title": "فارسی"}}]}"#;

    const SAMPLE: &str = r#"{
        "streams": [
            {
                "index": 0,
                "codec_name": "h264",
                "codec_type": "video",
                "disposition": {"default": 1}
            },
            {
                "index": 1,
                "codec_name": "aac",
                "codec_type": "audio"
            },
            {
                "index": 2,
                "codec_name": "subrip",
                "codec_type": "subtitle",
                "disposition": {"default": 1, "forced": 0},
                "tags": {"language": "fas", "title": "فارسی", "language_ietf": "fas"}
            },
            {
                "index": 5,
                "codec_name": "hdmv_pgs_subtitle",
                "codec_type": "subtitle",
                "disposition": {"default": 0, "forced": 1},
                "tags": {"language": "eng", "title": "English PGS"}
            },
            {
                "index": 7,
                "codec_name": "subrip",
                "codec_type": "subtitle",
                "tags": {"language": "eng"}
            },
            {
                "index": 9,
                "codec_name": "weird_codec",
                "codec_type": "subtitle",
                "tags": {"language": "fa", "title": "Persian"}
            }
        ]
    }"#;

    #[test]
    fn parses_streams_with_global_indexes() {
        let streams = parse_ffprobe_json(SAMPLE).unwrap();
        // video/audio فیلتر شده‌اند
        assert_eq!(streams.len(), 4);
        // Global index حفظ شده — نه شماره‌گذاری مجدد 0,1,2
        assert_eq!(
            streams.iter().map(|s| s.stream_index).collect::<Vec<_>>(),
            vec![2, 5, 7, 9]
        );
    }

    #[test]
    fn detects_persian_by_tag_and_title() {
        let streams = parse_ffprobe_json(SAMPLE).unwrap();

        let fa = &streams[0]; // index 2 — language fas + عنوان فارسی
        assert!(fa.persian_candidate);
        assert_eq!(fa.kind, SubtitleKind::Text);
        assert_eq!(fa.language.as_deref(), Some("fas"));
        assert_eq!(fa.title.as_deref(), Some("فارسی"));
        assert!(fa.default_disposition);
        assert!(!fa.forced_disposition);

        let pgs = &streams[1]; // index 5 — eng، تصویری
        assert!(!pgs.persian_candidate);
        assert_eq!(pgs.kind, SubtitleKind::Image);
        assert!(pgs.forced_disposition);

        let eng = &streams[2]; // index 7 — فقط language دارد، بدون title
        assert!(!eng.persian_candidate);
        assert_eq!(eng.kind, SubtitleKind::Text);
        assert_eq!(eng.language.as_deref(), Some("eng"));
        assert!(eng.title.is_none());

        let unknown = &streams[3]; // index 9 — کدک ناشناخته ولی کاندید فارسی
        assert!(unknown.persian_candidate);
        assert_eq!(unknown.kind, SubtitleKind::Unknown);
    }

    #[test]
    fn tolerates_empty_and_malformed_tag_structures() {
        let streams = parse_ffprobe_json(r#"{"streams": []}"#).unwrap();
        assert!(streams.is_empty());

        // stream بدون tags و بدون disposition
        let streams = parse_ffprobe_json(
            r#"{"streams": [{"index": 3, "codec_name": "subrip", "codec_type": "subtitle"}]}"#,
        )
        .unwrap();
        assert_eq!(streams.len(), 1);
        assert!(!streams[0].persian_candidate);
        assert!(!streams[0].default_disposition);

        // JSON کاملاً خراب → خطای واضح
        assert!(parse_ffprobe_json("not json").is_err());
    }

    #[test]
    fn persian_detection_variants() {
        for tag in [
            "fa",
            "fas",
            "per",
            "farsi",
            "persian",
            "فارسی",
            "Persian",
            "FARSI",
        ] {
            let json = format!(
                r#"{{"streams": [{{"index": 0, "codec_name": "subrip", "codec_type": "subtitle", "tags": {{"language": "{tag}"}}}}]}}"#
            );
            let streams = parse_ffprobe_json(&json).unwrap();
            assert!(streams[0].persian_candidate, "tag={tag}");
        }
        for tag in ["en", "eng", "ara", "ger", "chi", "SPA"] {
            let json = format!(
                r#"{{"streams": [{{"index": 0, "codec_name": "subrip", "codec_type": "subtitle", "tags": {{"language": "{tag}"}}}}]}}"#
            );
            let streams = parse_ffprobe_json(&json).unwrap();
            assert!(!streams[0].persian_candidate, "tag={tag}");
        }
    }

    /// ffprobe جعلی کوتاه — چرخهٔ عادی پروب با فرایند کنترل‌شده
    #[test]
    #[ignore]
    fn fake_probe_completes_and_parses() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-fakeprobe-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = write_fake_probe(&dir, "short", 1, FAKE_JSON_ONE_FA);
        let no_cancel = AtomicBool::new(false);

        let started = Instant::now();
        let streams =
            probe_subtitle_streams(&fake, Path::new("ignored.mkv"), &no_cancel).expect("پروب موفق");
        let elapsed = started.elapsed();

        assert_eq!(streams.len(), 1);
        assert_eq!(streams[0].stream_index, 2);
        assert_eq!(streams[0].kind, SubtitleKind::Text);
        assert!(streams[0].persian_candidate);
        // واقعاً به فرایند فرزند منتظر مانده (حداقل ~۱ ثانیه)
        assert!(elapsed >= Duration::from_millis(900), "{elapsed:?}");
        // و خیلی قبل از timeout برگشته
        assert!(elapsed < PROBE_TIMEOUT);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// لغو حین پروب: فرایند فرزند kill و جمع‌آوری می‌شود و خطا Cancelled است —
    /// نه Timeout و نه شکست FFprobe. پرچم تازه بعدی باید سالم کار کند.
    #[test]
    #[ignore]
    fn cancel_kills_probe_child_and_returns_cancelled() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-fakeprobe-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // sleeper ۳۰ ثانیه‌ای — اگر kill نشود، probe تا timeout ۲۰ ثانیه‌ای می‌ماند
        let fake = write_fake_probe(&dir, "long", 30, FAKE_JSON_ONE_FA);
        let cancel = Arc::new(AtomicBool::new(false));

        let t_fake = fake.clone();
        let t_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            probe_subtitle_streams(&t_fake, Path::new("ignored.mkv"), &t_cancel)
        });

        // فرصت به فرایند فرزند برای شروع
        std::thread::sleep(Duration::from_millis(700));
        let started_kill = Instant::now();
        cancel.store(true, Ordering::SeqCst);

        let result = worker
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
        assert!(
            started_kill.elapsed() < Duration::from_secs(15),
            "باید بلافاصله پس از لغو برگردد: {:?}",
            started_kill.elapsed()
        );
        assert_eq!(result, Err(ProbeError::Cancelled));

        // پرچم تازه → پروب بعدی سالم کار می‌کند
        let fresh = AtomicBool::new(false);
        let fake_short = write_fake_probe(&dir, "short-after-cancel", 1, FAKE_JSON_ONE_FA);
        let streams = probe_subtitle_streams(&fake_short, Path::new("ignored.mkv"), &fresh)
            .expect("پروب بعدی با پرچم تازه باید موفق شود");
        assert_eq!(streams.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
