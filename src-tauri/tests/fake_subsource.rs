//! Fake executable برای تست واقعی CLI adapter
//!
//! حالت‌ها (از آرگومان صریح یا فایل marker `.fake_mode` در پوشه کاری جاری،
//! پوشه `--directory` یا والدین آن تا ریشه):
//! - `slow` / `large-output` / `large-stderr` / `leak-credentials` /
//!   `leak-credentials-error` / `simulate-network-error` — رفتارهای قدیمی
//! - `write-archives` — برای هر ویدیوی داخل directory، آرشیو جعلی در
//!   `<video_dir>/sub/` می‌سازد (شبیه‌سازی دانلود موفق) و موفق خارج می‌شود
//! - `write-archives-then-fail` — همان نوشتن آرشیوها، سپس کد خروج ۱
//!   (شبیه‌سازی دانلود جزئی)
//! - `write-then-slow` — آرشیوها را می‌سازد و سپس مدت‌ها ادامه می‌دهد
//!   (شبیه‌سازی لغو در میانه دانلود)

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::Duration;

const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "webm", "mpg", "mpeg", "wmv", "flv",
];

fn directory_arg(args: &[String]) -> Option<PathBuf> {
    let pos = args.iter().position(|a| a == "-d" || a == "--directory")?;
    args.get(pos + 1).map(PathBuf::from)
}

fn determine_mode(args: &[String]) -> Option<String> {
    // 1. بررسی آرگومان‌های صریح (در صورت وجود)
    for arg in args {
        if arg == "--simulate-network-error"
            || arg == "--slow"
            || arg == "--large-output"
            || arg == "--large-stderr"
            || arg == "--leak-credentials"
            || arg == "--leak-credentials-error"
        {
            return Some(arg.trim_start_matches("--").to_string());
        }
    }

    // 2. بررسی فایل marker اختصاصی (.fake_mode) در پوشه کاری جاری، پوشه
    //    --directory و همهٔ والدین آن — تا تست‌های staging بتوانند marker را
    //    در پوشه بیس بگذارند در حالی که CLI با ریشه staging اجرا می‌شود.
    let mut check_dirs = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        check_dirs.push(cwd);
    }
    if let Some(dir) = directory_arg(args) {
        check_dirs.push(dir.clone());
        let mut anc = dir.parent();
        while let Some(a) = anc {
            check_dirs.push(a.to_path_buf());
            anc = a.parent();
        }
    }

    for dir in check_dirs {
        let marker = dir.join(".fake_mode");
        if marker.exists() {
            if let Ok(content) = fs::read_to_string(&marker) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}

/// آیا این فایل ویدیو حساب می‌شود؟
fn is_video(path: &Path) -> bool {
    path.extension()
        .map(|e| {
            let e = e.to_string_lossy().to_lowercase();
            VIDEO_EXTS.contains(&e.as_str())
        })
        .unwrap_or(false)
}

/// جمع‌آوری بازگشتی ویدیوها (پوشه‌های sub/ نادیده گرفته می‌شوند)
fn collect_videos(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if p.file_name().map(|n| n == "sub").unwrap_or(false) {
                continue;
            }
            collect_videos(&p, out);
        } else if p.is_file() && is_video(&p) {
            out.push(p);
        }
    }
}

/// شبیه‌سازی دانلود: برای هر ویدیو یک آرشیو جعلی در `<video_dir>/sub/`
fn write_archives(root: &Path) -> usize {
    let mut videos = Vec::new();
    collect_videos(root, &mut videos);
    let mut written = 0;
    for video in &videos {
        let Some(parent) = video.parent() else {
            continue;
        };
        let Some(stem) = video.file_stem() else {
            continue;
        };
        let sub_dir = parent.join("sub");
        if fs::create_dir_all(&sub_dir).is_err() {
            continue;
        }
        let archive = sub_dir.join(format!(
            "{}_sub_{}.zip",
            stem.to_string_lossy(),
            written + 1
        ));
        if fs::write(&archive, b"FAKE SUBSOURCE ARCHIVE").is_ok() {
            written += 1;
        }
    }
    written
}

const ZIP_MAGIC: &[u8] = &[0x50, 0x4B, 0x03, 0x04];

/// زیرنویس فارسی معتبر داخل آرشیو — UTF-8 با محتوای فارسی واقعی
const SRT_FA_MEMBER: &str =
    "1\n00:00:01,000 --> 00:00:04,000\nسلام دنیا\n\n2\n00:00:05,000 --> 00:00:08,000\nخط دوم فارسی\n";

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// عضو fake_7z سازگار: `F|path|size|attrs|symlink|hardlink|reparse|hex|flags`
fn manifest_member(path: &str, content: &[u8]) -> String {
    format!(
        "F|{}|{}||{}|{}|{}|{}|",
        path,
        content.len(),
        "",
        "",
        "",
        hex(content)
    )
}

/// آرشیو سازگار با fake_7z: magic واقعی ZIP + manifest متنی اعضا.
/// `ambiguous` = دو عضو منطبق فارسی برای سناریوی تصمیم کاربر.
fn write_manifest_archive(path: &Path, stem: &str, ambiguous: bool) -> bool {
    let mut data = ZIP_MAGIC.to_vec();
    let primary = manifest_member(&format!("{stem}.fa.srt"), SRT_FA_MEMBER.as_bytes());
    data.extend_from_slice(primary.as_bytes());
    data.push(b'\n');
    if ambiguous {
        let second = manifest_member(&format!("{stem}.fa.2.srt"), SRT_FA_MEMBER.as_bytes());
        data.extend_from_slice(second.as_bytes());
        data.push(b'\n');
    }
    fs::write(path, &data).is_ok()
}

/// دانلود شبیه‌سازی‌شده با آرشیوهای manifestدار — برای تست انتها-به-انتهای
/// pipeline که پس از دانلود، آرشیو را با fake 7-Zip باز می‌کند.
fn write_manifest_archives(root: &Path, ambiguous: bool) -> usize {
    let mut videos = Vec::new();
    collect_videos(root, &mut videos);
    let mut written = 0;
    for video in &videos {
        let Some(parent) = video.parent() else {
            continue;
        };
        let Some(stem) = video.file_stem() else {
            continue;
        };
        let sub_dir = parent.join("sub");
        if fs::create_dir_all(&sub_dir).is_err() {
            continue;
        }
        let archive = sub_dir.join(format!(
            "{}_sub_{}.zip",
            stem.to_string_lossy(),
            written + 1
        ));
        if write_manifest_archive(&archive, &stem.to_string_lossy(), ambiguous) {
            written += 1;
        }
    }
    written
}

fn main() {
    let args: Vec<String> = env::args().collect();

    // خروجی راهنما
    if args.contains(&"--help".to_string()) {
        println!("Fake SubSource CLI for testing");
        println!("Usage: fake-subsource [OPTIONS]");
        process::exit(0);
    }

    // شبیه‌سازی خطای احراز هویت
    if let Some(pos) = args.iter().position(|a| a == "--api-key") {
        if let Some(key) = args.get(pos + 1) {
            if key == "invalid-key" {
                eprintln!("Error: Invalid API key. Authentication failed (401)");
                process::exit(1);
            }
        }
    }

    let mode = determine_mode(&args);

    // شبیه‌سازی خطای شبکه
    if mode.as_deref() == Some("simulate-network-error") {
        eprintln!("Error: Connection timeout - network unreachable");
        process::exit(2);
    }

    // شبیه‌سازی عملیات طولانی برای تست cancellation
    if mode.as_deref() == Some("slow") {
        eprintln!("Starting slow operation...");
        for i in 0..30 {
            println!("Progress: {}%", i * 10);
            thread::sleep(Duration::from_millis(500));
        }
        println!("Slow operation completed");
        process::exit(0);
    }

    // شبیه‌سازی خروجی حجیم stdout
    if mode.as_deref() == Some("large-output") {
        for i in 0..1000 {
            println!("Line {}: Some output data", i);
        }
        process::exit(0);
    }

    // شبیه‌سازی stderr حجیم
    if mode.as_deref() == Some("large-stderr") {
        for i in 0..1000 {
            eprintln!("Error line {}: Some error data", i);
        }
        process::exit(1);
    }

    // شبیه‌سازی نشت API Key در stdout و stderr (برای بررسی redaction در خروجی موفق)
    if mode.as_deref() == Some("leak-credentials") {
        if let Some(pos) = args.iter().position(|a| a == "--api-key") {
            if let Some(key) = args.get(pos + 1) {
                println!("API Key used: {}", key);
                eprintln!("API Key echoed on stderr: {}", key);
                process::exit(0);
            }
        }
        process::exit(0);
    }

    // شبیه‌سازی نشت API Key در مسیر خطا (برای بررسی redaction در Error Display)
    if mode.as_deref() == Some("leak-credentials-error") {
        if let Some(pos) = args.iter().position(|a| a == "--api-key") {
            if let Some(key) = args.get(pos + 1) {
                println!("Leaked credential: {}", key);
                eprintln!("Credential leak on stderr: {}", key);
                process::exit(1);
            }
        }
        process::exit(1);
    }

    let target = directory_arg(&args).unwrap_or_else(|| env::current_dir().unwrap_or_default());

    // شبیه‌سازی دانلود موفق: آرشیو جعلی کنار هر ویدیو در sub/
    if mode.as_deref() == Some("write-archives") {
        let n = write_archives(&target);
        println!("Downloaded {n} subtitles successfully");
        process::exit(0);
    }

    // شبیه‌سازی دانلود جزئی: آرشیوها نوشته می‌شوند ولی کد خروج غیر صفر
    if mode.as_deref() == Some("write-archives-then-fail") {
        let n = write_archives(&target);
        println!("Downloaded {n} subtitles before failure");
        eprintln!("Error: simulated partial failure after writing {n} archives");
        process::exit(1);
    }

    // شبیه‌سازی لغو در میانه: آرشیوها نوشته می‌شوند و سپس عملیات ادامه می‌یابد
    if mode.as_deref() == Some("write-then-slow") {
        let n = write_archives(&target);
        println!("Downloaded {n} subtitles, continuing...");
        for i in 0..60 {
            println!("Progress: {}%", i * 5);
            thread::sleep(Duration::from_millis(500));
        }
        process::exit(0);
    }

    // آرشیوهای manifestدار (سازگار با fake 7-Zip) — یک عضو منطبق
    if mode.as_deref() == Some("write-manifest-archives") {
        let n = write_manifest_archives(&target, false);
        println!("Downloaded {n} subtitles successfully");
        process::exit(0);
    }

    // آرشیوهای manifestدار با دو عضو منطبق — سناریوی ابهام
    if mode.as_deref() == Some("write-manifest-archives-ambiguous") {
        let n = write_manifest_archives(&target, true);
        println!("Downloaded {n} subtitles successfully");
        process::exit(0);
    }

    // موفقیت پیش‌فرض
    println!("Downloaded 3 subtitles successfully");
    println!("Processing completed in 2.5 seconds");
    process::exit(0);
}
