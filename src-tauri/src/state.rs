use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::archive::provenance::ProvenanceRegistry;
use crate::models::{ScanSummary, VideoEntry};
use crate::settings::Settings;

/// وضعیت عملیات جاری — عمر پرچم لغو به ازای هر عملیات مدیریت می‌شود تا
/// لغوِ یک عملیات هرگز روی عملیات دیگر اثر نگذارد.
#[derive(Debug, Default)]
pub struct OpsRegistry {
    /// عملیات گروهی در جریان (حداکثر یکی) + پرچم لغو مخصوص همان عملیات.
    pub batch_cancel: Option<Arc<AtomicBool>>,
    /// ویدیوهایی با عملیات تکی در جریان (Analyze/Extract/Review/Promote)
    /// → پرچم لغو مخصوص همان عملیات.
    pub single_ops: HashMap<String, Arc<AtomicBool>>,
}

impl OpsRegistry {
    /// ثبت عملیات گروهی — فقط اگر هیچ عملیات تکی فعالی نباشد.
    /// بررسی و ثبت زیر یک قفل انجام می‌شود (بدون فاصلهٔ check-then-act).
    pub fn start_batch(&mut self) -> Result<Arc<AtomicBool>, String> {
        if self.batch_cancel.is_some() {
            return Err("یک عملیات گروهی در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
        }
        if !self.single_ops.is_empty() {
            return Err(format!(
                "{} عملیات تکی در جریان است؛ ابتدا آن‌ها را تمام یا متوقف کنید.",
                self.single_ops.len()
            ));
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
        for f in self.single_ops.values() {
            f.store(true, std::sync::atomic::Ordering::SeqCst);
            any = true;
        }
        any
    }
}

/// دستگیرهٔ عملیات تکی — با Drop، اسلات را از Registry آزاد می‌کند (RAII).
/// حتی در صورت panic یا خطای Thread، اسلات قفل نمی‌ماند.
///
/// پرچم لغو مخصوص همین عملیات است؛ از `.cancel` در طول اجرا استفاده کنید.
#[derive(Debug)]
pub struct SingleOpGuard {
    registry: Arc<Mutex<OpsRegistry>>,
    video_path: String,
    pub cancel: Arc<AtomicBool>,
}

impl SingleOpGuard {
    /// آزادسازی صریح (Drop هم همین کار را می‌کند).
    pub fn release(self) {
        // drop(self) رخ می‌دهد و اسلات آزاد می‌شود
    }
}

impl Drop for SingleOpGuard {
    fn drop(&mut self) {
        self.registry
            .lock()
            .unwrap()
            .single_ops
            .remove(&self.video_path);
    }
}

/// رزرو اتمیک اسلات عملیات تکی برای یک ویدیو.
/// قوانین (تحت یک قفل):
/// - حین عملیات گروهی → رد
/// - حین هر عملیات تکی دیگر روی همان ویدیو → رد
/// - روی ویدیوی دیگر → مجاز
pub fn acquire_single_op(
    registry: &Arc<Mutex<OpsRegistry>>,
    video_path: &str,
) -> Result<SingleOpGuard, String> {
    let mut ops = registry.lock().unwrap();
    if ops.batch_cancel.is_some() {
        return Err("یک عملیات گروهی در حال اجراست؛ صبر کنید یا ابتدا آن را متوقف کنید.".into());
    }
    if ops.single_ops.contains_key(video_path) {
        return Err("عملیات دیگری روی همین ویدیو در جریان است.".into());
    }
    // پرچم کاملاً تازه — پرچم باقی‌مانده از عملیات قبلی معنا ندارد
    let cancel = Arc::new(AtomicBool::new(false));
    ops.single_ops
        .insert(video_path.to_string(), cancel.clone());
    drop(ops);
    Ok(SingleOpGuard {
        registry: registry.clone(),
        video_path: video_path.to_string(),
        cancel,
    })
}

/// دستگیرهٔ عملیات گروهی — با Drop، اسلات را از Registry آزاد می‌کند (RAII).
/// حتی در صورت panic یا خطای Thread، اسلات قفل نمی‌ماند.
///
/// پرچم لغو مخصوص همین عملیات گروهی است؛ همان `AtomicBool` که در Registry
/// ثبت شده و `cancel_all` روی آن عمل می‌کند.
#[derive(Debug)]
pub struct BatchOpGuard {
    registry: Arc<Mutex<OpsRegistry>>,
    pub cancel: Arc<AtomicBool>,
}

impl BatchOpGuard {
    /// آزادسازی صریح (Drop هم همین کار را می‌کند).
    pub fn release(self) {
        // drop(self) رخ می‌دهد و اسلات آزاد می‌شود
    }
}

impl Drop for BatchOpGuard {
    fn drop(&mut self) {
        self.registry.lock().unwrap().end_batch();
    }
}

/// رزرو اتمیک اسلات عملیات گروهی با گارد RAII — همان قوانین `start_batch`
/// (تعارض با عملیات گروهی دیگر و همهٔ عملیات‌های تکی رد می‌شود).
pub fn acquire_batch_op(registry: &Arc<Mutex<OpsRegistry>>) -> Result<BatchOpGuard, String> {
    let cancel = registry.lock().unwrap().start_batch()?;
    Ok(BatchOpGuard {
        registry: registry.clone(),
        cancel,
    })
}

/// وضعیت سراسری برنامه — از طریق `app.state::<AppState>()` در دسترس است.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub library: Mutex<Vec<VideoEntry>>,
    pub last_scan: Mutex<Option<ScanSummary>>,
    pub scanning: AtomicBool,
    pub cancel_scan: AtomicBool,
    pub ops: Arc<Mutex<OpsRegistry>>,
    /// Provenance سمت سرور برای stagingهای ساخته‌شده توسط دانلود گروهی.
    pub archive_provenance: Mutex<ProvenanceRegistry>,
}

/// مسیرهای دائمی برنامه (تنظیمات و لاگ).
#[derive(Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    fn registry() -> Arc<Mutex<OpsRegistry>> {
        Arc::new(Mutex::new(OpsRegistry::default()))
    }

    /// سناریوی الزامی Audit: Single فعال → Batch رد؛ آزادسازی → Batch موفق؛
    /// Batch فعال → Single رد.
    #[test]
    fn batch_and_single_are_mutually_exclusive() {
        let reg = registry();

        // ۱) Single فعال
        let g1 = acquire_single_op(&reg, "v1").unwrap();
        assert!(!g1.cancel.load(Ordering::SeqCst));
        // روی ویدیوی دیگر هم Single دوم مجاز است
        let g2 = acquire_single_op(&reg, "v2").unwrap();

        // ۲) Batch در حالت Single فعال → رد
        let err = reg.lock().unwrap().start_batch().unwrap_err();
        assert!(err.contains("تکی"), "{err}");

        // ۳) آزادسازی Singleها
        drop(g1);
        drop(g2);
        assert!(reg.lock().unwrap().single_ops.is_empty());

        // ۴) Batch اکنون موفق است
        let batch = reg.lock().unwrap().start_batch().unwrap();
        assert!(!batch.load(Ordering::SeqCst));

        // ۵) در حالت Batch فعال → Single رد
        let err = acquire_single_op(&reg, "v3").unwrap_err();
        assert!(err.contains("گروهی"), "{err}");

        // لغو همه → پرچم گروهی true؛ پایان گروهی → Single با پرچم تازه مجاز
        assert!(reg.lock().unwrap().cancel_all());
        assert!(batch.load(Ordering::SeqCst));
        reg.lock().unwrap().end_batch();
        let g3 = acquire_single_op(&reg, "v3").unwrap();
        assert!(!g3.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn same_video_single_is_rejected_other_video_is_allowed() {
        let reg = registry();
        let _g = acquire_single_op(&reg, "v1").unwrap();
        let err = acquire_single_op(&reg, "v1").unwrap_err();
        assert!(err.contains("همین ویدیو"), "{err}");
        // ویدیوی دیگر → مجاز
        let _g2 = acquire_single_op(&reg, "v2").unwrap();
    }

    /// RAII: panic در حین عملیات باید اسلات را آزاد کند.
    #[test]
    fn panic_releases_slot() {
        let reg = registry();
        let handle = {
            let reg = reg.clone();
            std::thread::spawn(move || {
                let _g = acquire_single_op(&reg, "v").unwrap();
                panic!("شبیه‌سازی خطای غیرمنتظره در عملیات");
            })
        };
        assert!(handle.join().is_err());
        // اسلات آزاد شده باشد
        assert!(acquire_single_op(&reg, "v").is_ok());
    }

    /// گارد گروهی (RAII): Drop اسلات را آزاد می‌کند؛ لغو از cancel_all روی
    /// همان پرچم ثبت‌شده اثر می‌گذارد؛ panic هم اسلات را آزاد می‌کند.
    #[test]
    fn batch_guard_releases_slot_on_drop_and_panic() {
        let reg = registry();

        // ۱) acquire → batch فعال؛ تکی رد می‌شود
        let guard = acquire_batch_op(&reg).unwrap();
        assert!(acquire_single_op(&reg, "v").is_err());
        assert!(reg.lock().unwrap().start_batch().is_err());

        // ۲) cancel_all → همان پرچمِ گارد set می‌شود
        assert!(reg.lock().unwrap().cancel_all());
        assert!(guard.cancel.load(Ordering::SeqCst));

        // ۳) Drop → اسلات آزاد و پرچم تازه برای عملیات بعدی
        drop(guard);
        let g2 = acquire_batch_op(&reg).unwrap();
        assert!(!g2.cancel.load(Ordering::SeqCst));
        drop(g2);

        // ۴) panic در حین عملیات گروهی → اسلات آزاد
        let handle = {
            let reg = reg.clone();
            std::thread::spawn(move || {
                let _g = acquire_batch_op(&reg).unwrap();
                panic!("شبیه‌سازی خطای غیرمنتظره در عملیات گروهی");
            })
        };
        assert!(handle.join().is_err());
        assert!(acquire_batch_op(&reg).is_ok());
    }
}
