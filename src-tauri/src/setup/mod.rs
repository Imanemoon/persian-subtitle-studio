//! نصب خودکار ابزارهای بیرونی — هستهٔ مستقل از Tauri.
//!
//! هر نصب: دانلود (با progress واقعی و لغو) → تأیید SHA256 → نصب per-user
//! بدون نیاز به admin → مسیرهای نهایی فقط از طریق `settings.tool_paths`
//! ثبت می‌شوند (resolver موجود دست‌نخورده می‌ماند).
//!
//! تست‌پذیری: فقط `fetch` تزریق می‌شود (تست‌ها بدون شبکه فایل می‌سازند)؛
//! بقیهٔ مسیر با همان کد تولید اجرا می‌شود.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// فاصلهٔ بررسی پرچم لغو در حلقهٔ خواندن دانلود.
const CANCEL_CHECK_BYTES: u64 = 256 * 1024;

// ---------------------------------------------------------------------
// مشخصات پین‌شدهٔ ابزارها — نسخه + SHA256؛ تغییر نسخه = تغییر دو const
// ---------------------------------------------------------------------

/// ffmpeg essentials از ریلیز تگ‌دار GitHub (میرور رسمی gyan.dev) —
/// نسخهٔ پین‌شده؛ hash واقعی فایل دانلودشدهٔ همین URL.
pub const FFMPEG_URL: &str =
    "https://github.com/GyanD/codexffmpeg/releases/download/8.0/ffmpeg-8.0-essentials_build.zip";
pub const FFMPEG_SHA256: &str = "647E467CAF82B9FA200A562769B5FF4D736AAF725804ED2C64EA9752106FA569";

/// پایتون ۳٫۱۱٫۹ (آخرین ۳٫۱۱ با installer ویندوز) — نصب per-user بدون admin.
pub const PYTHON_URL: &str = "https://www.python.org/ftp/python/3.11.9/python-3.11.9-amd64.exe";
pub const PYTHON_SHA256: &str = "5EE42C4EEE1E6B4464BB23722F90B45303F79442DF63083F05322F1785F5FDDE";

/// نسخهٔ ffsubsync — همان نسخهٔ آماده‌شدهٔ پروژه (legacy flow).
pub const FFSUBSYNC_VERSION: &str = "0.5.1";

/// مشخصهٔ یک ابزار نصب‌شدنی در معاونت راه‌اندازی.
pub struct SetupToolSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// بدون این‌ها برنامه عملاً بی‌فایده است (ffmpeg/ffprobe).
    pub required_for_entry: bool,
    /// برچسب توضیحی صادقانه — حجم دانلود واقعی.
    pub size_hint: &'static str,
}

pub const SETUP_TOOLS: [SetupToolSpec; 3] = [
    SetupToolSpec {
        id: "ffmpeg",
        label: "FFmpeg / FFprobe (استخراج زیرنویس و سینک)",
        required_for_entry: true,
        size_hint: "دانلود ~۹۰MB",
    },
    SetupToolSpec {
        id: "ffsubsync",
        label: "FFsubsync + Python (همگام‌سازی)",
        required_for_entry: false,
        size_hint: "دانلود ~۱۲۰MB — نصبش چند دقیقه طول می‌کشد",
    },
    SetupToolSpec {
        id: "sevenZip",
        label: "7-Zip (استخراج آرشیوها)",
        required_for_entry: false,
        size_hint: "نصاب باندل‌شده — نیازی به دانلود ندارد",
    },
];

pub fn spec_by_id(id: &str) -> Option<&'static SetupToolSpec> {
    SETUP_TOOLS.iter().find(|s| s.id == id)
}

// ---------------------------------------------------------------------
// progress
// ---------------------------------------------------------------------

/// پیام پیشرفت — همهٔ اعداد از واقعیت می‌آیند؛ هیچ درصد جعلی‌ای نیست.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupProgress {
    pub tool_id: String,
    pub phase: String,
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub message: String,
}

pub type ProgressFn<'a> = &'a mut dyn FnMut(SetupProgress);

fn emit(
    tool_id: &str,
    phase: &str,
    received: u64,
    total: u64,
    message: &str,
    on_progress: ProgressFn,
) {
    on_progress(SetupProgress {
        tool_id: tool_id.to_string(),
        phase: phase.to_string(),
        received_bytes: received,
        total_bytes: total,
        message: message.to_string(),
    });
}

// ---------------------------------------------------------------------
// دانلود + hash
// ---------------------------------------------------------------------

/// دانلود استریمی با progress واقعی، لغو و تأیید SHA256.
/// نوشتن در فایل tmp و rename اتمیک — فایل نیمه‌کاره هرگز به‌عنوان کامل دیده نمی‌شود.
pub fn download_file(
    tool_id: &str,
    url: &str,
    sha256: Option<&str>,
    dest: &Path,
    on_progress: ProgressFn,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("ساخت پوشهٔ مقصد «{}» ناموفق: {e}", parent.display()))?;
    }
    let tmp = dest.with_extension("part");
    emit(
        tool_id,
        "download",
        0,
        0,
        &format!("اتصال به {}…", host_of(url)),
        on_progress,
    );

    let response = ureq::get(url)
        .timeout(Duration::from_secs(600))
        .call()
        .map_err(|e| format!("دانلود «{url}» ناموفق: {e}"))?;
    let total = response
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);

    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&tmp)
        .map_err(|e| format!("ساخت فایل موقت «{}» ناموفق: {e}", tmp.display()))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut buf = [0u8; 64 * 1024];
    let mut last_report = 0u64;
    loop {
        if cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err("دانلود توسط کاربر لغو شد".into());
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("خواندن استریم دانلود ناموفق: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("نوشتن فایل موقت ناموفق: {e}"))?;
        hasher.update(&buf[..n]);
        received += n as u64;
        if received.saturating_sub(last_report) >= CANCEL_CHECK_BYTES {
            last_report = received;
            emit(
                tool_id,
                "download",
                received,
                total,
                &format!(
                    "دانلود {} از {} — {}",
                    fmt_bytes(received),
                    fmt_bytes(total.max(received)),
                    url
                ),
                on_progress,
            );
        }
    }
    file.flush().ok();
    drop(file);
    if total > 0 && received != total {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("دانلود ناقص بود: {} از {} بایت", received, total));
    }

    if let Some(expected) = sha256 {
        let actual = format!("{:x}", hasher.finalize());
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!(
                "تأیید SHA256 ناموفق — انتظار {expected}، دریافت {actual}. فایل از منبع دستکاری‌شده یا نسخهٔ دیگری است."
            ));
        }
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("جابه‌جایی فایل دانلودشده ناموفق: {e}"))?;
    emit(
        tool_id,
        "download",
        received,
        received,
        &format!("دانلود کامل شد — {}", fmt_bytes(received)),
        on_progress,
    );
    Ok(())
}

/// تأیید مستقل hash یک فایل روی دیسک (برای فایل‌های fetchشده در تست‌ها).
pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("باز کردن «{}» برای hash ناموفق: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "تأیید SHA256 ناموفق — انتظار {expected}، دریافت {actual}"
        ))
    }
}

// ---------------------------------------------------------------------
// استخراج zip و اجرای silent
// ---------------------------------------------------------------------

/// استخراج فقط اعضای خواسته‌شده (تطبیق پسوند نام فایل، بی‌توجه به پوشهٔ داخلی zip).
pub fn extract_zip_members(
    zip_path: &Path,
    wanted_suffixes: &[&str],
    dest_dir: &Path,
) -> Result<Vec<PathBuf>, String> {
    let zip_file = std::fs::File::open(zip_path)
        .map_err(|e| format!("باز کردن «{}» ناموفق: {e}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(zip_file)
        .map_err(|e| format!("خواندن zip «{}» ناموفق: {e}", zip_path.display()))?;
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("ساخت پوشهٔ مقصد «{}» ناموفق: {e}", dest_dir.display()))?;
    let mut extracted = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("خواندن عضو zip ناموفق: {e}"))?;
        let name = entry.name().to_string();
        let Some(_) = wanted_suffixes
            .iter()
            .find(|s| name.to_lowercase().ends_with(&s.to_lowercase()))
        else {
            continue;
        };
        let file_name = Path::new(&name)
            .file_name()
            .ok_or_else(|| format!("نام عضو zip نامعتبر است: {name}"))?
            .to_string_lossy()
            .into_owned();
        let out = dest_dir.join(&file_name);
        let mut out_file = std::fs::File::create(&out)
            .map_err(|e| format!("ساخت «{}» ناموفق: {e}", out.display()))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("استخراج «{name}» ناموفق: {e}"))?;
        extracted.push(out);
    }
    if extracted.len() < wanted_suffixes.len() {
        return Err(format!(
            "اعضای مورد انتظار در zip یافت نشد — یافت‌شده: {}",
            extracted.len()
        ));
    }
    Ok(extracted)
}

/// اجرای silent بدون پنجره — خروجی جذب می‌شود تا بافر پر نشود.
pub fn run_silent(program: &Path, args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("اجرای «{}» ناموفق: {e}", program.display()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(" — {detail}")
        };
        return Err(format!(
            "«{}» با کد خروج {:?} تمام شد{}",
            program.display(),
            output.status.code(),
            detail
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------
// نصب هر ابزار — fetch تزریق‌شده؛ تولید = download_file با progress HTTP
// ---------------------------------------------------------------------

pub type FetchFn<'a> = &'a mut dyn FnMut(&str, &Path) -> Result<(), String>;

/// مسیرهای نهایی ffmpeg/ffprobe پس از نصب — فقط داخل پوشهٔ tools برنامه.
#[derive(Debug)]
pub struct FfmpegInstall {
    pub ffmpeg_path: PathBuf,
    pub ffprobe_path: PathBuf,
}

/// ffmpeg: دانلود zip پین‌شده → استخراج ffmpeg.exe و ffprobe.exe.
pub fn install_ffmpeg(
    tools_dir: &Path,
    fetch: FetchFn,
    sha256: Option<&str>,
    on_progress: ProgressFn,
    cancel: &AtomicBool,
) -> Result<FfmpegInstall, String> {
    let dest_dir = tools_dir.join("ffmpeg");
    let zip_path = tools_dir
        .join("cache")
        .join("ffmpeg-8.0-essentials_build.zip");
    emit(
        "ffmpeg",
        "download",
        0,
        0,
        "شروع دانلود FFmpeg…",
        on_progress,
    );
    if cancel.load(Ordering::SeqCst) {
        return Err("نصب توسط کاربر لغو شد".into());
    }
    fetch(FFMPEG_URL, &zip_path)?;
    if let Some(expected) = sha256 {
        verify_sha256(&zip_path, expected)?;
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("نصب توسط کاربر لغو شد".into());
    }
    emit(
        "ffmpeg",
        "extract",
        0,
        0,
        "استخراج ffmpeg.exe و ffprobe.exe…",
        on_progress,
    );
    let extracted = extract_zip_members(&zip_path, &["ffmpeg.exe", "ffprobe.exe"], &dest_dir)?;
    let ffmpeg_path = extracted
        .iter()
        .find(|p| {
            p.file_name()
                .map(|n| n.eq_ignore_ascii_case("ffmpeg.exe"))
                .unwrap_or(false)
        })
        .cloned()
        .ok_or("ffmpeg.exe در zip یافت نشد")?;
    let ffprobe_path = extracted
        .iter()
        .find(|p| {
            p.file_name()
                .map(|n| n.eq_ignore_ascii_case("ffprobe.exe"))
                .unwrap_or(false)
        })
        .cloned()
        .ok_or("ffprobe.exe در zip یافت نشد")?;
    emit(
        "ffmpeg",
        "done",
        1,
        1,
        &format!("FFmpeg آماده است — {}", ffmpeg_path.display()),
        on_progress,
    );
    Ok(FfmpegInstall {
        ffmpeg_path,
        ffprobe_path,
    })
}

/// مسیر نهایی ffs.exe پس از نصب ffsubsync.
#[derive(Debug)]
pub struct FfsubsyncInstall {
    pub ffs_path: PathBuf,
}

/// ffsubsync: نصب silent پایتون per-user → venv → pip install → probe.
/// مراحل طولانی‌اند؛ progress phase-based و صادقانه است (پایپ پکیج‌ها
/// درصد قابل‌اعتمادی نمی‌دهد) و لغو فقط بین مراحل بررسی می‌شود.
pub fn install_ffsubsync(
    tools_dir: &Path,
    fetch: FetchFn,
    sha256: Option<&str>,
    on_progress: ProgressFn,
    cancel: &AtomicBool,
) -> Result<FfsubsyncInstall, String> {
    let python_dir = tools_dir.join("python");
    let python_exe = python_dir.join("python.exe");
    let venv_dir = tools_dir.join("ffsubsync-venv");
    let venv_python = venv_dir.join("Scripts").join("python.exe");
    let ffs_exe = venv_dir.join("Scripts").join("ffs.exe");

    if ffs_exe.is_file() {
        return Ok(FfsubsyncInstall { ffs_path: ffs_exe });
    }

    // ۱) پایتون per-user (اگر از قبل نیست)
    if !python_exe.is_file() {
        let installer = tools_dir.join("cache").join("python-3.11.9-amd64.exe");
        emit(
            "ffsubsync",
            "download",
            0,
            0,
            "دانلود پایتون ۳٫۱۱٫۹ (بدون نیاز به admin)…",
            on_progress,
        );
        if cancel.load(Ordering::SeqCst) {
            return Err("نصب توسط کاربر لغو شد".into());
        }
        fetch(PYTHON_URL, &installer)?;
        if let Some(expected) = sha256 {
            verify_sha256(&installer, expected)?;
        }
        emit(
            "ffsubsync",
            "install",
            0,
            0,
            "نصب silent پایتون (per-user)…",
            on_progress,
        );
        run_silent(
            &installer,
            &[
                "/quiet".to_string(),
                "InstallAllUsers=0".to_string(),
                "Include_pip=1".to_string(),
                "Include_test=0".to_string(),
                "PrependPath=0".to_string(),
                "Shortcuts=0".to_string(),
                format!("TargetDir={}", python_dir.display()),
            ],
        )?;
        if !python_exe.is_file() {
            return Err(format!(
                "پایتون پس از نصب در «{}» یافت نشد",
                python_exe.display()
            ));
        }
    }

    // ۲) venv
    if !venv_python.is_file() {
        emit(
            "ffsubsync",
            "install",
            0,
            0,
            "ساخت محیط مجازی پایتون…",
            on_progress,
        );
        if cancel.load(Ordering::SeqCst) {
            return Err("نصب توسط کاربر لغو شد".into());
        }
        run_silent(
            &python_exe,
            &["-m".into(), "venv".into(), venv_dir.display().to_string()],
        )?;
        if !venv_python.is_file() {
            return Err(format!(
                "محیط مجازی پس از ساخت در «{}» یافت نشد",
                venv_python.display()
            ));
        }
    }

    // ۳) pip install — مراحل طولانی؛ پیام صادقانه بدون درصد
    emit(
        "ffsubsync",
        "packages",
        0,
        0,
        "نصب بسته‌های ffsubsync — بسته به اینترنت چند دقیقه طول می‌کشد…",
        on_progress,
    );
    if cancel.load(Ordering::SeqCst) {
        return Err("نصب توسط کاربر لغو شد".into());
    }
    run_silent(
        &venv_python,
        &[
            "-m".into(),
            "pip".into(),
            "install".into(),
            "--upgrade".into(),
            "pip".into(),
        ],
    )?;
    if cancel.load(Ordering::SeqCst) {
        return Err("نصب توسط کاربر لغو شد".into());
    }
    run_silent(
        &venv_python,
        &[
            "-m".into(),
            "pip".into(),
            "install".into(),
            format!("ffsubsync=={FFSUBSYNC_VERSION}"),
        ],
    )?;
    if !ffs_exe.is_file() {
        return Err(format!(
            "ffs.exe پس از نصب در «{}» یافت نشد — pip ممکن است ناموفق بوده باشد",
            ffs_exe.display()
        ));
    }
    emit(
        "ffsubsync",
        "done",
        1,
        1,
        &format!("FFsubsync آماده است — {}", ffs_exe.display()),
        on_progress,
    );
    Ok(FfsubsyncInstall { ffs_path: ffs_exe })
}

/// 7-Zip: اجرای نصاب باندل‌شده با UAC — نصاب رسمی به %ProgramFiles%\7-Zip
/// می‌رود و resolver موجود همان مسیر را می‌شناسد. اگر از قبل هست، هیچ‌کاری نمی‌کند.
pub fn install_seven_zip(installer_path: &Path, on_progress: ProgressFn) -> Result<String, String> {
    emit(
        "sevenZip",
        "install",
        0,
        0,
        "اجرای نصاب رسمی 7-Zip — پنجرهٔ UAC را تأیید کنید…",
        on_progress,
    );
    run_elevated_installer(installer_path)?;
    emit(
        "sevenZip",
        "done",
        1,
        1,
        "نصاب 7-Zip اجرا شد — پس از اتمام، «بررسی دوباره» را بزنید",
        on_progress,
    );
    Ok(installer_path.display().to_string())
}

// ---------------------------------------------------------------------
// PowerShell helpers — امتیاز، Defender، میان‌بر admin
// ---------------------------------------------------------------------

/// اجرای نصاب با امتیاز (UAC) — Start-Process -Verb RunAs -Wait.
fn run_elevated_installer(installer: &Path) -> Result<(), String> {
    let script = format!(
        "try {{ Start-Process {} -Verb RunAs -Wait -ArgumentList '/S' }} catch {{ exit 1602 }}",
        ps_quote_single(&installer.display().to_string()),
    );
    run_powershell(&script).map_err(|e| {
        if e.contains("1602") {
            "پنجرهٔ UAC توسط شما لغو شد — نصب 7-Zip انجام نشد".to_string()
        } else {
            format!("اجرای نصاب 7-Zip ناموفق: {e}")
        }
    })?;
    Ok(())
}

fn run_powershell(script: &str) -> Result<String, String> {
    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("اجرای PowerShell ناموفق: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("PowerShell با کد {:?} تمام شد", output.status.code())
        } else {
            stderr
        });
    }
    Ok(stdout)
}

fn ps_quote_single(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// آیا فرایند فعلی با امتیاز Administrator اجرا می‌شود؟
pub fn is_elevated() -> Result<bool, String> {
    let out = run_powershell(
        "([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)",
    )?;
    Ok(out.trim().eq_ignore_ascii_case("true"))
}

/// افزودن استثنای Windows Defender با رضایت کاربر (UAC).
/// خروجی همیشه صادقانه است: یا «تأیید شد» یا «درخواست ارسال شد — نتیجه نامشخص».
pub fn add_defender_exclusions(paths: &[String]) -> Result<String, String> {
    let valid: Vec<&String> = paths.iter().filter(|p| !p.trim().is_empty()).collect();
    if valid.is_empty() {
        return Err("هیچ مسیری برای افزودن به استثناها داده نشده است".into());
    }
    let adds = valid
        .iter()
        .map(|p| format!("Add-MpPreference -ExclusionPath {}", ps_quote_single(p)))
        .collect::<Vec<_>>()
        .join("; ");
    let inner = format!("{}; exit 0", adds);
    let outer = format!(
        "try {{ Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList {} }} catch {{ exit 1602 }}",
        ps_quote_single(&format!("-NoProfile -NonInteractive -Command {}", inner))
    );
    run_powershell(&outer).map_err(|e| {
        if e.contains("1602") {
            "پنجرهٔ UAC توسط شما لغو شد — استثنایی اضافه نشد".to_string()
        } else {
            e
        }
    })?;
    // تأیید مستقل — اگر خواندن ترجیحات ممکن بود، واقعیت را می‌گوییم
    let verify = format!(
        "(Get-MpPreference).ExclusionPath -contains {}",
        ps_quote_single(valid[0])
    );
    match run_powershell(&verify) {
        Ok(out) if out.trim().eq_ignore_ascii_case("true") => {
            Ok("تأیید شد — مسیرها به استثناهای Windows Defender اضافه شدند".to_string())
        }
        _ => Ok(
            "درخواست اجرا شد؛ اما خواندن ترجیحات Defender برای تأیید ممکن نشد — از پنجرهٔ امنیت ویندوز می‌توانید ببینید".to_string(),
        ),
    }
}

/// ساخت میان‌بر دسکتاپ با پرچم Run as administrator (بایت ۲۱ لینک).
pub fn create_admin_shortcut(target_exe: &Path, link_name: &str) -> Result<String, String> {
    let desktop = run_powershell("[Environment]::GetFolderPath('Desktop')")?;
    let desktop = desktop.trim().to_string();
    if desktop.is_empty() {
        return Err("پوشهٔ Desktop یافت نشد".into());
    }
    let link = Path::new(&desktop).join(format!("{link_name}.lnk"));
    let script = format!(
        r#"$ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut({}); $s.TargetPath = {}; $s.WorkingDirectory = {}; $s.Description = 'SubSync Studio (Run as administrator)'; $s.Save(); $b = [System.IO.File]::ReadAllBytes({}); $b[21] = $b[21] -bor 0x20; [System.IO.File]::WriteAllBytes({}, $b); exit 0"#,
        ps_quote_single(&link.display().to_string()),
        ps_quote_single(&target_exe.display().to_string()),
        ps_quote_single(
            &target_exe
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ),
        ps_quote_single(&link.display().to_string()),
        ps_quote_single(&link.display().to_string()),
    );
    run_powershell(&script).map_err(|e| format!("ساخت میان‌بر ناموفق: {e}"))?;
    Ok(link.display().to_string())
}

// ---------------------------------------------------------------------
// باندل‌ها — sub-rs پیش‌آماده می‌شود و اصلاً وارد معاونت نمی‌شود
// ---------------------------------------------------------------------

/// کپی باینری باندل‌شده از resources به پوشهٔ tools برنامه (فقط اگر ابزار
/// در هیچ‌کجا یافت نشود) — بازگشت: مسیر نهایی در صورت انجام.
pub fn seed_bundled_binary(
    source: &Path,
    dest_dir: &Path,
    file_name: &str,
) -> Result<Option<PathBuf>, String> {
    if !source.is_file() {
        return Ok(None);
    }
    std::fs::create_dir_all(dest_dir)
        .map_err(|e| format!("ساخت پوشهٔ «{}» ناموفق: {e}", dest_dir.display()))?;
    let dest = dest_dir.join(file_name);
    std::fs::copy(source, &dest).map_err(|e| format!("کپی «{}» ناموفق: {e}", source.display()))?;
    Ok(Some(dest))
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

/// نمایش بایت به‌صورت قابل‌خواندن — فقط برای پیام‌های progress.
pub fn fmt_bytes(v: u64) -> String {
    if v >= 1024 * 1024 * 1024 {
        format!("{:.1} گیگ", v as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if v >= 1024 * 1024 {
        format!("{:.0}MB", v as f64 / (1024.0 * 1024.0))
    } else if v >= 1024 {
        format!("{:.0}KB", v as f64 / 1024.0)
    } else {
        format!("{v}B")
    }
}

// ---------------------------------------------------------------------
// تست‌ها — بدون شبکه: fetch فیک، zip ساختگی، لغو و hash نامعتبر
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_with(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            for (name, data) in files {
                w.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn fetch_from(
        files: &'static [(&'static str, &'static [u8])],
    ) -> impl FnMut(&str, &Path) -> Result<(), String> {
        move |_url, dest| {
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(dest, zip_with(files)).unwrap();
            Ok(())
        }
    }

    fn noop(_p: SetupProgress) {}

    #[test]
    fn ffmpeg_install_extracts_both_exes() {
        let dir = std::env::temp_dir().join(format!("setup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files: &'static [(&'static str, &'static [u8])] = Box::leak(Box::new([
            (
                "ffmpeg-8.0-essentials_build/bin/ffmpeg.exe",
                b"ff-exe" as &'static [u8],
            ),
            (
                "ffmpeg-8.0-essentials_build/bin/ffprobe.exe",
                b"probe-exe" as &'static [u8],
            ),
        ]));
        let mut fetch = fetch_from(files);
        let cancel = AtomicBool::new(false);
        let mut progress: Box<dyn FnMut(SetupProgress)> = Box::new(noop);
        let inst = install_ffmpeg(&dir, &mut fetch, None, &mut progress, &cancel).expect("نصب");
        assert!(inst.ffmpeg_path.is_file());
        assert!(inst.ffprobe_path.is_file());
        assert_eq!(std::fs::read(&inst.ffmpeg_path).unwrap(), b"ff-exe");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sha_mismatch_is_honest_error() {
        let dir = std::env::temp_dir().join(format!("setup-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("f.bin");
        std::fs::write(&f, b"payload").unwrap();
        let err = verify_sha256(&f, "deadbeef").unwrap_err();
        assert!(err.contains("تأیید SHA256 ناموفق"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_zip_member_is_reported() {
        let dir = std::env::temp_dir().join(format!("setup-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files: &'static [(&'static str, &'static [u8])] =
            Box::leak(Box::new([("x/only.exe", b"a" as &'static [u8])]));
        let zip_path = dir.join("t.zip");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&zip_path, zip_with(files)).unwrap();
        let err = extract_zip_members(&zip_path, &["ffmpeg.exe", "ffprobe.exe"], &dir).unwrap_err();
        assert!(err.contains("یافت نشد"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn helpers_are_sane() {
        assert_eq!(host_of("https://github.com/a/b.zip"), "github.com");
        assert_eq!(fmt_bytes(1024 * 1024 * 1024), "1.0 گیگ");
        assert_eq!(fmt_bytes(90 * 1024 * 1024), "90MB");
        assert_eq!(fmt_bytes(512), "512B");
        assert_eq!(ps_quote_single("a'b"), "'a''b'");
    }

    /// ffsubsync از قبل نصب → هیچ fetch و pip ای اجرا نمی‌شود.
    #[test]
    fn ffsubsync_skips_when_already_installed() {
        let dir = std::env::temp_dir().join(format!("setup-ffs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ffs = dir.join("ffsubsync-venv/Scripts/ffs.exe");
        std::fs::create_dir_all(ffs.parent().unwrap()).unwrap();
        std::fs::write(&ffs, b"ffs").unwrap();
        let mut fetch = |_url: &str, _dest: &Path| -> Result<(), String> {
            panic!("با ffs موجود، هیچ دانلودی نباید اجرا شود");
        };
        let cancel = AtomicBool::new(false);
        let mut progress: Box<dyn FnMut(SetupProgress)> = Box::new(noop);
        let inst = install_ffsubsync(&dir, &mut fetch, None, &mut progress, &cancel).expect("نصب");
        assert!(inst.ffs_path.is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// لغو در install_ffmpeg قبل از هر fetch — بدون تماس شبکه.
    #[test]
    fn install_cancelled_before_fetch() {
        let dir = std::env::temp_dir().join(format!("setup-cancel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cancel = AtomicBool::new(true);
        let mut fetch = |_url: &str, _dest: &Path| -> Result<(), String> {
            panic!("fetch نباید با لغو از قبل صدا زده شود");
        };
        let mut progress: Box<dyn FnMut(SetupProgress)> = Box::new(noop);
        let err = install_ffmpeg(&dir, &mut fetch, None, &mut progress, &cancel).unwrap_err();
        assert!(err.contains("لغو"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
