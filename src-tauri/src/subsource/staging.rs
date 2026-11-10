//! مدیریت Staging برای دانلود گروهی SubSource (فاز ۳ PR ۲).
//!
//! ## اصول
//!
//! - ویدیوها با `std::fs::hard_link` به staging می‌روند — **هرگز کپی نمی‌شوند**.
//! - ساختار پوشه‌ها (نسبت به مشترک‌ترین والد گروه) حفظ می‌شود تا فاز بعدی
//!   بتواند آرشیو هر ویدیو را پیدا کند.
//! - مالکیت هر مسیر موقت صریح ردیابی می‌شود؛ پاک‌سازی فقط مسیرهای
//!   ساخته‌شده توسط خود برنامه را حذف می‌کند و هرگز درختی نامعلوم را بازگشتی
//!   حذف نمی‌کند.
//! - ریشه staging با نام `_subsource_batch_<uid>` ساخته می‌شود — Scanner از
//!   قبل پوشه‌های با پیشوند `_subsource_` را از اسکن مستثنا می‌کند.
//! - hard link فقط روی همان volume ممکن است؛ ویدیوها بر اساس volume گروه
//!   می‌شوند و هر گروه staging مخصوص خودش را روی همان volume می‌گیرد.
//!   اگر ساخت hard link ناموفق باشد، هرگز به کپی به‌عنوان جایگزین
//!   برنمی‌گردیم — عملیات با rollback کامل و پیام خطا متوقف می‌شود.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::logging;
use crate::subsource::plan::{DownloadPlanItem, SkippedDownload};

/// پیشوند ریشه staging — باید با `STAGING_PREFIX` در Scanner هم‌خوان بماند.
pub const STAGING_DIR_PREFIX: &str = "_subsource_batch_";

/// یک لینک برنامه‌ریزی‌شده: ویدیوی اصلی و مسیر نسبی آن در staging.
#[derive(Debug, Clone)]
pub struct StagedPlacement {
    pub source: PathBuf,
    /// مسیر نسبی پوشه نسبت به ریشه staging (خالی = خود ریشه).
    pub rel_dir: PathBuf,
    pub file_name: String,
}

/// گروه ویدیوهای یک volume — هر گروه staging مستقل و اجرای جداگانه دارد.
#[derive(Debug, Clone)]
pub struct VolumeGroup {
    /// کلید نرمال‌شده volume (مثلاً `C:\`).
    pub volume_key: String,
    /// ریشه staging این گروه — روی همان volume ویدیوهاست.
    pub staging_root: PathBuf,
    pub placements: Vec<StagedPlacement>,
}

static UID: AtomicU64 = AtomicU64::new(0);

/// شناسه یکتا برای ریشه staging — برخورد عملاً ناممکن.
pub fn new_uid() -> String {
    let n = UID.fetch_add(1, Ordering::SeqCst);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{:x}-{}", n, std::process::id())
}

/// ریشه volume یک مسیر مطلق — `C:\` برای درایوها، `/` روی یونیکس.
/// برای مسیرهای نسبی `None` برمی‌گرداند.
pub fn volume_root_of(path: &Path) -> Option<PathBuf> {
    let mut comps = path.components();
    match comps.next()? {
        Component::Prefix(p) => {
            let mut s = p.as_os_str().to_os_string();
            if comps.next() == Some(Component::RootDir) {
                s.push(std::path::MAIN_SEPARATOR.to_string());
            }
            Some(PathBuf::from(s))
        }
        Component::RootDir => Some(PathBuf::from(std::path::MAIN_SEPARATOR.to_string())),
        _ => None,
    }
}

/// مشترک‌ترین والد چند پوشه مطلق.
fn common_ancestor(dirs: &[PathBuf]) -> Option<PathBuf> {
    let mut anc = dirs.first()?.clone();
    for d in dirs.iter().skip(1) {
        while !d.starts_with(&anc) {
            anc = anc.parent()?.to_path_buf();
        }
    }
    Some(anc)
}

/// گروه‌بندی ویدیوها بر اساس volume و محاسبه مسیر نسبی هر فایل در staging.
///
/// `base_dir` (فقط تست): به‌جای ریشه volume، staging زیر این پوشه ساخته
/// می‌شود — باید روی همان volume ویدیوها باشد.
///
/// مواردی که volumeشان تشخیص داده نمی‌شود یا مسیر stagingشان با مورد دیگری
/// تصادم می‌کند، با دلیل در skipped برمی‌گردند.
pub fn group_by_volume(
    items: &[DownloadPlanItem],
    base_dir: Option<&Path>,
) -> (Vec<VolumeGroup>, Vec<SkippedDownload>) {
    let mut skipped = Vec::new();

    // پاس ۱: تشخیص volume و والد هر ویدیو (حفظ ترتیب ورودی)
    let mut by_volume: HashMap<String, Vec<(PathBuf, DownloadPlanItem)>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut volume_roots: HashMap<String, PathBuf> = HashMap::new();

    for item in items {
        let video = PathBuf::from(&item.video_path);
        let Some(root) = volume_root_of(&video) else {
            skipped.push(SkippedDownload {
                video_path: item.video_path.clone(),
                video_file_name: item.video_file_name.clone(),
                reason: "مسیر نسبی برای staging پشتیبانی نمی‌شود".into(),
            });
            continue;
        };
        let key = root.to_string_lossy().to_uppercase();
        if !by_volume.contains_key(&key) {
            order.push(key.clone());
            volume_roots.insert(key.clone(), root);
        }
        let parent = video.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        by_volume
            .entry(key)
            .or_default()
            .push((parent, item.clone()));
    }

    // پاس ۲: والد مشترک هر گروه + مسیر نسبی هر فایل + کنترل تصادم
    let mut groups = Vec::new();
    for key in order {
        let entries = by_volume.remove(&key).unwrap_or_default();
        let parents: Vec<PathBuf> = entries.iter().map(|(p, _)| p.clone()).collect();
        let Some(ancestor) = common_ancestor(&parents) else {
            continue;
        };

        let mut placements = Vec::new();
        let mut used: HashMap<PathBuf, String> = HashMap::new();
        for (parent, item) in entries {
            let rel = parent.strip_prefix(&ancestor).unwrap_or(Path::new(""));
            let staged_dir = PathBuf::from(&key).join(rel);
            let target = staged_dir.join(&item.video_file_name);
            if let Some(owner) = used.get(&target) {
                skipped.push(SkippedDownload {
                    video_path: item.video_path.clone(),
                    video_file_name: item.video_file_name.clone(),
                    reason: format!("مسیر staging با «{owner}» تصادم می‌کند"),
                });
                continue;
            }
            used.insert(target, item.video_file_name.clone());
            placements.push(StagedPlacement {
                source: PathBuf::from(&item.video_path),
                rel_dir: rel.to_path_buf(),
                file_name: item.video_file_name.clone(),
            });
        }

        let staging_parent = base_dir
            .map(|b| b.to_path_buf())
            .unwrap_or_else(|| volume_roots[&key].clone());
        groups.push(VolumeGroup {
            volume_key: key,
            staging_root: staging_parent.join(format!("{}{}", STAGING_DIR_PREFIX, new_uid())),
            placements,
        });
    }

    (groups, skipped)
}

/// ساخت hard link بدون بازنویسی — مقصد موجود هرگز بازنویسی نمی‌شود.
pub fn hard_link_no_overwrite(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        return Err(format!(
            "مقصد از قبل موجود است — بازنویسی ممنوع: {}",
            target.display()
        ));
    }
    std::fs::hard_link(source, target).map_err(|e| e.to_string())
}

/// نتیجهٔ پاک‌سازی لینک‌ها در جلسهٔ staging.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupResult {
    /// تعداد لینک‌هایی که با موفقیت حذف شدند یا از قبل موجود نبودند.
    pub removed: usize,
    /// لینک‌هایی که حذف آن‌ها ناموفق بود و همراه با پیام خطا در مالکیت باقی ماندند.
    pub failed: Vec<(PathBuf, String)>,
}

impl CleanupResult {
    /// آیا همهٔ لینک‌ها بدون خطا پاک شدند؟
    pub fn is_success(&self) -> bool {
        self.failed.is_empty()
    }
}

/// جلسه staging — مالکیت صریح همه مسیرهای موقتی ساخته‌شده توسط برنامه.
#[derive(Debug)]
pub struct StagingSession {
    root: PathBuf,
    /// لینک‌های ساخته‌شده توسط این جلسه (به ترتیب ساخت).
    links: Vec<PathBuf>,
    /// پوشه‌های ساخته‌شده توسط این جلسه (به ترتیب ساخت — از بیرون به داخل؛
    /// خود ریشه هم اولین عنصر است).
    created_dirs: Vec<PathBuf>,
}

impl StagingSession {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn link_count(&self) -> usize {
        self.links.len()
    }

    pub fn links(&self) -> &[PathBuf] {
        &self.links
    }

    pub fn created_dirs(&self) -> &[PathBuf] {
        &self.created_dirs
    }

    /// حذف همه لینک‌های ساخته‌شده توسط این جلسه با تابع حذف سفارشی (برای تست قطعی).
    /// لینک‌های موفق و موارد `NotFound` از ردیابی حذف می‌شوند؛ لینک‌های با خطا در
    /// `self.links` باقی می‌مانند تا مالکیت صریح آن‌ها حفظ شود.
    pub fn remove_links_with<F>(&mut self, mut remove_fn: F) -> CleanupResult
    where
        F: FnMut(&Path) -> std::io::Result<()>,
    {
        let mut removed = 0;
        let mut remaining_links = Vec::new();
        let mut failed = Vec::new();

        for link in self.links.drain(..) {
            match remove_fn(&link) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => removed += 1,
                Err(e) => {
                    let err_msg = e.to_string();
                    logging::warn(
                        "subsource",
                        &format!("حذف لینک موقت ناموفق: {} ({err_msg})", link.display()),
                    );
                    failed.push((link.clone(), err_msg));
                    remaining_links.push(link);
                }
            }
        }

        self.links = remaining_links;
        CleanupResult { removed, failed }
    }

    /// حذف همه لینک‌های ساخته‌شده توسط این جلسه — فایل اصلی هرگز دست
    /// نمی‌خورد (حذف یک hard link فقط همان لینک را برمی‌دارد).
    pub fn remove_links(&mut self) -> CleanupResult {
        self.remove_links_with(|p| std::fs::remove_file(p))
    }

    /// حذف پوشه‌های ساخته‌شده توسط جلسه — از عمیق به سطحی. پوشه‌های
    /// غیرخالی (مثلاً حاوی خروجی SubSource یا لینک‌های باقیمانده) عمداً باقی می‌مانند
    /// و در `self.created_dirs` ثبت می‌مانند.
    /// `including_root`: در rollback صدا زده می‌شود تا در صورت خالی بودن، ریشه هم حذف شود.
    fn remove_created_dirs(&mut self, including_root: bool) {
        let mut remaining_dirs = Vec::new();
        for dir in self.created_dirs.drain(..).rev() {
            if dir == self.root && !including_root {
                remaining_dirs.push(dir);
                continue;
            }
            // خطای «غیرخالی» یعنی محتوای ارزشمند (خروجی SubSource) یا لینک باقیمانده — نادیده
            if let Err(e) = std::fs::remove_dir(&dir) {
                if e.kind() != std::io::ErrorKind::DirectoryNotEmpty
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    logging::warn(
                        "subsource",
                        &format!("حذف پوشه موقت ناموفق: {} ({e})", dir.display()),
                    );
                }
                remaining_dirs.push(dir);
            }
        }
        remaining_dirs.reverse();
        self.created_dirs = remaining_dirs;
    }

    /// پاک‌سازی پس از پایان SubSource با تابع حذف سفارشی: حذف لینک‌های ویدیو +
    /// پوشه‌های خالیِ ساخته‌شده؛ ریشه staging و هر خروجی (آرشیوهای `sub/`)
    /// حفظ می‌شود. لینک‌های ناموفق در مالکیت باقی می‌مانند و در CleanupResult گزارش می‌شوند.
    pub fn cleanup_keep_output_with<F>(&mut self, remove_fn: F) -> CleanupResult
    where
        F: FnMut(&Path) -> std::io::Result<()>,
    {
        let res = self.remove_links_with(remove_fn);
        self.remove_created_dirs(false);
        res
    }

    /// پاک‌سازی پس از پایان SubSource: حذف لینک‌های ویدیو + پوشه‌های خالیِ
    /// ساخته‌شده؛ ریشه staging و هر خروجی (آرشیوهای `sub/`) **حفظ می‌شود**
    /// تا فاز بعدی (Archive Extraction) آن‌ها را بخواند.
    pub fn cleanup_keep_output(&mut self) -> CleanupResult {
        self.cleanup_keep_output_with(|p| std::fs::remove_file(p))
    }

    /// برگرداندن ساختار staging با تابع حذف سفارشی (پیش از اجرای SubSource یا هنگام خطا).
    pub fn rollback_with<F>(&mut self, remove_fn: F) -> CleanupResult
    where
        F: FnMut(&Path) -> std::io::Result<()>,
    {
        let res = self.remove_links_with(remove_fn);
        self.remove_created_dirs(true);
        res
    }

    /// برگرداندن کامل ساختار staging (فقط پیش از اجرای SubSource):
    /// حذف لینک‌ها، پوشه‌های ساخته‌شده و خود ریشه (در صورت خالی بودن).
    pub fn rollback(&mut self) -> CleanupResult {
        self.rollback_with(|p| std::fs::remove_file(p))
    }
}

/// خطای ساخت staging — حفظ صریح دلیل خطا، وضعیت rollback، ریشه staging و مالکیت پیوندهای باقیمانده.
#[derive(Debug)]
pub struct StagingBuildError {
    /// دلیل اصلی شکست ساخت staging یا پیام لغو.
    pub message: String,
    /// مسیر ریشه staging برای بازیابی دستی.
    pub staging_root: PathBuf,
    /// نتیجه rollback (تعداد لینک‌های حذف‌شده و موارد با خطا).
    pub cleanup: CleanupResult,
    /// جلسه staging در صورت باقی ماندن لینک‌ها یا پوشه‌ها (حفظ کامل مالکیت).
    pub session: Option<Box<StagingSession>>,
}

impl std::fmt::Display for StagingBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.cleanup.is_success() {
            write!(f, "{}", self.message)
        } else {
            write!(
                f,
                "{} (هشدار: rollback ناقص بود و {} لینک موقت حذف نشد: «{}»)",
                self.message,
                self.cleanup.failed.len(),
                self.staging_root.display()
            )
        }
    }
}

impl std::error::Error for StagingBuildError {}

/// ساخت ریشه staging و همه لینک‌های یک گروه با تابع حذف سفارشی برای rollback.
pub fn build_with_remover<F>(
    root: PathBuf,
    placements: &[StagedPlacement],
    cancel: Option<&std::sync::atomic::AtomicBool>,
    on_progress: &mut dyn FnMut(usize, usize, &StagedPlacement),
    remover: F,
) -> Result<StagingSession, StagingBuildError>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let mut session = StagingSession {
        root: root.clone(),
        links: Vec::with_capacity(placements.len()),
        created_dirs: Vec::new(),
    };

    let mut remover = remover;
    let mut fail = |mut session: StagingSession, msg: String| -> StagingBuildError {
        let cleanup = session.rollback_with(&mut remover);
        if !cleanup.is_success() {
            logging::warn(
                "subsource",
                &format!(
                    "rollback ناقص در حین ساخت staging: {} لینک حذف نشد: {:?}",
                    cleanup.failed.len(),
                    cleanup.failed
                ),
            );
        }
        let staging_root = session.root().to_path_buf();
        let remaining_session = if cleanup.is_success() {
            None
        } else {
            Some(Box::new(session))
        };
        StagingBuildError {
            message: msg,
            staging_root,
            cleanup,
            session: remaining_session,
        }
    };

    // ریشه staging — نباید از قبل موجود باشد (برخورد با مسیر نامعلوم ممنوع)
    match std::fs::create_dir(&root) {
        Ok(()) => session.created_dirs.push(root.clone()),
        Err(e) => {
            return Err(StagingBuildError {
                message: format!("ساخت ریشه staging ناموفق: {} ({e})", root.display()),
                staging_root: root,
                cleanup: CleanupResult::default(),
                session: None,
            });
        }
    }

    let total = placements.len();
    for (i, placement) in placements.iter().enumerate() {
        if let Some(c) = cancel {
            if c.load(Ordering::SeqCst) {
                return Err(fail(session, "عملیات در حین ساخت staging لغو شد".into()));
            }
        }

        // ساخت زنجیره پوشه‌های نسبی (فقط سطح‌هایی که ساخته می‌شوند ثبت می‌شوند)
        let mut cur = root.clone();
        for comp in placement.rel_dir.components() {
            cur.push(comp);
            match std::fs::create_dir(&cur) {
                Ok(()) => session.created_dirs.push(cur.clone()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => {
                    return Err(fail(
                        session,
                        format!("ساخت پوشه staging ناموفق: {} ({e})", cur.display()),
                    ));
                }
            }
        }

        let target = cur.join(&placement.file_name);
        if let Err(e) = hard_link_no_overwrite(&placement.source, &target) {
            return Err(fail(
                session,
                format!(
                    "ساخت hard link برای «{}» ناموفق: {e} — عملیات بدون تغییر ادامه نیافت",
                    placement.file_name
                ),
            ));
        }
        session.links.push(target);

        on_progress(i + 1, total, placement);
    }

    if session.links.is_empty() {
        return Err(fail(session, "هیچ فایلی به staging لینک نشد".into()));
    }

    Ok(session)
}

/// ساخت ریشه staging و همه لینک‌های یک گروه.
///
/// در هر خطا (ساخت پوشه یا هر hard link) کل ساختار تا همان لحظه rollback
/// می‌شود و هیچ اثری از staging باقی نمی‌ماند. هرگز به کپی جایگزین
/// برنمی‌گردیم.
///
/// `on_progress` پس از هر لینک موفق با (تعداد موفق، کل، placement) صدا
/// زده می‌شود.
pub fn build(
    root: PathBuf,
    placements: &[StagedPlacement],
    cancel: Option<&std::sync::atomic::AtomicBool>,
    on_progress: &mut dyn FnMut(usize, usize, &StagedPlacement),
) -> Result<StagingSession, StagingBuildError> {
    build_with_remover(root, placements, cancel, on_progress, |p| {
        std::fs::remove_file(p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "subsync-staging-test-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_video(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn volume_root_detection() {
        #[cfg(windows)]
        {
            assert_eq!(
                volume_root_of(Path::new("C:\\a\\b.mkv")),
                Some(PathBuf::from("C:\\"))
            );
            assert_eq!(
                volume_root_of(Path::new("d:\\x.mkv")),
                Some(PathBuf::from("d:\\"))
            );
        }
        assert_eq!(
            volume_root_of(Path::new("/tmp/x.mkv")),
            Some(PathBuf::from("/"))
        );
        assert_eq!(volume_root_of(Path::new("relative/x.mkv")), None);
    }

    #[test]
    fn common_ancestor_computation() {
        let dirs = vec![
            PathBuf::from("C:\\lib\\Show\\S01"),
            PathBuf::from("C:\\lib\\Show\\S02"),
            PathBuf::from("C:\\lib\\Movies"),
        ];
        assert_eq!(common_ancestor(&dirs), Some(PathBuf::from("C:\\lib")));
        let single = vec![PathBuf::from("C:\\lib\\Show\\S01")];
        assert_eq!(
            common_ancestor(&single),
            Some(PathBuf::from("C:\\lib\\Show\\S01"))
        );
    }

    #[test]
    fn grouping_splits_by_volume_and_preserves_rel_structure() {
        // مسیرهای فرضی — فقط ریاضیات مسیر، بدون دیسک
        let items = vec![
            DownloadPlanItem {
                video_path: "C:\\lib\\Show\\S01\\E01.mkv".into(),
                video_file_name: "E01.mkv".into(),
            },
            DownloadPlanItem {
                video_path: "C:\\lib\\Show\\S02\\E02.mkv".into(),
                video_file_name: "E02.mkv".into(),
            },
            DownloadPlanItem {
                video_path: "C:\\lib\\E03.mkv".into(),
                video_file_name: "E03.mkv".into(),
            },
            DownloadPlanItem {
                video_path: "D:\\other\\E04.mkv".into(),
                video_file_name: "E04.mkv".into(),
            },
        ];
        let (groups, skipped) = group_by_volume(&items, None);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(groups.len(), 2);

        let c = groups
            .iter()
            .find(|g| g.volume_key.starts_with("C:"))
            .unwrap();
        let d = groups
            .iter()
            .find(|g| g.volume_key.starts_with("D:"))
            .unwrap();
        assert_eq!(c.placements.len(), 3);
        assert_eq!(d.placements.len(), 1);

        // ساختار نسبی: والد مشترک C:\lib — ریشه‌های staging جدا و روی همان volume
        let e01 = c
            .placements
            .iter()
            .find(|p| p.file_name == "E01.mkv")
            .unwrap();
        assert_eq!(e01.rel_dir, PathBuf::from("Show\\S01"));
        let e03 = c
            .placements
            .iter()
            .find(|p| p.file_name == "E03.mkv")
            .unwrap();
        assert_eq!(e03.rel_dir, PathBuf::from(""));
        assert!(c
            .staging_root
            .starts_with(volume_root_of(Path::new("C:\\lib\\x")).unwrap()));
        assert!(d
            .staging_root
            .starts_with(volume_root_of(Path::new("D:\\other")).unwrap()));
        assert!(c
            .staging_root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(STAGING_DIR_PREFIX));

        // base_dir override — ریشه زیر پوشه داده‌شده
        let base = unique_dir("group-base");
        let (groups, _) = group_by_volume(&items, Some(&base));
        assert!(groups[0].staging_root.starts_with(&base));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn hard_link_refuses_existing_target() {
        let dir = unique_dir("link-overwrite");
        let src = write_video(&dir, "src.mkv", b"SRC");
        let target = write_video(&dir, "target.mkv", b"EXISTING");

        let err = hard_link_no_overwrite(&src, &target).unwrap_err();
        assert!(err.contains("بازنویسی ممنوع"), "{err}");
        // محتوای مقصد دست‌نخورده
        assert_eq!(std::fs::read(&target).unwrap(), b"EXISTING");
        assert_eq!(std::fs::read(&src).unwrap(), b"SRC");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_link_shares_content_without_copying() {
        let dir = unique_dir("link-real");
        let src = write_video(&dir, "video.mkv", b"CONTENT-ABC");
        let target = dir.join("linked.mkv");
        hard_link_no_overwrite(&src, &target).unwrap();

        // تغییر در target باید بلافاصله در src دیده شود (اشاره به همان داده بدون کپی)
        std::fs::write(&target, b"MUTATED-CONTENT").unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), b"MUTATED-CONTENT");

        // حذف لینک به فایل اصلی آسیب نمی‌زند
        std::fs::remove_file(&target).unwrap();
        assert_eq!(std::fs::read(&src).unwrap(), b"MUTATED-CONTENT");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_creates_structure_and_rollback_removes_everything() {
        let base = unique_dir("build-rollback");
        let lib = base.join("کتابخانه سریال - Season 01");
        std::fs::create_dir_all(&lib).unwrap();
        let v1 = write_video(&lib, "Show.S01E01.mkv", b"V1");
        let v2 = write_video(&lib, "فارسی.نام.مخصوص.mkv", b"V2");

        let placements = vec![
            StagedPlacement {
                source: v1.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "Show.S01E01.mkv".into(),
            },
            StagedPlacement {
                source: v2.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "فارسی.نام.مخصوص.mkv".into(),
            },
        ];
        let root = base.join(format!("{STAGING_DIR_PREFIX}test1"));
        let mut session = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap();
        assert_eq!(session.link_count(), 2);

        // rollback کامل — هیچ اثری نماند و فایل‌های اصلی سالم باشند
        let cleanup = session.rollback();
        assert_eq!(cleanup.removed, 2);
        assert!(cleanup.is_success());
        assert_eq!(session.link_count(), 0);
        assert!(!root.exists());
        assert_eq!(std::fs::read(&v1).unwrap(), b"V1");
        assert_eq!(std::fs::read(&v2).unwrap(), b"V2");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_failure_rolls_back_partial_work() {
        let base = unique_dir("build-fail");
        let lib = base.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let good = write_video(&lib, "good.mkv", b"GOOD");

        // لینک دوم به منبع ناموجود → build خطا می‌دهد و rollback کامل انجام می‌شود
        let placements = vec![
            StagedPlacement {
                source: good.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "good.mkv".into(),
            },
            StagedPlacement {
                source: lib.join("missing.mkv"),
                rel_dir: PathBuf::from(""),
                file_name: "missing.mkv".into(),
            },
        ];
        let root = base.join(format!("{STAGING_DIR_PREFIX}test2"));
        let err = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap_err();
        assert!(err.message.contains("hard link"), "{err}");
        assert!(err.cleanup.is_success());
        assert!(err.session.is_none());
        assert!(!root.exists(), "rollback باید ریشه را حذف کند");
        assert_eq!(std::fs::read(&good).unwrap(), b"GOOD");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_refuses_preexisting_root_without_touching_it() {
        let base = unique_dir("build-preexisting");
        let root = base.join(format!("{STAGING_DIR_PREFIX}foreign"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("foreign.txt"), b"do not touch").unwrap();

        let src = write_video(&base, "v.mkv", b"V");
        let placements = vec![StagedPlacement {
            source: src,
            rel_dir: PathBuf::from(""),
            file_name: "v.mkv".into(),
        }];
        let err = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap_err();
        assert!(err.message.contains("ناموفق"), "{err}");
        assert!(err.session.is_none());
        assert_eq!(
            std::fs::read_to_string(root.join("foreign.txt")).unwrap(),
            "do not touch"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_respects_cancellation_and_rolls_back() {
        let base = unique_dir("build-cancel");
        let lib = base.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let v = write_video(&lib, "v.mkv", b"V");

        let placements = vec![StagedPlacement {
            source: v,
            rel_dir: PathBuf::from(""),
            file_name: "v.mkv".into(),
        }];
        let root = base.join(format!("{STAGING_DIR_PREFIX}test3"));
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let err = build(root.clone(), &placements, Some(&cancel), &mut |_, _, _| {}).unwrap_err();
        assert!(err.message.contains("لغو"), "{err}");
        assert!(err.cleanup.is_success());
        assert!(err.session.is_none());
        assert!(!root.exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_with_remover_preserves_error_and_session_on_failed_rollback() {
        let base = unique_dir("build-err-pres");
        let v1 = write_video(&base, "v1.mkv", b"V1");
        let root = base.join(format!("{STAGING_DIR_PREFIX}b_err"));

        let placements = vec![
            StagedPlacement {
                source: v1.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "v1.mkv".into(),
            },
            StagedPlacement {
                source: base.join("missing.mkv"),
                rel_dir: PathBuf::from(""),
                file_name: "missing.mkv".into(),
            },
        ];

        let target_v1 = root.join("v1.mkv");
        let err = build_with_remover(root.clone(), &placements, None, &mut |_, _, _| {}, |p| {
            if p == target_v1 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "خطای شبیه‌سازی‌شده در rollback",
                ))
            } else {
                std::fs::remove_file(p)
            }
        })
        .unwrap_err();

        assert!(err.message.contains("hard link"));
        assert_eq!(err.staging_root, root);
        assert!(!err.cleanup.is_success());
        assert_eq!(err.cleanup.failed.len(), 1);
        assert_eq!(err.cleanup.failed[0].0, target_v1);
        assert!(err.session.is_some());
        let session = err.session.unwrap();
        assert_eq!(session.link_count(), 1);
        assert_eq!(session.links(), std::slice::from_ref(&target_v1));
        assert!(target_v1.exists());

        let _ = std::fs::remove_file(&target_v1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_keeps_output_and_removes_empty_dirs() {
        let base = unique_dir("cleanup");
        let season = base.join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        let v1 = write_video(&season, "Show.S01E01.mkv", b"V1");
        let v2 = write_video(&base, "Movie.mkv", b"V2");

        let placements = vec![
            StagedPlacement {
                source: v1.clone(),
                rel_dir: PathBuf::from("Season 01"),
                file_name: "Show.S01E01.mkv".into(),
            },
            StagedPlacement {
                source: v2.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "Movie.mkv".into(),
            },
        ];
        let root = base.join(format!("{STAGING_DIR_PREFIX}test4"));
        let mut session = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap();

        // خروجی شبیه‌سازی‌شده SubSource: sub/ کنار ویدیوی لینک‌شده
        let sub_dir = root.join("Season 01").join("sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        std::fs::write(sub_dir.join("Show.S01E01_sub_1.zip"), b"ARCHIVE").unwrap();

        let cleanup = session.cleanup_keep_output();
        assert_eq!(cleanup.removed, 2);
        assert!(cleanup.is_success());
        assert_eq!(session.link_count(), 0);

        // لینک‌ها حذف شدند
        assert!(!root.join("Movie.mkv").exists());
        assert!(!root.join("Season 01").join("Show.S01E01.mkv").exists());
        // خروجی و ساختار حاوی آن حفظ شد
        assert!(sub_dir.join("Show.S01E01_sub_1.zip").is_file());
        assert!(root.join("Season 01").is_dir());
        assert!(root.is_dir(), "ریشه staging برای فاز بعدی حفظ می‌شود");
        // پوشه خالی تودرتو ساخته‌شده حذف شود (p.empty در root)
        std::fs::create_dir_all(root.join("p").join("empty")).unwrap();
        // (این پوشه مال جلسه نیست — cleanup آن را دست نمی‌زند)
        assert!(root.join("p").join("empty").is_dir());

        // فایل‌های اصلی سالم
        assert_eq!(std::fs::read(&v1).unwrap(), b"V1");
        assert_eq!(std::fs::read(&v2).unwrap(), b"V2");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn remove_links_failure_preserves_failed_paths() {
        let base = unique_dir("link-preserve");
        let v1 = write_video(&base, "v1.mkv", b"V1");
        let v2 = write_video(&base, "v2.mkv", b"V2");
        let root = base.join(format!("{STAGING_DIR_PREFIX}pres1"));
        let placements = vec![
            StagedPlacement {
                source: v1.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "v1.mkv".into(),
            },
            StagedPlacement {
                source: v2.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "v2.mkv".into(),
            },
        ];
        let mut session = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap();
        assert_eq!(session.link_count(), 2);

        let target_fail = root.join("v2.mkv");
        let res = session.remove_links_with(|p| {
            if p == target_fail {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "دسترسی شبیه‌سازی‌شده ناموفق",
                ))
            } else {
                std::fs::remove_file(p)
            }
        });

        // لینک اول حذف شد، لینک دوم ناموفق و در مالکیت باقی ماند
        assert_eq!(res.removed, 1);
        assert_eq!(res.failed.len(), 1);
        assert_eq!(res.failed[0].0, target_fail);
        assert!(res.failed[0].1.contains("دسترسی"));
        assert!(!res.is_success());

        assert_eq!(session.link_count(), 1);
        assert_eq!(session.links(), std::slice::from_ref(&target_fail));
        assert!(target_fail.exists());

        // اجرای دوباره پاک‌سازی با حذف عادی، لینک باقیمانده را پاک می‌کند
        let res2 = session.remove_links();
        assert_eq!(res2.removed, 1);
        assert!(res2.is_success());
        assert_eq!(session.link_count(), 0);
        assert!(!target_fail.exists());

        let _ = session.rollback();
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn cleanup_keep_output_reports_failed_links() {
        let base = unique_dir("cleanup-fail");
        let season = base.join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        let v1 = write_video(&season, "Show.S01E01.mkv", b"V1");
        let v2 = write_video(&base, "Movie.mkv", b"V2");

        let placements = vec![
            StagedPlacement {
                source: v1.clone(),
                rel_dir: PathBuf::from("Season 01"),
                file_name: "Show.S01E01.mkv".into(),
            },
            StagedPlacement {
                source: v2.clone(),
                rel_dir: PathBuf::from(""),
                file_name: "Movie.mkv".into(),
            },
        ];
        let root = base.join(format!("{STAGING_DIR_PREFIX}cleanup_fail"));
        let mut session = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap();

        let staged_v1 = root.join("Season 01").join("Show.S01E01.mkv");
        let res = session.cleanup_keep_output_with(|p| {
            if p == staged_v1 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "خطای دسترسی در cleanup",
                ))
            } else {
                std::fs::remove_file(p)
            }
        });

        assert_eq!(res.removed, 1);
        assert_eq!(res.failed.len(), 1);
        assert_eq!(res.failed[0].0, staged_v1);
        assert_eq!(session.link_count(), 1);
        assert_eq!(session.links(), std::slice::from_ref(&staged_v1));
        assert!(staged_v1.exists());

        // پاک‌سازی نهایی برای تمیز کردن دیسک تست
        let _ = std::fs::remove_file(&staged_v1);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rollback_failure_preserves_failed_paths_and_dirs() {
        let base = unique_dir("rollback-fail");
        let season = base.join("Season 01");
        std::fs::create_dir_all(&season).unwrap();
        let v1 = write_video(&season, "Ep1.mkv", b"V1");

        let placements = vec![StagedPlacement {
            source: v1.clone(),
            rel_dir: PathBuf::from("Season 01"),
            file_name: "Ep1.mkv".into(),
        }];
        let root = base.join(format!("{STAGING_DIR_PREFIX}rb_fail"));
        let mut session = build(root.clone(), &placements, None, &mut |_, _, _| {}).unwrap();

        let staged_v1 = root.join("Season 01").join("Ep1.mkv");
        let res = session.rollback_with(|p| {
            if p == staged_v1 {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "خطای دسترسی در rollback",
                ))
            } else {
                std::fs::remove_file(p)
            }
        });

        assert_eq!(res.removed, 0);
        assert_eq!(res.failed.len(), 1);
        assert_eq!(res.failed[0].0, staged_v1);
        assert_eq!(session.link_count(), 1);
        assert!(staged_v1.exists());
        // پوشه‌ها چون خالی نبودند نباید حذف شده باشند
        assert!(root.join("Season 01").is_dir());
        assert!(root.is_dir());

        let _ = std::fs::remove_file(&staged_v1);
        let _ = std::fs::remove_dir_all(&base);
    }
}
