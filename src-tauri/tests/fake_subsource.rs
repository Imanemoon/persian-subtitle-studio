//! Fake executable برای تست واقعی CLI adapter

use std::env;
use std::process;
use std::thread;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();

    // شبیه‌سازی رفتارهای مختلف بر اساس آرگومان‌ها
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

    // شبیه‌سازی خطای شبکه
    if args.contains(&"--simulate-network-error".to_string()) {
        eprintln!("Error: Connection timeout - network unreachable");
        process::exit(2);
    }

    // شبیه‌سازی عملیات طولانی برای تست cancellation
    if args.contains(&"--slow".to_string()) {
        eprintln!("Starting slow operation...");
        for i in 0..30 {
            println!("Progress: {}%", i * 10);
            thread::sleep(Duration::from_millis(500));
        }
        println!("Slow operation completed");
        process::exit(0);
    }

    // شبیه‌سازی خروجی حجیم
    if args.contains(&"--large-output".to_string()) {
        for i in 0..1000 {
            println!("Line {}: Some output data", i);
        }
        process::exit(0);
    }

    // شبیه‌سازی stderr حجیم
    if args.contains(&"--large-stderr".to_string()) {
        for i in 0..1000 {
            eprintln!("Error line {}: Some error data", i);
        }
        process::exit(1);
    }

    // شبیه‌سازی credential leakage (برای تست redaction)
    if let Some(pos) = args.iter().position(|a| a == "--api-key") {
        if let Some(key) = args.get(pos + 1) {
            if args.contains(&"--leak-credentials".to_string()) {
                println!("API Key used: {}", key);
                if let Some(proxy_pos) = args.iter().position(|a| a == "--proxy") {
                    if let Some(proxy) = args.get(proxy_pos + 1) {
                        println!("Proxy URL: {}", proxy);
                    }
                }
                process::exit(0);
            }
        }
    }

    // موفقیت پیش‌فرض
    println!("Downloaded 3 subtitles successfully");
    println!("Processing completed in 2.5 seconds");
    process::exit(0);
}
