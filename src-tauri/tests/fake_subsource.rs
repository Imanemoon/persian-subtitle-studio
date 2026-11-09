//! Fake executable برای تست واقعی CLI adapter

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::Duration;

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

    // 2. بررسی فایل marker اختصاصی (.fake_mode) در پوشه کاری جاری یا پوشه --directory
    let mut check_dirs = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        check_dirs.push(cwd);
    }
    if let Some(pos) = args.iter().position(|a| a == "-d" || a == "--directory") {
        if let Some(dir) = args.get(pos + 1) {
            check_dirs.push(PathBuf::from(dir));
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

    // موفقیت پیش‌فرض
    println!("Downloaded 3 subtitles successfully");
    println!("Processing completed in 2.5 seconds");
    process::exit(0);
}
