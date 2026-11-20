use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::archive::provenance::ProvenanceRegistry;
use crate::models::{ScanSummary, VideoEntry};
use crate::settings::Settings;

/// شمارندهٔ توکن رزرو — هر رزرو یک هویت یکتا می‌گیرد تا گارد قدیم‌تر نتواند
/// رزرو عملیاتی جدیدتر را آزاد کند.
fn next_token() -> u64 {
    static TOKEN: AtomicU64 = AtomicU64::new(1);
    TOKEN.fetch_add(1, Ordering::Relaxed)
}

/// دسترسی poison-safe به Registry — قفل مسموم به‌جای panic بازیابی می‌شود.
/// پاک‌سازی و Drop هرگز به‌خاطر قفل مسموم panic نمی‌کنند.
fn lock_registry(registry: &Mutex<OpsRegistry>) -> MutexGuard<'_, OpsRegistry> {
    registry.lock().unwrap_or_else(PoisonError::into_inner)
}

/// رزرو عملیات گروهی — پرچم لغو + هویت یکتا.
#[derive(Debug, Clone)]
struct BatchReservation {
    cancel: Arc<AtomicBool>,
    token: u64,
}

/// رزرو عملیات تکی روی یک ویدیو — پرچم لغو + هویت یکتا.
#[derive(Debug, Clone)]
struct SingleReservation {
    cancel: Arc<AtomicBool>,
    token: u64,
}

/// رزرو اسکن — پرچم لغو تازه به‌ازای هر نسل اسکن + هویت یکتا.
/// پرچم هیچ‌گاه بین دو نسل اسکن استفاده مجدد نمی‌شود.
#[derive(Debug, Clone)]
struct ScanReservation {
    cancel: Arc<AtomicBool>,
    token: u64,
}

/// وضعیت عملیات جاری — همهٔ admissionها و رزروها اتمیک زیر همین Mutex.
/// هر رزرو یک توکن یکتا دارد؛ Drop گارد فقط رزروِ هم‌توکن خودش را آزاد می‌کند.
///
/// قواعد تعارض (پذیرفته‌شده در طرح pipeline):
/// - اسکن ↔ عملیات گروهی: متقابلاً رد (اسکن کتابخانه را کامل جایگزین می‌کند)
/// - اسکن ↔ عملیات تکی: متقابلاً رد (همان دلیل)
/// - عملیات گروهی ↔ عملیات تکی: متقابلاً رد (رفتار قدیمی)
/// - عملیات تکی روی ویدیوی دیگر: مجاز
/// - فقط یک pipeline در آینده از همین اسلات گروهی عبور می‌کند
#[derive(Debug, Default)]
pub struct OpsRegistry {
    batch: Option<BatchReservation>,
    single_ops: HashMap<String, SingleReservation>,
    scan_op: Option<ScanReservation>,
}

impl OpsRegistry {
    // ---- primitives (خصوصی؛ فقط admission helperهای همین ماژول صدا می‌زنند) ----

    /// ثبت عملیات گروهی — همهٔ پیش‌شرط‌ها قبل از هر mutation بررسی و سپس
    /// رزرو اتمیک زیر همان قفل انجام می‌شود.
    fn reserve_batch(&mut self) -> Result<BatchReservation, String> {
        if self.batch.is_some() {
            return Err("یک عملیات گروهی در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
        }
        if self.scan_op.is_some() {
            return Err("یک اسکن در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
        }
        if !self.single_ops.is_empty() {
            return Err(format!(
                "{} عملیات تکی در جریان است؛ ابتدا آن‌ها را تمام یا متوقف کنید.",
                self.single_ops.len()
            ));
        }
        let reservation = BatchReservation {
            cancel: Arc::new(AtomicBool::new(false)),
            token: next_token(),
        };
        self.batch = Some(reservation.clone());
        Ok(reservation)
    }

    /// آزادسازی فقط رزروِ هم‌توکن — گارد قدیم‌تر رزرو جدیدتر را نمی‌زند.
    fn release_batch(&mut self, token: u64) {
        if let Some(reservation) = &self.batch {
            if reservation.token == token {
                self.batch = None;
            }
        }
    }

    /// ثبت عملیات تکی روی یک ویدیو — اتمیک زیر قفل.
    fn reserve_single(&mut self, video_path: &str) -> Result<SingleReservation, String> {
        if self.batch.is_some() {
            return Err(
                "یک عملیات گروهی در حال اجراست؛ صبر کنید یا ابتدا آن را متوقف کنید.".into(),
            );
        }
        if self.scan_op.is_some() {
            return Err("یک اسکن در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
        }
        if self.single_ops.contains_key(video_path) {
            return Err("عملیات دیگری روی همین ویدیو در جریان است.".into());
        }
        let reservation = SingleReservation {
            cancel: Arc::new(AtomicBool::new(false)),
            token: next_token(),
        };
        self.single_ops
            .insert(video_path.to_string(), reservation.clone());
        Ok(reservation)
    }

    /// آزادسازی فقط رزروِ هم‌توکن برای همان ویدیو.
    fn release_single(&mut self, video_path: &str, token: u64) {
        if let Some(reservation) = self.single_ops.get(video_path) {
            if reservation.token == token {
                self.single_ops.remove(video_path);
            }
        }
    }

    /// ثبت رزرو اسکن — اتمیک؛ پرچم لغو کاملاً تازه (هیچ پرچم stale ای معنا ندارد).
    fn reserve_scan(&mut self) -> Result<ScanReservation, String> {
        if self.scan_op.is_some() {
            return Err("یک اسکن در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
        }
        if self.batch.is_some() {
            return Err("یک عملیات گروهی در حال اجراست؛ ابتدا آن را متوقف کنید.".into());
        }
        if !self.single_ops.is_empty() {
            return Err(format!(
                "{} عملیات تکی در جریان است؛ ابتدا آن‌ها را تمام یا متوقف کنید.",
                self.single_ops.len()
            ));
        }
        let reservation = ScanReservation {
            cancel: Arc::new(AtomicBool::new(false)),
            token: next_token(),
        };
        self.scan_op = Some(reservation.clone());
        Ok(reservation)
    }

    /// آزادسازی فقط رزروِ هم‌توکن اسکن.
    fn release_scan(&mut self, token: u64) {
        if let Some(reservation) = &self.scan_op {
            if reservation.token == token {
                self.scan_op = None;
            }
        }
    }

    // ---- سطح عمومی Registry ----

    /// آیا عملیات گروهی (دانلود/آرشیو/آینده: pipeline) فعالی جریان دارد؟
    pub fn has_active_batch(&self) -> bool {
        self.batch.is_some()
    }

    /// لغو عملیات‌های پوشش داده‌شدهٔ قرارداد فعلی (گروهی + تکی‌ها).
    /// عمداً رزرو اسکن را لغو نمی‌کند: اسکن با عملیات‌های گروهی/تکی متقابلاً
    /// انحصاری است؛ پس وقتی اسکن فعال است، هیچ عملیات subtitle فعالی وجود
    /// ندارد و رفتار قبلیِ «بی‌ضرر بودن» حفظ می‌شود. لغو اسکن فقط از طریق
    /// `request_scan_cancel` (stop_scan) انجام می‌شود.
    pub fn cancel_all(&mut self) -> bool {
        let mut any = false;
        if let Some(reservation) = &self.batch {
            reservation.cancel.store(true, Ordering::SeqCst);
            any = true;
        }
        for reservation in self.single_ops.values() {
            reservation.cancel.store(true, Ordering::SeqCst);
            any = true;
        }
        any
    }
}

/// دستگیرهٔ عملیات تکی — با Drop، فقط رزروِ هم‌توکن خودش را آزاد می‌کند (RAII).
/// حتی در صورت panic یا خطای Thread، اسلات قفل نمی‌ماند؛ قفل مسموم هم recovery می‌شود.
///
/// پرچم لغو مخصوص همین عملیات است؛ از `.cancel` در طول اجرا استفاده کنید.
#[derive(Debug)]
pub struct SingleOpGuard {
    registry: Arc<Mutex<OpsRegistry>>,
    video_path: String,
    pub cancel: Arc<AtomicBool>,
    token: u64,
}

impl Drop for SingleOpGuard {
    fn drop(&mut self) {
        lock_registry(&self.registry).release_single(&self.video_path, self.token);
    }
}

/// رزرو اتمیک اسلات عملیات تکی برای یک ویدیو.
/// قوانین (تحت یک قفل):
/// - حین عملیات گروهی → رد
/// - حین اسکن فعال → رد
/// - حین هر عملیات تکی دیگر روی همان ویدیو → رد
/// - روی ویدیوی دیگر → مجاز
pub fn acquire_single_op(
    registry: &Arc<Mutex<OpsRegistry>>,
    video_path: &str,
) -> Result<SingleOpGuard, String> {
    let reservation = lock_registry(registry).reserve_single(video_path)?;
    Ok(SingleOpGuard {
        registry: Arc::clone(registry),
        video_path: video_path.to_string(),
        cancel: reservation.cancel,
        token: reservation.token,
    })
}

/// دستگیرهٔ عملیات گروهی — با Drop، فقط رزروِ هم‌توکن خودش را آزاد می‌کند (RAII).
///
/// پرچم لغو مخصوص همین عملیات گروهی است؛ همان `AtomicBool` که در Registry
/// ثبت شده و `cancel_all` روی آن عمل می‌کند.
#[derive(Debug)]
pub struct BatchOpGuard {
    registry: Arc<Mutex<OpsRegistry>>,
    pub cancel: Arc<AtomicBool>,
    token: u64,
}

impl Drop for BatchOpGuard {
    fn drop(&mut self) {
        lock_registry(&self.registry).release_batch(self.token);
    }
}

/// رزرو اتمیک اسلات عملیات گروهی با گارد RAII — همان قوانین `reserve_batch`
/// (تعارض با عملیات گروهی دیگر، اسکن فعال و همهٔ عملیات‌های تکی رد می‌شود).
pub fn acquire_batch_op(registry: &Arc<Mutex<OpsRegistry>>) -> Result<BatchOpGuard, String> {
    let reservation = lock_registry(registry).reserve_batch()?;
    Ok(BatchOpGuard {
        registry: Arc::clone(registry),
        cancel: reservation.cancel,
        token: reservation.token,
    })
}

/// دستگیرهٔ رزرو اسکن — با Drop، فقط رزروِ هم‌توکن خودش را آزاد می‌کند (RAII).
///
/// پرچم لغو مخصوص همین نسل اسکن است؛ هیچ درخواست لغو قدیمی‌ای روی آن اثری
/// ندارد چون هر نسل اسکن پرچم کاملاً تازه می‌گیرد.
#[derive(Debug)]
pub struct ScanOpGuard {
    registry: Arc<Mutex<OpsRegistry>>,
    pub cancel: Arc<AtomicBool>,
    token: u64,
}

impl Drop for ScanOpGuard {
    fn drop(&mut self) {
        lock_registry(&self.registry).release_scan(self.token);
    }
}

/// رزرو اتمیک اسکن — با گارد RAII؛ تعارض با اسکن دیگر، عملیات گروهی و
/// عملیات تکی‌ها رد می‌شود (اسکن کتابخانه را کامل جایگزین می‌کند).
pub fn acquire_scan_op(registry: &Arc<Mutex<OpsRegistry>>) -> Result<ScanOpGuard, String> {
    let reservation = lock_registry(registry).reserve_scan()?;
    Ok(ScanOpGuard {
        registry: Arc::clone(registry),
        cancel: reservation.cancel,
        token: reservation.token,
    })
}

/// درخواست لغو اسکن فعال — اتمیک زیر قفل Registry و فقط برای رزروِ جاری.
/// بدون اسکن فعال → `Ok(false)` و هیچ پرچمی ست نمی‌شود؛ پس درخواست قدیمی
/// هرگز اسکن بعدی را لغو نمی‌کند (هر نسل پرچم تازه دارد).
pub fn request_scan_cancel(registry: &Arc<Mutex<OpsRegistry>>) -> Result<bool, String> {
    let ops = lock_registry(registry);
    match &ops.scan_op {
        Some(reservation) => {
            reservation.cancel.store(true, Ordering::SeqCst);
            Ok(true)
        }
        None => Ok(false),
    }
}

/// آیا اسکن فعالی جریان دارد؟
pub fn is_scan_active(registry: &Arc<Mutex<OpsRegistry>>) -> Result<bool, String> {
    Ok(lock_registry(registry).scan_op.is_some())
}

/// وضعیت سراسری برنامه — از طریق `app.state::<AppState>()` در دسترس است.
pub struct AppState {
    pub settings: Mutex<Settings>,
    pub library: Mutex<Vec<VideoEntry>>,
    pub last_scan: Mutex<Option<ScanSummary>>,
    pub ops: Arc<Mutex<OpsRegistry>>,
    /// Provenance سمت سرور برای stagingهای ساخته‌شده توسط دانلود گروهی.
    pub archive_provenance: Mutex<ProvenanceRegistry>,
    /// pipeline فعال (در صورت وجود) — admission از طریق ops registry انجام می‌شود.
    pub pipeline: Mutex<Option<Arc<crate::pipeline::PipelineControl>>>,
    /// هاردساب فعال (در صورت وجود) — admission از طریق همان اسلات batch.
    pub hardsub: Mutex<Option<Arc<crate::hardsub::HardsubControl>>>,
    /// انکودر اثبات‌شدهٔ هاردساب (کش سشن) — کلید: تنظیم encoder + مسیر ffmpeg.
    pub hardsub_encoder_cache:
        Mutex<Option<(String, crate::hardsub::encoder::VideoEncoder)>>,
    /// لغو نصب خودکار ابزارها (معاونت راه‌اندازی) — مستقل از registry عملیات.
    pub setup_cancel: Arc<AtomicBool>,
}

/// مسیرهای دائمی برنامه (تنظیمات، لاگ و داده‌های محلی ابزارها).
#[derive(Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    /// ریشهٔ داده‌های محلی — پوشهٔ `tools` نصب خودکار زیر آن ساخته می‌شود.
    pub data_dir: PathBuf,
}

impl AppPaths {
    /// پوشهٔ نصب ابزارهای خودکار — `<data_dir>/tools`.
    pub fn tools_dir(&self) -> PathBuf {
        self.data_dir.join("tools")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::Barrier;
    use std::thread;
    use std::time::Duration;

    fn registry() -> Arc<Mutex<OpsRegistry>> {
        Arc::new(Mutex::new(OpsRegistry::default()))
    }

    /// سناریوی پایه: Single فعال → Batch و Scan هر دو رد؛ آزادسازی → Batch موفق؛
    /// Batch فعال → Single و Scan رد.
    #[test]
    fn batch_and_single_are_mutually_exclusive() {
        let reg = registry();

        // ۱) Single فعال
        let g1 = acquire_single_op(&reg, "v1").unwrap();
        assert!(!g1.cancel.load(Ordering::SeqCst));
        let g2 = acquire_single_op(&reg, "v2").unwrap();

        // ۲) Batch و Scan در حالت Single فعال → هر دو رد
        assert!(acquire_batch_op(&reg).is_err());
        assert!(acquire_scan_op(&reg).is_err());

        // ۳) آزادسازی Singleها
        drop(g1);
        drop(g2);
        assert!(!is_scan_active(&reg).unwrap());

        // ۴) Batch اکنون موفق است
        let batch = acquire_batch_op(&reg).unwrap();
        assert!(!batch.cancel.load(Ordering::SeqCst));

        // ۵) در حالت Batch فعال → Single و Scan هر دو رد
        assert!(acquire_single_op(&reg, "v3").is_err());
        assert!(acquire_scan_op(&reg).is_err());

        // لغو قرارداد subtitle (گروهی + تکی) → پرچم گروهی true؛ پایان گروهی → تازه
        assert!(reg.lock().unwrap().cancel_all());
        assert!(batch.cancel.load(Ordering::SeqCst));
        drop(batch);
        let g3 = acquire_single_op(&reg, "v3").unwrap();
        assert!(!g3.cancel.load(Ordering::SeqCst));
    }

    #[test]
    fn same_video_single_is_rejected_other_video_is_allowed() {
        let reg = registry();
        let _g = acquire_single_op(&reg, "v1").unwrap();
        let err = acquire_single_op(&reg, "v1").unwrap_err();
        assert!(err.contains("همین ویدیو"), "{err}");
        let _g2 = acquire_single_op(&reg, "v2").unwrap();
    }

    /// RAII: panic در حین عملیات باید اسلات را آزاد کند.
    #[test]
    fn panic_releases_slot() {
        let reg = registry();
        let handle = {
            let reg = reg.clone();
            thread::spawn(move || {
                let _g = acquire_single_op(&reg, "v").unwrap();
                panic!("شبیه‌سازی خطای غیرمنتظره در عملیات");
            })
        };
        assert!(handle.join().is_err());
        assert!(acquire_single_op(&reg, "v").is_ok());
    }

    /// گارد گروهی (RAII): Drop اسلات را آزاد می‌کند؛ لغو از cancel_all روی
    /// همان پرچم ثبت‌شده اثر می‌گذارد؛ panic هم اسلات را آزاد می‌کند.
    #[test]
    fn batch_guard_releases_slot_on_drop_and_panic() {
        let reg = registry();

        let guard = acquire_batch_op(&reg).unwrap();
        assert!(acquire_single_op(&reg, "v").is_err());
        assert!(acquire_batch_op(&reg).is_err());
        assert!(acquire_scan_op(&reg).is_err());

        assert!(reg.lock().unwrap().cancel_all());
        assert!(guard.cancel.load(Ordering::SeqCst));

        drop(guard);
        let g2 = acquire_batch_op(&reg).unwrap();
        assert!(!g2.cancel.load(Ordering::SeqCst));
        drop(g2);

        let handle = {
            let reg = reg.clone();
            thread::spawn(move || {
                let _g = acquire_batch_op(&reg).unwrap();
                panic!("شبیه‌سازی خطای غیرمنتظره در عملیات گروهی");
            })
        };
        assert!(handle.join().is_err());
        assert!(acquire_batch_op(&reg).is_ok());
    }

    // ------------------------------------------------------------------
    // رزرو اسکن — admission اتمیک، لغو نسل‌محور، توکن‌محور
    // ------------------------------------------------------------------

    #[test]
    fn scan_admission_conflicts_with_batch_and_singles() {
        let reg = registry();

        // Single فعال → Scan رد؛ Scan فعال → Single و Batch رد
        let single = acquire_single_op(&reg, "v1").unwrap();
        assert!(acquire_scan_op(&reg).is_err());
        drop(single);

        let scan = acquire_scan_op(&reg).unwrap();
        assert!(acquire_scan_op(&reg).is_err()); // اسکن دوم
        assert!(acquire_batch_op(&reg).is_err()); // Batch حین اسکن
        assert!(acquire_single_op(&reg, "v1").is_err()); // Single حین اسکن

        // cancel_all قرارداد subtitle‌ها اسکن را لغو نمی‌کند — بدون اسکن/گروهی/تکیِ
        // دیگر فعال، بی‌ضرر false است و پرچم اسکن دست‌نخورده می‌ماند
        assert!(!reg.lock().unwrap().cancel_all());
        assert!(!scan.cancel.load(Ordering::SeqCst));

        // لغو اسکن فقط از مسیر اختصاصی آن
        assert!(request_scan_cancel(&reg).unwrap());
        assert!(scan.cancel.load(Ordering::SeqCst));

        drop(scan);
        assert!(!is_scan_active(&reg).unwrap());
        assert!(acquire_batch_op(&reg).is_ok()); // پس از پایان اسکن، Batch مجاز
    }

    /// درخواست لغو بدون اسکن فعال → Ok(false) و کاملاً بی‌ضرر.
    #[test]
    fn scan_cancel_without_active_scan_is_harmless() {
        let reg = registry();
        assert!(!request_scan_cancel(&reg).unwrap());
        // Batch فعال ولی بدون اسکن → stop_scan همچنان false است (اسکن را لغو نمی‌کند)
        let _batch = acquire_batch_op(&reg).unwrap();
        assert!(!request_scan_cancel(&reg).unwrap());
        assert!(reg.lock().unwrap().has_active_batch());
    }

    /// گارد قدیمی هرگز رزرو جدیدتر را آزاد یا لغو نمی‌کند — توکن هویت است.
    #[test]
    fn stale_scan_token_never_releases_or_cancels_newer_scan() {
        let reg = registry();
        let first = acquire_scan_op(&reg).unwrap();
        let first_cancel = Arc::clone(&first.cancel);
        let first_token = {
            let ops = reg.lock().unwrap();
            ops.scan_op.as_ref().unwrap().token
        };

        // نسل اول پایان می‌یابد؛ نسل دوم با پرچم و توکن تازه شروع می‌شود
        drop(first);
        let second = acquire_scan_op(&reg).unwrap();
        let second_token = {
            let ops = reg.lock().unwrap();
            ops.scan_op.as_ref().unwrap().token
        };
        assert_ne!(first_token, second_token);

        // «گارد قدیمی» تلاش می‌کند رزرو نسل دوم را آزاد کند — mismatch → بی‌اثر
        lock_registry(&reg).release_scan(first_token);
        assert!(is_scan_active(&reg).unwrap());

        // درخواست لغوِ قدیمی (روی پرچم نسل اول) پرچم نسل دوم را ست نمی‌کند
        first_cancel.store(true, Ordering::SeqCst);
        assert!(!second.cancel.load(Ordering::SeqCst));
        // درخواست لغوِ «فعلی» فقط نسل دوم را هدف می‌گیرد
        assert!(request_scan_cancel(&reg).unwrap());
        assert!(second.cancel.load(Ordering::SeqCst));
    }

    /// admission هم‌زمان — دقیقاً یکی می‌برد؛ ماتریس تعارض زیر فشار حفظ می‌شود.
    /// گارد برنده تا پایان شمارش نگه داشته می‌شود (در غیر این صورت Drop فوریِ
    /// temporary، رزرو را قبل از شمارش آزاد می‌کرد و نتیجه ناپایدار می‌شد).
    #[test]
    fn concurrent_admissions_are_atomic() {
        const SECS: Duration = Duration::from_secs(5);
        let reg = registry();
        let barrier = Arc::new(Barrier::new(3));
        let (result_tx, result_rx) = mpsc::channel();
        let mut release_txs = Vec::new();
        let mut handles = Vec::new();

        for kind in 0..3 {
            let reg = Arc::clone(&reg);
            let barrier = Arc::clone(&barrier);
            let result_tx = result_tx.clone();
            let (release_tx, release_rx) = mpsc::channel::<()>();
            release_txs.push(release_tx);
            handles.push(thread::spawn(move || {
                barrier.wait();
                match kind {
                    0 => {
                        let guard = acquire_scan_op(&reg);
                        let _ = result_tx.send(guard.is_ok());
                        let _ = release_rx.recv_timeout(SECS);
                        drop(guard);
                    }
                    1 => {
                        let guard = acquire_batch_op(&reg);
                        let _ = result_tx.send(guard.is_ok());
                        let _ = release_rx.recv_timeout(SECS);
                        drop(guard);
                    }
                    _ => {
                        let guard = acquire_single_op(&reg, "v");
                        let _ = result_tx.send(guard.is_ok());
                        let _ = release_rx.recv_timeout(SECS);
                        drop(guard);
                    }
                }
            }));
        }

        // سه عملیات متقابلاً انحصاری — دقیقاً یکی موفق (رزروها در طول شمارش زنده‌اند)
        let mut winners = 0;
        for _ in 0..3 {
            if result_rx.recv_timeout(SECS).expect("نتیجهٔ worker") {
                winners += 1;
            }
        }
        assert_eq!(winners, 1);

        // آزادسازی workerها → گاردها drop → رزروها آزاد
        drop(release_txs);
        for handle in handles {
            handle.join().unwrap();
        }
        // پس از پایان همه، اسلات کاملاً آزاد است
        assert!(acquire_batch_op(&reg).is_ok());
    }

    /// Drop هر گارد فقط رزرو هم‌توکن خودش را آزاد می‌کند (batch/single/scan).
    #[test]
    fn guard_drop_releases_only_its_own_token_reservation() {
        let reg = registry();
        let single = acquire_single_op(&reg, "v1").unwrap();
        let single_token = reg.lock().unwrap().single_ops["v1"].token;
        drop(single);
        // پس از Drop، رزرو حذف شده — Drop دوباره با همان توکن اثری ندارد
        lock_registry(&reg).release_single("v1", single_token);
        assert!(!reg.lock().unwrap().single_ops.contains_key("v1"));
        // acquisition جدید token تازه می‌گیرد
        let again = acquire_single_op(&reg, "v1").unwrap();
        assert_ne!(reg.lock().unwrap().single_ops["v1"].token, single_token);
        drop(again);

        let batch = acquire_batch_op(&reg).unwrap();
        let batch_token = reg.lock().unwrap().batch.as_ref().unwrap().token;
        drop(batch);
        lock_registry(&reg).release_batch(batch_token);
        assert!(reg.lock().unwrap().batch.is_none());
    }

    /// قفل مسموم: پاک‌سازی panic نمی‌کند و admission بعدی سالم کار می‌کند.
    #[test]
    fn poisoned_registry_recovery_keeps_cleanup_working() {
        let reg = registry();
        let scan = acquire_scan_op(&reg).unwrap();

        // قفل را با panic نگه‌دارنده مسموم می‌کنیم
        let handle = {
            let reg = reg.clone();
            thread::spawn(move || {
                let _guard = reg.lock().unwrap();
                panic!("مسموم‌سازی عمدی قفل Registry");
            })
        };
        assert!(handle.join().is_err());

        // Drop گارد روی قفل مسموم → recovery → رزرو آزاد، بدون panic
        drop(scan);
        assert!(!is_scan_active(&reg).unwrap());

        // admission بعدی سالم کار می‌کند
        let next = acquire_batch_op(&reg).unwrap();
        assert!(!next.cancel.load(Ordering::SeqCst));
        drop(next);
    }

    /// early-return/خطا → گارد Drop می‌شود → هیچ رزرو stale ای نمی‌ماند.
    #[test]
    fn early_return_releases_reservation() {
        let reg = registry();
        let outcome: Result<(), String> = (|| {
            let _guard = acquire_scan_op(&reg)?;
            Err("خطای عادی پس از رزرو".into())
        })();
        assert!(outcome.is_err());
        assert!(!is_scan_active(&reg).unwrap());
        assert!(acquire_batch_op(&reg).is_ok());
    }
}
