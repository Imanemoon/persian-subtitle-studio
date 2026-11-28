//! گزارش JSON ماندگار هر batch — اتمیک، بدون بازنویسی، بدون secret.
//!
//! قراردادها:
//! - مقصد: `<data_dir>/runs/run-<run_id>.json` — run_id یکتای بک‌اند؛ در
//!   برخورد غیرمنتظره، پسوند توکن یکتا می‌گیرد (هرگز بازنویسی نمی‌شود).
//! - نوشتن با فایل tmp + rename اتمیک؛ شکست فقط warning است و هرگز نتیجهٔ
//!   پردازش را خراب یا منکر نمی‌کند (فراخواننده در engine.rs).
//! - محتوا: فقط مسیرها/وضعیت‌ها/دلایل — کلید API و هر secret دیگری هرگز
//!   وارد summary نمی‌شود (قرارداد کل کدبیس) پس وارد گزارش هم نمی‌شود.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::types::PipelineSummary;

/// نسخهٔ schema گزارش — برای سازگاری آینده.
pub(crate) const REPORT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BatchReport {
    pub schema_version: u32,
    pub run_id: String,
    /// آیا این run در حالت خودکار اجرا شده؟
    pub auto_mode: bool,
    /// زمان تولید گزارش (epoch ms)
    pub generated_at_ms: u64,
    /// ورودی‌های run — مسیرهای ویدیو
    pub inputs: Vec<String>,
    /// خلاصهٔ کامل run — شمارش‌ها، آیتم‌ها با outcome نهایی، تصمیم‌های خودکار
    /// با دلایل امتیازدهی، رکورد تلاش‌های سینک، خطاها و staging paths.
    pub summary: PipelineSummary,
}

/// نوشتن گزارش batch — مسیر فایل نهایی یا خطا؛ هیچ بازنویسی‌ای ممکن نیست.
pub(crate) fn write_batch_report(
    dir: &Path,
    summary: &PipelineSummary,
    auto_mode: bool,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("ساخت پوشهٔ گزارش‌ها: {e}"))?;
    let generated_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let inputs: Vec<String> = summary
        .items
        .iter()
        .map(|item| item.video_path.clone())
        .collect();
    let report = BatchReport {
        schema_version: REPORT_SCHEMA_VERSION,
        run_id: summary.run_id.clone(),
        auto_mode,
        generated_at_ms,
        inputs,
        summary: summary.clone(),
    };
    let data = serde_json::to_vec_pretty(&report).map_err(|e| format!("سریال‌سازی گزارش: {e}"))?;

    // نام یکتا از run_id؛ برخورد (که هرگز انتظار نمی‌رود) → پسوند توکن یکتا
    let mut target = dir.join(format!("run-{}.json", summary.run_id));
    if target.exists() {
        target = dir.join(format!(
            "run-{}-{}.json",
            summary.run_id,
            crate::subtitle::extraction::unique_probe_token()
        ));
    }
    let tmp = dir.join(format!(
        ".{}.tmp-{}",
        target
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "report".into()),
        crate::subtitle::extraction::unique_probe_token()
    ));
    std::fs::write(&tmp, &data).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("نوشتن گزارش موقت: {e}")
    })?;
    if let Err(err) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("جایگزینی گزارش: {err}"));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::types::{
        ItemOutcome, PipelineCounts, PipelineItemSnapshot, PipelineStage, PipelineStatus,
        PipelineSummary, StageStatus,
    };

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "subsync-report-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_summary(run_id: &str) -> PipelineSummary {
        PipelineSummary {
            run_id: run_id.into(),
            status: PipelineStatus::Completed,
            cancelled: false,
            counts: PipelineCounts {
                synced: 1,
                needs_review: 1,
                skipped: 1,
                ..Default::default()
            },
            items: vec![PipelineItemSnapshot {
                video_path: "D:/Media/S01E01.mkv".into(),
                video_file_name: "S01E01.mkv".into(),
                stage: PipelineStage::Done,
                stage_status: StageStatus::Done,
                outcome: Some(ItemOutcome::NeedsReview {
                    reason: "دو کاندیدای هم‌امتیاز".into(),
                }),
                artifacts: Default::default(),
            }],
            invalid_selections: vec![],
            staged_paths: vec![],
            warnings: vec![],
            key_store_error: None,
            duration_ms: 1234,
            ephemeral: true,
            profile_id: Some("builtin-default".into()),
            profile_name: Some("پیش‌فرض".into()),
            naming_template: "{name}".into(),
        }
    }

    #[test]
    fn report_is_written_atomically_and_parses_back() {
        let dir = unique_dir("ok");
        let summary = sample_summary("run-test-1");
        let path = write_batch_report(&dir, &summary, true).expect("گزارش");
        assert!(path.is_file());
        let bytes = std::fs::read(&path).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["runId"], "run-test-1");
        assert_eq!(value["autoMode"], true);
        assert_eq!(value["inputs"][0], "D:/Media/S01E01.mkv");
        assert_eq!(value["summary"]["counts"]["needsReview"], 1);
        assert_eq!(
            value["summary"]["items"][0]["outcome"]["kind"],
            "needsReview"
        );
        // فایل tmp باقی نمانده باشد
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "tmp باقی مانده: {leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_collision_never_overwrites() {
        let dir = unique_dir("collision");
        let summary = sample_summary("run-same");
        let first = write_batch_report(&dir, &summary, false).expect("اول");
        let second = write_batch_report(&dir, &summary, true).expect("دوم");
        assert_ne!(first, second, "برخورد باید فایل جدید بسازد نه بازنویسی");
        assert!(first.is_file() && second.is_file());
        // محتوای اول دست‌نخورده مانده
        let first_value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&first).unwrap()).unwrap();
        assert_eq!(first_value["autoMode"], false);
        let second_value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&second).unwrap()).unwrap();
        assert_eq!(second_value["autoMode"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn report_write_failure_returns_error_not_panic() {
        // مقصد به‌جای پوشه، یک فایل است — create_dir_all شکست می‌خورد
        let dir = unique_dir("failure");
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"not a dir").unwrap();
        let err = write_batch_report(&blocker, &sample_summary("run-x"), false);
        assert!(err.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
