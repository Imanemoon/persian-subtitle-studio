//! اعتبارسنجی real-world هاردساب با FFmpeg/FFprobe واقعی — گated با env var.
//!
//! اجرا:
//! `cargo test --lib hardsub::real_world -- --ignored --test-threads=1`
//!
//! ابزارها: `SUBSYNC_REAL_FFMPEG` / `SUBSYNC_REAL_FFPROBE` یا fallback به
//! resolve_tool_path (PATH / نصب پیش‌فرض). هیچ فایل رسانه‌ای داخل ریپو قرار
//! نمی‌گیرد — ویدیوی مصنوعی با lavfi ساخته و در temp پاک می‌شود.
//!
//! تست اختیاری `real_archive_video` با `SUBSYNC_REAL_HARDSUB_VIDEO` روی یک
//! ویدیوی واقعی از آرشیو کاربر اجرا می‌شود (منبع فقط خوانده می‌شود).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::hardsub::{burn, encoder, plan};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const SRT_FA: &str = "1\n00:00:00,200 --> 00:00:01,000\nسلام دنیا\n\n2\n00:00:01,100 --> 00:00:02,900\nخط دوم فارسی برای هاردساب\n";

fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "subsync-hardsub-real-{tag}-{}-{nanos}",
        std::process::id()
    ))
}

fn resolve_tool_from_env_or_path(id: &str) -> PathBuf {
    let env_name = format!("SUBSYNC_REAL_{}", id.to_uppercase());
    if let Ok(path) = std::env::var(&env_name) {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "{env_name} باید فایل موجود باشد: {}",
            path.display()
        );
        return path;
    }
    crate::tools::resolve_tool_path(id, &Default::default()).unwrap_or_else(|| {
        panic!(
            "{id} پیدا نشد — SUBSYNC_REAL_{} را تنظیم کنید یا نصب کنید",
            id.to_uppercase()
        )
    })
}

/// ساخت ویدیوی مصنوعی با lavfi — mpeg4 + aac (و در صورت درخواست Track متنی فارسی).
fn build_synthetic_video(ffmpeg: &Path, video: &Path, with_embedded_fa: bool) {
    let srt = video.parent().unwrap().join("fixture.srt");
    fs::write(&srt, SRT_FA).unwrap();
    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-y",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=320x240:d=3",
        "-f",
        "lavfi",
        "-i",
        "sine=d=3",
    ]);
    if with_embedded_fa {
        cmd.arg("-i").arg(&srt);
    }
    cmd.args(["-map", "0:v", "-map", "1:a"]);
    if with_embedded_fa {
        cmd.args([
            "-map",
            "2:s",
            "-metadata:s:s:0",
            "language=fas",
            "-metadata:s:s:0",
            "title=فارسی",
        ]);
    }
    cmd.args(["-c:v", "mpeg4", "-q:v", "7", "-c:a", "aac", "-shortest"]);
    if with_embedded_fa {
        // mp4 نمی‌تواند subrip خام نگه دارد — mov_text (قرینهٔ SoftSub واقعی)
        cmd.args(["-c:s", "mov_text"]);
    }
    cmd.arg(video).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().expect("اجرای ffmpeg");
    assert!(
        out.status.success(),
        "ساخت ویدیوی مصنوعی ناموفق: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_file(&srt);
}

/// probe همهٔ استریم‌ها (نوع + کدک) — برای ادعاهای ساختاری خروجی.
fn probe_all_streams(ffprobe: &Path, video: &Path) -> Vec<(String, String)> {
    let mut cmd = Command::new(ffprobe);
    cmd.args([
        "-v",
        "error",
        "-show_entries",
        "stream=codec_type,codec_name",
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
    let out = cmd.output().expect("اجرای ffprobe");
    assert!(
        out.status.success(),
        "ffprobe خروجی: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json = String::from_utf8_lossy(&out.stdout);
    // parse سبک محلی — بدون وابستگی به ساختار FfprobeOutput داخلی
    let mut streams = Vec::new();
    for chunk in json.split("{").skip(1) {
        let codec_type = chunk
            .split("\"codec_type\"")
            .nth(1)
            .and_then(|rest| rest.split('"').nth(1))
            .unwrap_or("")
            .to_string();
        let codec_name = chunk
            .split("\"codec_name\"")
            .nth(1)
            .and_then(|rest| rest.split('"').nth(1))
            .unwrap_or("")
            .to_string();
        if !codec_type.is_empty() {
            streams.push((codec_type, codec_name));
        }
    }
    streams
}

fn source_fingerprint(path: &Path) -> (u64, Option<SystemTime>) {
    let meta = fs::symlink_metadata(path).expect("منبع باید موجود بماند");
    (meta.len(), meta.modified().ok())
}

fn leftover_workspaces(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().starts_with(".subsync-hardsub-"))
                .unwrap_or(false)
        })
        .collect()
}

fn assert_burn_success(
    ffprobe: &Path,
    video: &Path,
    output: &Path,
    source_fp: (u64, Option<SystemTime>),
) {
    // خروجی نصب شده
    let meta = fs::symlink_metadata(output).expect("خروجی هاردساب باید نصب شده باشد");
    assert!(
        meta.is_file() && meta.len() > 0,
        "خروجی باید فایل غیرخالی باشد"
    );

    // ساختار خروجی: یک استریم ویدیوی h264، صدا حفظ‌شده، بدون Track زیرنویس
    let streams = probe_all_streams(ffprobe, output);
    let video_streams = streams.iter().filter(|(kind, _)| kind == "video").count();
    let audio_streams = streams.iter().filter(|(kind, _)| kind == "audio").count();
    let sub_streams = streams
        .iter()
        .filter(|(kind, _)| kind == "subtitle")
        .count();
    assert!(
        video_streams >= 1,
        "خروجی باید استریم ویدیو داشته باشد: {streams:?}"
    );
    assert!(
        streams
            .iter()
            .any(|(kind, name)| kind == "video" && name == "h264"),
        "استریم ویدیو باید h264 باشد (خروجی بازانکود): {streams:?}"
    );
    assert_eq!(audio_streams, 1, "صدا باید با copy حفظ شود: {streams:?}");
    assert_eq!(
        sub_streams, 0,
        "زیرنویس نباید به‌صورت Track بماند: {streams:?}"
    );

    // مدت خروجی ≈ مدت منبع (±15٪) — بدون حذف/کشش زمانی
    let no_cancel = AtomicBool::new(false);
    let source_duration =
        crate::media::ffprobe::probe_duration(ffprobe, video, &no_cancel).expect("probe منبع");
    let output_duration =
        crate::media::ffprobe::probe_duration(ffprobe, output, &no_cancel).expect("probe خروجی");
    if let (Some(src), Some(out)) = (source_duration, output_duration) {
        let ratio = out / src;
        assert!(
            (0.85..=1.15).contains(&ratio),
            "نسبت مدت خروجی/منبع باید ~۱ باشد: src={src} out={out}"
        );
    }

    // منبع دست‌نخورده + بدون workspace باقی‌مانده
    assert_eq!(
        source_fingerprint(video),
        source_fp,
        "فایل اصلی نباید تغییر کند"
    );
    assert!(
        leftover_workspaces(video.parent().unwrap()).is_empty(),
        "workspace باید پاک شده باشد"
    );
}

/// سورس sidecar → پلن → burn واقعی → ساختار خروجی + دست‌نخوردگی منبع.
#[test]
#[ignore]
fn real_hardsub_sidecar_burns_and_preserves_source() {
    let ffmpeg = resolve_tool_from_env_or_path("ffmpeg");
    let ffprobe = resolve_tool_from_env_or_path("ffprobe");
    let dir = unique_temp_dir("sidecar");
    fs::create_dir_all(&dir).unwrap();

    let video = dir.join("Show.S01E01.720p.mkv");
    build_synthetic_video(&ffmpeg, &video, false);
    let sub = dir.join("Show.S01E01.720p.fa.synced.srt");
    fs::write(&sub, SRT_FA).unwrap();
    let source_fp = source_fingerprint(&video);
    let sub_fp = source_fingerprint(&sub);

    let no_cancel = AtomicBool::new(false);
    let chosen =
        encoder::resolve_encoder(&ffmpeg, crate::settings::HardsubEncoder::Auto, &no_cancel);
    let burn_plan = plan::build_plan(
        &[video.to_string_lossy().into_owned()],
        &ffprobe,
        &no_cancel,
        chosen.label(),
    );
    assert_eq!(burn_plan.items.len(), 1, "پلن: {burn_plan:?}");
    assert!(burn_plan.skipped.is_empty(), "پلن: {burn_plan:?}");
    let item = &burn_plan.items[0];
    assert_eq!(
        item.subtitle_path.as_deref(),
        Some(sub.to_string_lossy().as_ref())
    );
    let output = PathBuf::from(&item.output_path);

    let result = burn::burn(&burn::BurnRequest {
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        video: &video,
        output: &output,
        source: plan::SubtitleSource::File(sub.clone()),
        encoder: chosen,
        quality: crate::settings::HardsubQuality::Fast,
        duration_seconds: crate::media::ffprobe::probe_duration(&ffprobe, &video, &no_cancel)
            .unwrap(),
        timeout_override: None,
        cancel: &no_cancel,
        progress_slot: Some(Arc::new(Mutex::new(None))),
        typography: &Default::default(),
    });
    let outcome = result.unwrap_or_else(|error| panic!("burn باید موفق شود: {error:?}"));
    assert_eq!(outcome.output_path, output);
    assert!(
        outcome.cleanup_warning.is_none(),
        "{:?}",
        outcome.cleanup_warning
    );

    assert_burn_success(&ffprobe, &video, &output, source_fp);
    assert_eq!(
        source_fingerprint(&sub),
        sub_fp,
        "زیرنویس هم نباید تغییر کند"
    );

    let _ = fs::remove_dir_all(&dir);
}

/// منبع Track داخلی (سورس SoftSub) → استخراج واقعی → burn.
#[test]
#[ignore]
fn real_hardsub_embedded_track_extraction_and_burn() {
    let ffmpeg = resolve_tool_from_env_or_path("ffmpeg");
    let ffprobe = resolve_tool_from_env_or_path("ffprobe");
    let dir = unique_temp_dir("embedded");
    fs::create_dir_all(&dir).unwrap();

    let video = dir.join("Movie.SoftSub.mp4");
    build_synthetic_video(&ffmpeg, &video, true);
    let source_fp = source_fingerprint(&video);

    let no_cancel = AtomicBool::new(false);
    let mut burn_plan = plan::build_plan(
        &[video.to_string_lossy().into_owned()],
        &ffprobe,
        &no_cancel,
        "CPU (libx264)",
    );
    assert_eq!(burn_plan.items.len(), 1, "پلن: {burn_plan:?}");
    let item = burn_plan.items.remove(0);
    assert!(item.subtitle_path.is_none(), "بدون sidecar → منبع داخلی");
    let stream_index = item.embedded_stream_index.expect("index Track داخلی");
    let output = PathBuf::from(&item.output_path);
    assert!(output.to_string_lossy().ends_with(".fa.hardsubbed.mp4"));

    let result = burn::burn(&burn::BurnRequest {
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        video: &video,
        output: &output,
        source: plan::SubtitleSource::EmbeddedStream { stream_index },
        encoder: encoder::VideoEncoder::Libx264,
        quality: crate::settings::HardsubQuality::Fast,
        duration_seconds: crate::media::ffprobe::probe_duration(&ffprobe, &video, &no_cancel)
            .unwrap(),
        timeout_override: None,
        cancel: &no_cancel,
        progress_slot: Some(Arc::new(Mutex::new(None))),
        typography: &Default::default(),
    });
    result.unwrap_or_else(|error| panic!("burn باید موفق شود: {error:?}"));

    assert_burn_success(&ffprobe, &video, &output, source_fp);
    let _ = fs::remove_dir_all(&dir);
}

/// خروجی موجود → Conflict؛ منبع و خروجی قبلی هرگز بازنویسی نمی‌شوند.
#[test]
#[ignore]
fn real_hardsub_existing_output_conflicts_without_overwrite() {
    let ffmpeg = resolve_tool_from_env_or_path("ffmpeg");
    let ffprobe = resolve_tool_from_env_or_path("ffprobe");
    let dir = unique_temp_dir("conflict");
    fs::create_dir_all(&dir).unwrap();

    let video = dir.join("Show.S01E02.720p.mkv");
    build_synthetic_video(&ffmpeg, &video, false);
    let sub = dir.join("Show.S01E02.720p.fa.srt");
    fs::write(&sub, SRT_FA).unwrap();
    let output = dir.join("Show.S01E02.720p.fa.hardsubbed.mkv");
    fs::write(&output, b"PREVIOUS-OUTPUT").unwrap();
    let output_fp = source_fingerprint(&output);
    let source_fp = source_fingerprint(&video);

    let no_cancel = AtomicBool::new(false);
    let result = burn::burn(&burn::BurnRequest {
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        video: &video,
        output: &output,
        source: plan::SubtitleSource::File(sub.clone()),
        encoder: encoder::VideoEncoder::Libx264,
        quality: crate::settings::HardsubQuality::Fast,
        duration_seconds: Some(3.0),
        timeout_override: None,
        cancel: &no_cancel,
        progress_slot: None,
        typography: &Default::default(),
    });
    assert!(
        matches!(result, Err(burn::BurnError::Conflict)),
        "{result:?}"
    );
    assert_eq!(source_fingerprint(&output), output_fp);
    assert_eq!(source_fingerprint(&video), source_fp);
    assert!(
        leftover_workspaces(&dir).is_empty(),
        "workspace در مسیر Conflict باید پاک شود"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// ویدیوی واقعی آرشیو کاربر (اختیاری): SUBSYNC_REAL_HARDSUB_VIDEO —
/// فقط خواندنی؛ خروجی در temp و منبع با fingerprint تأیید می‌شود.
#[test]
#[ignore]
fn real_archive_video() {
    let Ok(video) = std::env::var("SUBSYNC_REAL_HARDSUB_VIDEO") else {
        eprintln!("SKIP: SUBSYNC_REAL_HARDSUB_VIDEO تنظیم نشده — این تست اختیاری است");
        return;
    };
    let ffmpeg = resolve_tool_from_env_or_path("ffmpeg");
    let ffprobe = resolve_tool_from_env_or_path("ffprobe");
    let video = PathBuf::from(video);
    assert!(
        video.is_file(),
        "ویدیوی واقعی موجود نیست: {}",
        video.display()
    );
    let source_fp = source_fingerprint(&video);

    let dir = unique_temp_dir("archive");
    fs::create_dir_all(&dir).unwrap();
    let sub = dir.join("archive.fa.srt");
    fs::write(&sub, SRT_FA).unwrap();

    let no_cancel = AtomicBool::new(false);
    let chosen =
        encoder::resolve_encoder(&ffmpeg, crate::settings::HardsubEncoder::Auto, &no_cancel);
    crate::logging::info(
        "hardsub-real",
        &format!("انکودر انتخاب‌شده: {}", chosen.label()),
    );

    // خروجی در temp — آرشیو دست‌نخورده می‌ماند
    let output = dir.join(format!(
        "{}.fa.hardsubbed.mkv",
        video.file_stem().unwrap().to_string_lossy()
    ));
    let result = burn::burn(&burn::BurnRequest {
        ffmpeg: &ffmpeg,
        ffprobe: &ffprobe,
        video: &video,
        output: &output,
        source: plan::SubtitleSource::File(sub.clone()),
        encoder: chosen,
        quality: crate::settings::HardsubQuality::Fast,
        duration_seconds: crate::media::ffprobe::probe_duration(&ffprobe, &video, &no_cancel)
            .unwrap(),
        timeout_override: None,
        cancel: &no_cancel,
        progress_slot: Some(Arc::new(Mutex::new(None))),
        typography: &Default::default(),
    });
    let outcome = result.unwrap_or_else(|error| panic!("burn باید موفق شود: {error:?}"));
    assert_eq!(outcome.output_path, output);

    // منبع واقعی دست‌نخورده؛ خروجی قابل خواندن (ادعای مدت با tolerance گسترده)
    assert_eq!(
        source_fingerprint(&video),
        source_fp,
        "آرشیو واقعی نباید تغییر کند"
    );
    let out_duration =
        crate::media::ffprobe::probe_duration(&ffprobe, &output, &no_cancel).expect("probe خروجی");
    assert!(
        out_duration.is_some(),
        "خروجی باید مدت قابل خواندن داشته باشد"
    );
    let _ = fs::remove_dir_all(&dir);
}
