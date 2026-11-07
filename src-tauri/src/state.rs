use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::models::{ScanSummary, VideoEntry};
use crate::settings::Settings;

/// وضعیت عملیات جاری — عمر پرچم لغو به ازای هر عملیات مدیریت می‌شود تا
/// لغوِ یک عملیات هرگز روی عملیات دیگر اثر نگذارد.
#[derive(Default)]
pub struct OpsRegistry {
    /// عملیات گروهی در جریان (حداکثر یکی) + پرچم لغو مخصوص همان عملیات.
    pub batch_cancel: Option<Arc<AtomicBool>>,
    /// ویدیوهایی با عملیات استخراج/بررسی تکی در جریان → پرچم لغو مخصوص همان عملیات.
    pub single_extracts: HashMap<String, Arc<AtomicBool>>,
}

impl OpsRegistry {
    /// گرفتن اسلات عملیات تکی برای یک ویدیو.
    /// هم‌زمانی: با عملیات گروهی و با عملیات تکی دیگر روی همان ویدیو تضاد دارد.
    pub fn acquire_single(&mut self, video_path: &str) -> Result<Arc<AtomicBool>, String> {
        if self.batch_cancel.is_some() {
            return Err(
                "یک عملیات گروهی در حال اجراست؛ صبر کنید یا ابتدا آن را متوقف کنید.".into(),
            );
        }
        if self.single_extracts.contains_key(video_path) {
            return Err("عملیات دیگری روی همین ویدیو در جریان است.".into());
        }
        // پرچم کاملاً تازه — پرچم باقی‌مانده از عملیات قبلی معنا ندارد
        let flag = Arc::new(AtomicBool::new(false));
        self.single_extracts
            .insert(video_path.to_string(), flag.clone());
        Ok(flag)
    }

    pub fn release_single(&mut self, video_path: &str) {
        self.single_extracts.remove(video_path);
    }

    /// ثبت عملیات گروهی؛ در صورت وجود عملیات گروهی فعال خطا می‌دهد.
    pub fn start_batch(&mut self) -> Result<Arc<AtomicBool>, String> {
        if self.batch_cancel.is_some() {
            return Err("یک عملیات گروهی در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
        }
        let flag = Arc::new(AtomicBool::new(false));
        self.batch_cancel = Some(flag.clone());
        Ok(flag)
    }

    pub fn end_batch(&mut self) {
        self.batch_cancel = None;
    }

    /// لغو همهٔ عملیات جاری (گروهی + تکی‌ها). خروجی: آیا چیزی در جریان بود؟
    pub fn cancel_all(&mut self) -> bool {
        let mut any = false;
        if let Some(b) = &self.batch_cancel {
            b.store(true, std::sync::atomic::Ordering::SeqCst);
            any = true;
        }
        for f in self.single_extracts.values() {
            f.store(true, std::sync::atomic::Ordering::SeqCst);
            any = true;
        }
        any
    }
}

/// وضعیت سراسری برنامه — از طریق `app.state::<AppState>()` در دسترس است.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub library: Mutex<Vec<VideoEntry>>,
    pub last_scan: Mutex<Option<ScanSummary>>,
    pub scanning: AtomicBool,
    pub cancel_scan: AtomicBool,
    pub ops: Mutex<OpsRegistry>,
}

/// مسیرهای دائمی برنامه (تنظیمات و لاگ).
#[derive(Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_rejects_batch_and_duplicate_single() {
        let mut ops = OpsRegistry::default();

        // اسلات تکی تازه — پرچم باید false باشد
        let f1 = ops.acquire_single("v1").unwrap();
        assert!(!f1.load(std::sync::atomic::Ordering::SeqCst));

        // ویدیوی دیگر مشکل ندارد
        let f2 = ops.acquire_single("v2").unwrap();
        assert!(!f2.load(std::sync::atomic::Ordering::SeqCst));

        // همان ویدیو → رد
        assert!(ops.acquire_single("v1").is_err());

        // عملیات گروهی فعال → همهٔ اسلات‌های تکی جدید رد می‌شوند
        ops.end_batch(); // noop
        assert!(ops.start_batch().is_ok());
        assert!(ops.acquire_single("v3").is_err());

        // لغو همه: پرچم گروهی و تکی‌ها true می‌شوند
        assert!(ops.cancel_all());
        assert!(f1.load(std::sync::atomic::Ordering::SeqCst));
        assert!(f2.load(std::sync::atomic::Ordering::SeqCst));

        // پایان گروهی → بعد از آزادسازی اسلات‌های لغوشده (کار wrapper پس از
        // پایان هر عملیات)، اسلات جدید مجاز و پرچمش کاملاً تازه است
        ops.end_batch();
        ops.release_single("v1");
        ops.release_single("v2");
        let f3 = ops.acquire_single("v1").unwrap();
        assert!(!f3.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn release_frees_slot() {
        let mut ops = OpsRegistry::default();
        let _f = ops.acquire_single("v1").unwrap();
        ops.release_single("v1");
        assert!(ops.acquire_single("v1").is_ok());
    }
}
