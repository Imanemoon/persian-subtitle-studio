//! Fake 7-Zip برای تست‌های یکپارچه — بدون وابستگی به 7z واقعی.
//!
//! هر «آرشیو» یک فایل است که با magic واقعی ZIP/RAR شروع می‌شود و بعد از آن
//! یک manifest متنی دارد. بنابراین تشخیص magic در کد اصلی واقعی است.
//!
//! Manifest:
//! ```text
//! F|path|size|attrs|symlink|hardlink|reparse|hex_content|flags
//! D|path
//! ```
//!
//! modes (از فایل `.fake_7z_mode` کنار آرشیو یا env `FAKE_7Z_MODE`، جداشده با
//! کاما/خط جدید):
//! - `list-warning` / `list-fail`
//! - `extract-warning` / `extract-fail` (فایل‌ها را می‌نویسد، سپس ۲)
//! - `partial` (اعضای با flag=SKIP را رد می‌کند و با ۱ خارج می‌شود)
//! - `slow` (۳۰ ثانیه تأخیر برای تست لغو)

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::Duration;

fn manifest_start(data: &[u8]) -> usize {
    const RAR5: &[u8] = &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00];
    const RAR4: &[u8] = &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00];
    if data.starts_with(RAR5) {
        8
    } else if data.starts_with(RAR4) {
        7
    } else if data.starts_with(&[0x50, 0x4B, 0x03, 0x04])
        || data.starts_with(&[0x50, 0x4B, 0x05, 0x06])
        || data.starts_with(&[0x50, 0x4B, 0x07, 0x08])
    {
        4
    } else {
        0
    }
}

fn modes(archive: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(value) = env::var("FAKE_7Z_MODE") {
        for part in value.split([',', '\n']) {
            let part = part.trim();
            if !part.is_empty() {
                out.push(part.to_ascii_lowercase());
            }
        }
    }
    let mut dirs = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        dirs.push(cwd);
    }
    if let Some(parent) = archive.parent() {
        dirs.push(parent.to_path_buf());
    }
    for dir in dirs {
        let marker = dir.join(".fake_7z_mode");
        if let Ok(text) = fs::read_to_string(marker) {
            for line in text.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    out.push(line.to_ascii_lowercase());
                }
            }
        }
    }
    out
}

fn has_mode(modes: &[String], mode: &str) -> bool {
    modes.iter().any(|m| m == mode)
}

fn archive_arg(args: &[String]) -> Option<PathBuf> {
    if let Some(pos) = args.iter().position(|a| a == "--") {
        return args.get(pos + 1).map(PathBuf::from);
    }
    args.iter()
        .rev()
        .find(|a| !a.starts_with('-') && *a != "l" && *a != "x")
        .map(PathBuf::from)
}

fn out_dir_arg(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .find_map(|arg| arg.strip_prefix("-o").map(PathBuf::from))
}

fn read_manifest(archive: &Path) -> Vec<String> {
    let data = match fs::read(archive) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("fake 7z: cannot read archive: {e}");
            process::exit(2);
        }
    };
    let start = manifest_start(&data);
    String::from_utf8_lossy(&data[start..])
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_file(line: &str) -> Vec<String> {
    let mut parts: Vec<String> = line.split('|').map(str::to_string).collect();
    while parts.len() < 9 {
        parts.push(String::new());
    }
    parts
}

fn listing(archive: &Path, lines: &[String]) {
    println!("Path = {}", archive.display());
    println!("Type = fake");
    println!();
    println!("----------");
    for line in lines {
        if let Some(rest) = line.strip_prefix("F|") {
            let p = split_file(&format!("F|{rest}"));
            println!("Path = {}", p[1]);
            println!("Size = {}", p[2]);
            println!("Folder = -");
            if !p[3].is_empty() {
                println!("Attributes = {}", p[3]);
            }
            if !p[4].is_empty() {
                println!("Symbolic Link = {}", p[4]);
            }
            if !p[5].is_empty() {
                println!("Hard Link = {}", p[5]);
            }
            if !p[6].is_empty() {
                println!("Reparse = {}", p[6]);
            }
            println!();
        } else if let Some(rest) = line.strip_prefix("D|") {
            println!("Path = {rest}");
            println!("Size = 0");
            println!("Folder = +");
            println!();
        }
    }
}

fn decode_hex(hex: &str) -> Vec<u8> {
    let bytes = hex.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16).unwrap_or(0) as u8;
        let lo = (bytes[i + 1] as char).to_digit(16).unwrap_or(0) as u8;
        out.push((hi << 4) | lo);
        i += 2;
    }
    out
}

fn extract(out_dir: &Path, lines: &[String], partial: bool) -> (bool, bool) {
    // (wrote_any, skipped_any)
    let mut wrote_any = false;
    let mut skipped_any = false;
    for line in lines {
        if let Some(rest) = line.strip_prefix("D|") {
            let target = out_dir.join(rest);
            let _ = fs::create_dir_all(target);
        } else if let Some(rest) = line.strip_prefix("F|") {
            let p = split_file(&format!("F|{rest}"));
            let flags = p[8].to_ascii_lowercase();
            if partial && flags.split(',').any(|f| f.trim() == "skip") {
                skipped_any = true;
                continue;
            }
            let target = out_dir.join(&p[1]);
            if let Some(parent) = target.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let content = decode_hex(&p[7]);
            if fs::write(&target, content).is_ok() {
                wrote_any = true;
            }
        }
    }
    (wrote_any, skipped_any)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().cloned() else {
        eprintln!("fake 7z: missing command");
        process::exit(2);
    };
    let Some(archive) = archive_arg(&args) else {
        eprintln!("fake 7z: missing archive");
        process::exit(2);
    };
    let modes = modes(&archive);

    if has_mode(&modes, "slow") {
        thread::sleep(Duration::from_secs(30));
    }

    match command.as_str() {
        "l" => {
            if has_mode(&modes, "list-fail") {
                eprintln!("fake 7z: simulated listing failure");
                process::exit(2);
            }
            let lines = read_manifest(&archive);
            listing(&archive, &lines);
            if has_mode(&modes, "list-warning") {
                eprintln!("fake 7z: simulated listing warning");
                process::exit(1);
            }
        }
        "x" => {
            let Some(out_dir) = out_dir_arg(&args) else {
                eprintln!("fake 7z: missing -o output directory");
                process::exit(2);
            };
            if fs::create_dir_all(&out_dir).is_err() {
                eprintln!("fake 7z: cannot create output directory");
                process::exit(2);
            }
            let lines = read_manifest(&archive);
            let partial = has_mode(&modes, "partial");
            let (wrote_any, skipped_any) = extract(&out_dir, &lines, partial);
            if has_mode(&modes, "extract-fail") {
                // استخراج جزئی واقعی: فایل‌ها نوشته شده‌اند اما کد ۲ برمی‌گردد.
                let _ = wrote_any;
                eprintln!("fake 7z: simulated extraction failure");
                process::exit(2);
            }
            if has_mode(&modes, "extract-warning") || (partial && skipped_any) {
                eprintln!("fake 7z: simulated extraction warning");
                process::exit(1);
            }
            if partial && !skipped_any && !wrote_any {
                eprintln!("fake 7z: nothing extracted");
                process::exit(2);
            }
        }
        _ => {
            eprintln!("fake 7z: unsupported command {command}");
            process::exit(2);
        }
    }
}
