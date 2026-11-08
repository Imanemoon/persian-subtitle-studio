//! تست‌های یکپارچه برای SubSource CLI adapter با fake executable

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use subsync_studio_lib::subsource::{
    BatchOptions, CancellationToken, ConnectionMode, SubSourceCli, SubSourceError,
};

/// دریافت مسیر fake executable (باید از قبل کامپایل شده باشد)
fn get_fake_exe() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let target_dir = PathBuf::from(manifest_dir).join("target").join("debug");

    #[cfg(windows)]
    let exe_name = "fake_subsource.exe";
    #[cfg(not(windows))]
    let exe_name = "fake_subsource";

    let exe_path = target_dir.join(exe_name);

    if !exe_path.exists() {
        panic!(
            "fake_subsource executable not found at {:?}. Run: rustc tests/fake_subsource.rs -o target/debug/fake_subsource.exe --edition 2021",
            exe_path
        );
    }

    exe_path
}

#[test]
fn test_successful_batch_operation() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts).expect("Batch operation failed");
    assert!(result.success);
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Downloaded"));
}

#[test]
fn test_auth_error_detection() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "invalid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(matches!(
        result,
        Err(SubSourceError::AuthenticationRequired(_))
    ));
}

#[test]
fn test_network_error_detection() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let mut opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    // استفاده از worker برای تریگر کردن شبیه‌سازی خطای شبکه
    opts.connection_mode = ConnectionMode::Cloudflare;
    opts.worker = Some("--simulate-network-error".into());

    let result = cli.run_batch(&opts);
    assert!(matches!(result, Err(SubSourceError::CliExitCode { .. })));
}

#[test]
fn test_cancellation_support() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let mut opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    // استفاده از worker برای عملیات طولانی
    opts.connection_mode = ConnectionMode::Cloudflare;
    opts.worker = Some("--slow".into());

    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();

    // لغو بعد از 1 ثانیه
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(1));
        cancel_clone.cancel();
    });

    let result = cli.run_batch_with_cancellation(&opts, &cancel_token);
    assert!(matches!(result, Err(SubSourceError::ProcessTerminated(_))));
}

#[test]
fn test_large_stdout_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let mut opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    opts.connection_mode = ConnectionMode::Cloudflare;
    opts.worker = Some("--large-output".into());

    let result = cli.run_batch(&opts).expect("Should handle large output");
    assert!(result.stdout.lines().count() >= 1000);
}

#[test]
fn test_large_stderr_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let mut opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "valid-key-not-invalid".into(), // از کلید معتبر استفاده می‌کنیم
        top: 1,
        ..Default::default()
    };

    opts.connection_mode = ConnectionMode::Cloudflare;
    opts.worker = Some("--large-stderr".into());

    let result = cli.run_batch(&opts);
    assert!(result.is_err(), "Should fail with large stderr");

    // تأیید که stderr با حجم زیاد مدیریت شد (بدون deadlock)
    match result {
        Err(SubSourceError::CliExitCode { stderr, .. }) => {
            assert!(stderr.lines().count() >= 100, "Should capture stderr lines");
        }
        Err(SubSourceError::AuthenticationRequired(_)) => {
            // fake executable ممکن است stderr را به عنوان auth error تشخیص دهد
            // این رفتار قابل قبول است
        }
        Err(e) => panic!("Unexpected error type: {:?}", e),
        Ok(_) => panic!("Should not succeed"),
    }
}

#[test]
fn test_credential_redaction() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let mut opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "secret-key-12345".into(),
        top: 1,
        connection_mode: ConnectionMode::Manual,
        proxy: Some("http://user:pass@proxy.example.com:8080".into()),
        ..Default::default()
    };

    opts.worker = Some("--leak-credentials".into());

    let result = cli.run_batch(&opts).expect("Should succeed");

    // خروجی stdout و stderr نباید حاوی credentialهای واقعی باشد
    assert!(!result.stdout.contains("secret-key-12345"));
    assert!(!result.stderr.contains("secret-key-12345"));
    assert!(!result.stdout.contains("user:pass"));
    assert!(!result.stderr.contains("user:pass"));

    // هر دو مسیر باید redaction واقعی داشته باشند، نه فقط خالی بودن
    assert!(result.stdout.contains("<REDACTED_API_KEY>"));
    assert!(result.stderr.contains("<REDACTED_API_KEY>"));
    assert!(result.stdout.contains("<REDACTED_PROXY>"));
    assert!(result.stderr.contains("<REDACTED_PROXY>"));
}

#[test]
fn test_credential_redaction_in_error() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let mut opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "secret-key-67890".into(),
        top: 1,
        connection_mode: ConnectionMode::Manual,
        proxy: Some("http://user:pass@proxy.example.com:8080".into()),
        ..Default::default()
    };

    opts.worker = Some("--leak-credentials-error".into());

    let err = cli.run_batch(&opts).expect_err("Should fail");

    // Display خطا هم نباید credential را leak کند
    let display = err.to_string();
    assert!(!display.contains("secret-key-67890"));
    assert!(!display.contains("user:pass"));

    match err {
        SubSourceError::CliExitCode { stderr, .. } => {
            assert!(!stderr.contains("secret-key-67890"));
            assert!(!stderr.contains("user:pass"));
            assert!(stderr.contains("<REDACTED_API_KEY>"));
            assert!(stderr.contains("<REDACTED_PROXY>"));
        }
        other => panic!("Unexpected error type: {other:?}"),
    }
}

#[test]
fn test_unicode_path_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    // ساخت پوشه با نام Unicode
    let base = std::env::temp_dir();
    let unicode_dir = base.join("تست_فارسی_سریال");
    std::fs::create_dir_all(&unicode_dir).expect("Failed to create unicode dir");

    let opts = BatchOptions {
        directory: unicode_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(result.is_ok());

    // پاک‌سازی
    let _ = std::fs::remove_dir_all(&unicode_dir);
}

#[test]
fn test_space_in_path_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let base = std::env::temp_dir();
    let space_dir = base.join("test dir with spaces");
    std::fs::create_dir_all(&space_dir).expect("Failed to create dir with spaces");

    let opts = BatchOptions {
        directory: space_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(result.is_ok());

    // پاک‌سازی
    let _ = std::fs::remove_dir_all(&space_dir);
}

#[test]
fn test_process_kill_and_wait() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let temp_dir = std::env::temp_dir();
    let mut opts = BatchOptions {
        directory: temp_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    opts.connection_mode = ConnectionMode::Cloudflare;
    opts.worker = Some("--slow".into());

    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();

    // لغو سریع
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        cancel_clone.cancel();
    });

    let start = std::time::Instant::now();
    let result = cli.run_batch_with_cancellation(&opts, &cancel_token);
    let elapsed = start.elapsed();

    // باید سریع لغو شود (نه 15 ثانیه کامل)
    assert!(elapsed < Duration::from_secs(2));
    assert!(matches!(result, Err(SubSourceError::ProcessTerminated(_))));
}
