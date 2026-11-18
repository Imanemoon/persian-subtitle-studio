//! تست‌های یکپارچه برای SubSource CLI adapter با fake executable

use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use subsync_studio_lib::subsource::{
    BatchOptions, CancellationToken, SubSourceCli, SubSourceError,
};

/// ساخت پوشه موقت منحصربه‌فرد برای هر تست تا در اجرای موازی هیچ تداخلی رخ ندهد.
fn unique_test_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "subsync-int-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("ایجاد پوشه تستی منحصربه‌فرد");
    dir
}

/// دریافت مسیر fake executable.
/// ساخت با rustc باید بین تست‌های موازی همگام شود — دو link هم‌زمان روی
/// یک فایل خروجی، LNK1104 (قفل فایل) می‌دهد که روی CI اجرای ۸ تایی را
/// شکست می‌دهد. Mutex + double-check: سازنده دقیقاً یک‌بار اجرا می‌شود.
fn get_fake_exe() -> PathBuf {
    use std::sync::{Mutex, OnceLock};
    static FAKE_BUILD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let lock = FAKE_BUILD_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let target_dir = PathBuf::from(manifest_dir).join("target").join("debug");

    #[cfg(windows)]
    let exe_name = "fake_subsource.exe";
    #[cfg(not(windows))]
    let exe_name = "fake_subsource";

    let exe_path = target_dir.join(exe_name);

    if !exe_path.exists() {
        let source_path = PathBuf::from(manifest_dir)
            .join("tests")
            .join("fake_subsource.rs");
        let status = std::process::Command::new("rustc")
            .arg(&source_path)
            .arg("-o")
            .arg(&exe_path)
            .arg("--edition")
            .arg("2021")
            .status();

        if status.map(|s| s.success()).unwrap_or(false) && exe_path.exists() {
            return exe_path;
        }

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

    let test_dir = unique_test_dir("success");
    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts).expect("Batch operation failed");
    assert!(result.success);
    assert_eq!(result.exit_code, 0);
    assert!(result.stdout.contains("Downloaded"));

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_auth_error_detection() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("auth-err");
    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "invalid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(matches!(
        result,
        Err(SubSourceError::AuthenticationRequired(_))
    ));

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_network_error_detection() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("net-err");
    // استفاده از فایل marker اختصاصی برای تعیین حالت fake executable
    std::fs::write(test_dir.join(".fake_mode"), "simulate-network-error").unwrap();

    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(matches!(result, Err(SubSourceError::CliExitCode { .. })));

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_cancellation_support() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("cancel");
    std::fs::write(test_dir.join(".fake_mode"), "slow").unwrap();

    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let cancel_token = CancellationToken::new();
    let cancel_clone = cancel_token.clone();

    // لغو بعد از 1 ثانیه
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(1));
        cancel_clone.cancel();
    });

    let result = cli.run_batch_with_cancellation(&opts, &cancel_token);
    assert!(matches!(result, Err(SubSourceError::ProcessTerminated(_))));

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_large_stdout_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("large-stdout");
    std::fs::write(test_dir.join(".fake_mode"), "large-output").unwrap();

    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts).expect("Should handle large output");
    assert!(result.stdout.lines().count() >= 1000);

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_large_stderr_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("large-stderr");
    std::fs::write(test_dir.join(".fake_mode"), "large-stderr").unwrap();

    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "valid-key-not-invalid".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(result.is_err(), "Should fail with large stderr");

    match result {
        Err(SubSourceError::CliExitCode { stderr, .. }) => {
            assert!(stderr.lines().count() >= 100, "Should capture stderr lines");
        }
        Err(SubSourceError::AuthenticationRequired(_)) => {}
        Err(e) => panic!("Unexpected error type: {:?}", e),
        Ok(_) => panic!("Should not succeed"),
    }

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_credential_redaction() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("redact");
    std::fs::write(test_dir.join(".fake_mode"), "leak-credentials").unwrap();

    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "secret-key-12345".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts).expect("Should succeed");

    // خروجی stdout و stderr نباید حاوی API key واقعی باشد
    assert!(!result.stdout.contains("secret-key-12345"));
    assert!(!result.stderr.contains("secret-key-12345"));

    // باید redaction واقعی وجود داشته باشد
    assert!(result.stdout.contains("<REDACTED_API_KEY>"));
    assert!(result.stderr.contains("<REDACTED_API_KEY>"));

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_credential_redaction_in_error() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("redact-err");
    std::fs::write(test_dir.join(".fake_mode"), "leak-credentials-error").unwrap();

    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "secret-key-67890".into(),
        top: 1,
        ..Default::default()
    };

    let err = cli.run_batch(&opts).expect_err("Should fail");

    // Display خطا نباید کلید را افشا کند
    let display = err.to_string();
    assert!(!display.contains("secret-key-67890"));

    match err {
        SubSourceError::CliExitCode { stderr, .. } => {
            assert!(!stderr.contains("secret-key-67890"));
            assert!(stderr.contains("<REDACTED_API_KEY>"));
        }
        other => panic!("Unexpected error type: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&test_dir);
}

#[test]
fn test_unicode_path_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    // ساخت پوشه با نام Unicode و شناسه منحصربه‌فرد
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let base = std::env::temp_dir();
    let unicode_dir = base.join(format!("تست_فارسی_سریال_{}_{nanos}", std::process::id()));
    std::fs::create_dir_all(&unicode_dir).expect("Failed to create unicode dir");

    let opts = BatchOptions {
        directory: unicode_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(result.is_ok());

    let _ = std::fs::remove_dir_all(&unicode_dir);
}

#[test]
fn test_space_in_path_handling() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let base = std::env::temp_dir();
    let space_dir = base.join(format!(
        "test dir with spaces_{}_{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&space_dir).expect("Failed to create dir with spaces");

    let opts = BatchOptions {
        directory: space_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

    let result = cli.run_batch(&opts);
    assert!(result.is_ok());

    let _ = std::fs::remove_dir_all(&space_dir);
}

#[test]
fn test_process_kill_and_wait() {
    let exe = get_fake_exe();
    let cli = SubSourceCli::new(exe.to_str().unwrap()).expect("CLI creation failed");

    let test_dir = unique_test_dir("kill-wait");
    std::fs::write(test_dir.join(".fake_mode"), "slow").unwrap();

    let opts = BatchOptions {
        directory: test_dir.to_str().unwrap().to_string(),
        api_key: "valid-key".into(),
        top: 1,
        ..Default::default()
    };

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

    let _ = std::fs::remove_dir_all(&test_dir);
}
