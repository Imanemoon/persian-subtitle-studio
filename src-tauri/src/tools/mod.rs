//! تشخیص ابزارهای خارجی (FFmpeg، FFprobe، 7-Zip، SubSource، FFsubsync)
//! با پیمایش مسیرهای پیش‌فرض و PATH و اجرای یک پروب سبک `--version`/`--help`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::models::{ToolInfo, ToolStatus};
use crate::settings::ToolPathOverrides;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

pub fn display_name(id: &str) -> &'static str {
    match id {
        "ffmpeg" => "FFmpeg",
        "ffprobe" => "FFprobe",
        "sevenZip" => "7-Zip",
        "subsource" => "SubSource CLI (sub-rs)",
        "ffsubsync" => "FFsubsync (ffs)",
        _ => "ابزار ناشناس",
    }
}

/// مسیرهای محتمل برای هر ابزار بر اساس محیط ویندوز.
pub fn candidate_paths(id: &str, path_var: Option<&OsStr>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    match id {
        "ffmpeg" | "ffprobe" => {
            out.extend(scan_path(path_var, &format!("{id}.exe")));
        }
        "sevenZip" => {
            for var in ["ProgramFiles", "ProgramFiles(x86)"] {
                if let Some(pf) = std::env::var_os(var) {
                    out.push(PathBuf::from(pf).join("7-Zip").join("7z.exe"));
                }
            }
            out.extend(scan_path(path_var, "7z.exe"));
        }
        "subsource" => {
            if let Some(la) = std::env::var_os("LOCALAPPDATA") {
                out.push(
                    PathBuf::from(la)
                        .join("Programs")
                        .join("SubSource")
                        .join("sub-rs.exe"),
                );
            }
            out.extend(scan_path(path_var, "sub-rs.exe"));
        }
        "ffsubsync" => {
            if let Some(la) = std::env::var_os("LOCALAPPDATA") {
                out.push(
                    PathBuf::from(la)
                        .join("Programs")
                        .join("SubSource")
                        .join("ffsubsync-venv")
                        .join("Scripts")
                        .join("ffs.exe"),
                );
            }
            out.extend(scan_path(path_var, "ffs.exe"));
        }
        _ => {}
    }
    out
}

/// جستجوی یک فایل اجرایی در مقدار PATH داده‌شده.
pub fn scan_path(path_var: Option<&OsStr>, exe: &str) -> Vec<PathBuf> {
    let Some(pv) = path_var else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for dir in std::env::split_paths(pv) {
        let candidate = dir.join(exe);
        if candidate.is_file() {
            out.push(candidate);
        }
    }
    out
}

fn path_var() -> Option<std::ffi::OsString> {
    std::env::var_os("PATH")
}

/// تشخیص همه ابزارها؛ ابتدا مسیر Override تنظیمات، سپس مسیرهای پیش‌فرض.
pub fn detect_all(overrides: &ToolPathOverrides) -> Vec<ToolInfo> {
    let pv = path_var();
    let by_id = |id: &str, probe: &[&str], usage_tolerant: bool| {
        let override_path = match id {
            "ffmpeg" => overrides.ffmpeg.as_deref(),
            "ffprobe" => overrides.ffprobe.as_deref(),
            "subsource" => overrides.subsource.as_deref(),
            "sevenZip" => overrides.seven_zip.as_deref(),
            "ffsubsync" => overrides.ffsubsync.as_deref(),
            _ => None,
        };
        detect(
            id,
            override_path,
            &candidate_paths(id, pv.as_deref()),
            probe,
            usage_tolerant,
        )
    };

    vec![
        by_id("ffmpeg", &["-version"], false),
        by_id("ffprobe", &["-version"], false),
        by_id("sevenZip", &["i"], false),
        by_id("subsource", &["--help"], true),
        by_id("ffsubsync", &["--help"], true),
    ]
}

fn detect(
    id: &str,
    override_path: Option<&str>,
    candidates: &[PathBuf],
    probe_args: &[&str],
    usage_tolerant: bool,
) -> ToolInfo {
    let name = display_name(id).to_string();

    // ۱) مسیر دستی تنظیم‌شده توسط کاربر
    if let Some(p) = override_path.map(str::trim).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(p);
        if !path.is_file() {
            return ToolInfo {
                id: id.into(),
                name,
                status: ToolStatus::Broken,
                path: Some(p.to_string()),
                version: None,
                detail: Some("مسیر تنظیم‌شده وجود ندارد".into()),
            };
        }
        return match run_probe(&path, probe_args, usage_tolerant) {
            Ok(version) => ToolInfo {
                id: id.into(),
                name,
                status: ToolStatus::Found,
                path: Some(path.to_string_lossy().to_string()),
                version: Some(version),
                detail: None,
            },
            Err(detail) => ToolInfo {
                id: id.into(),
                name,
                status: ToolStatus::Broken,
                path: Some(path.to_string_lossy().to_string()),
                version: None,
                detail: Some(detail),
            },
        };
    }

    // ۲) مسیرهای پیش‌فرض و PATH
    let mut broken: Option<(PathBuf, String)> = None;
    for cand in candidates {
        if !cand.is_file() {
            continue;
        }
        match run_probe(cand, probe_args, usage_tolerant) {
            Ok(version) => {
                return ToolInfo {
                    id: id.into(),
                    name,
                    status: ToolStatus::Found,
                    path: Some(cand.to_string_lossy().to_string()),
                    version: Some(version),
                    detail: None,
                };
            }
            Err(err) => {
                if broken.is_none() {
                    broken = Some((cand.clone(), err));
                }
            }
        }
    }

    if let Some((path, err)) = broken {
        return ToolInfo {
            id: id.into(),
            name,
            status: ToolStatus::Broken,
            path: Some(path.to_string_lossy().to_string()),
            version: None,
            detail: Some(format!("ابزار یافت شد اما اجرا نشد: {err}")),
        };
    }

    ToolInfo {
        id: id.into(),
        name,
        status: ToolStatus::Missing,
        path: None,
        version: None,
        detail: Some("در مسیرهای پیش‌فرض و PATH پیدا نشد".into()),
    }
}

/// اجرای یک دستور سبک برای اطمینان از سلامت ابزار و گرفتن شماره نسخه.
fn run_probe(exe: &Path, args: &[&str], usage_tolerant: bool) -> Result<String, String> {
    let mut cmd = Command::new(exe);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = cmd.spawn().map_err(|e| format!("اجرای فرایند: {e}"))?;

    // خواندن هر دو pipe در thread جدا تا از پر شدن بافر جلوگیری شود
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
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                return Err(format!("انتظار برای فرایند: {e}"));
            }
        }
    };

    let stdout = t_out.join().unwrap_or_default();
    let stderr = t_err.join().unwrap_or_default();
    let combined = if stdout.trim().is_empty() {
        stderr
    } else if stderr.trim().is_empty() {
        stdout
    } else {
        format!("{stdout}\n{stderr}")
    };
    let first_line = combined
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(160)
        .collect::<String>();

    match status {
        Some(st) if st.success() => Ok(first_line),
        Some(_) if usage_tolerant && looks_like_usage(&combined) => Ok(first_line),
        Some(st) => Err(match st.code() {
            Some(code) => format!("کد خروج {code}"),
            None => "فرایند به‌صورت غیرعادی خاتمه یافت".into(),
        }),
        None => Err("پاسخ نداد (Timeout)".into()),
    }
}

fn looks_like_usage(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("usage") || lower.contains("options") || lower.contains("help")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_path_finds_exe_in_given_dirs() {
        let dir = std::env::temp_dir().join(format!(
            "subsync-tools-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("fake.exe"), b"").unwrap();

        let found = scan_path(Some(OsStr::new(&dir)), "fake.exe");
        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with("fake.exe"));

        assert!(scan_path(Some(OsStr::new(&dir)), "nothing.exe").is_empty());
        assert!(scan_path(None, "fake.exe").is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn detect_with_missing_override_reports_broken() {
        let info = detect(
            "ffmpeg",
            Some("C:\\definitely\\not\\a\\real\\ffmpeg.exe"),
            &[],
            &["-version"],
            false,
        );
        assert_eq!(info.status, ToolStatus::Broken);
    }

    #[cfg(windows)]
    #[test]
    fn detect_with_valid_override_probes_binary() {
        // cmd.exe همیشه روی ویندوز هست؛ پروب واقعی اجرا می‌شود
        let info = detect(
            "ffmpeg",
            Some("C:\\Windows\\System32\\cmd.exe"),
            &[],
            &["/c", "echo", "ok"],
            false,
        );
        assert_eq!(info.status, ToolStatus::Found);
        assert!(info.version.as_deref().unwrap_or_default().contains("ok"));
    }

    #[test]
    fn usage_detection() {
        assert!(looks_like_usage("Usage: sub-rs [OPTIONS]"));
        assert!(looks_like_usage("Options:\n  --help"));
        assert!(!looks_like_usage("ffmpeg version 8.0"));
    }

    /// تست زنده روی این سیستم (فقط با `cargo test -- --ignored`).
    /// ابزارهای واقعی این ماشین: ffmpeg، 7-Zip، sub-rs، ffs همه نصب‌اند.
    #[test]
    #[ignore]
    fn live_detect_finds_installed_tools() {
        let overrides = ToolPathOverrides::default();
        let results = detect_all(&overrides);
        for t in &results {
            println!(
                "{} -> {:?} {} ({})",
                t.name,
                t.status,
                t.version.as_deref().unwrap_or("-"),
                t.path.as_deref().unwrap_or("-")
            );
        }
        let status = |id: &str| results.iter().find(|t| t.id == id).map(|t| t.status);
        assert_eq!(status("ffmpeg"), Some(ToolStatus::Found));
        assert_eq!(status("sevenZip"), Some(ToolStatus::Found));
        assert_eq!(status("subsource"), Some(ToolStatus::Found));
        assert_eq!(status("ffsubsync"), Some(ToolStatus::Found));
    }
}
