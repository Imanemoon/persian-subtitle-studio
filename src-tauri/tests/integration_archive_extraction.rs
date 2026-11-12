//! تست‌های یکپارچه استخراج آرشیو و تطبیق قسمت (فاز ۳ PR ۳).
//!
//! همه تست‌ها با fake 7-Zip (فایل اجرایی مستقل) و روی فایل‌سیستم واقعی اجرا
//! می‌شوند؛ هیچ آرشیو یا ویدیوی واقعی داخل ریپو نیست.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use subsync_studio_lib::archive::engine::{execute_archive_extraction, plan_archives};
use subsync_studio_lib::archive::types::{
    ArchiveExecuteRequest, ArchiveExecuteResult, ArchiveExtractionPlan, ArchiveItemStatus,
    ArchivePlanStatus, ArchiveSelection, ArchiveSource, SubtitleCandidate,
};

const ZIP: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
const RAR5: &[u8] = &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00];
const SEVEN_ZIP_MAGIC: &[u8] = b"7z\xBC\xAF\x27\x1C";

const SRT_FA: &str = "1\n00:00:01,000 --> 00:00:04,000\nسلام دنیا\n\n2\n00:00:05,000 --> 00:00:08,000\nخط دوم فارسی\n";
const SRT_LATIN: &str = "1\n00:00:01,000 --> 00:00:04,000\nJust latin text\n";
const VTT_FA: &str = "WEBVTT\n\n00:01.000 --> 00:04.000\nسلام دنیا\n";
const ASS_FA: &str = "[Script Info]\nTitle: Test\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,سلام دنیا\n";
const SSA_FA: &str = "[Script Info]\nTitle: Test\n\n[Events]\nFormat: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: Marked=0,0:00:01.00,0:00:04.00,Default,,0,0,0,,سلام دنیا\n";
const MICRODVD_FA: &str = "{1}{1}23.976\n{100}{200}سلام دنیا\n{250}{300}خط دوم\n";

fn get_fake_exe() -> PathBuf {
    static FAKE: OnceLock<PathBuf> = OnceLock::new();
    FAKE.get_or_init(|| {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let target_dir = PathBuf::from(manifest_dir).join("target").join("debug");
        fs::create_dir_all(&target_dir).unwrap();
        #[cfg(windows)]
        let exe_name = "fake_7z.exe";
        #[cfg(not(windows))]
        let exe_name = "fake_7z";
        let exe_path = target_dir.join(exe_name);
        let source_path = PathBuf::from(manifest_dir)
            .join("tests")
            .join("fake_7z.rs");
        // اگر منبع تازه‌تر از executable است، دوباره بساز تا تست‌ها همیشه
        // رفتار فعلی fake را ببینند (مهم برای اجرای محلی بعد از تغییر fake).
        let stale = match (fs::metadata(&exe_path), fs::metadata(&source_path)) {
            (Ok(exe), Ok(src)) => match (exe.modified(), src.modified()) {
                (Ok(exe_time), Ok(src_time)) => src_time > exe_time,
                _ => true,
            },
            _ => true,
        };
        if stale {
            let status = std::process::Command::new("rustc")
                .arg(&source_path)
                .arg("-o")
                .arg(&exe_path)
                .arg("--edition")
                .arg("2021")
                .status();
            assert!(
                status.map(|s| s.success()).unwrap_or(false) && exe_path.exists(),
                "fake_7z executable not found at {:?}; run: rustc tests/fake_7z.rs -o target/debug/fake_7z.exe --edition 2021",
                exe_path
            );
        }
        exe_path
    })
    .clone()
}

fn unique_test_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "subsync-archive-test-{tag}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn file_member(path: &str, content: &[u8]) -> String {
    file_member_with(path, content, "")
}

fn file_member_with(path: &str, content: &[u8], flags: &str) -> String {
    file_member_declared(path, &content.len().to_string(), content, flags)
}

/// عضو با اندازهٔ اعلامی دلخواه (برای تست metadata غایب/نامعتبر/سرریز/بزرگ).
fn file_member_declared(path: &str, declared: &str, content: &[u8], flags: &str) -> String {
    format!(
        "F|{}|{}||{}|{}|{}|{}|{}",
        path,
        declared,
        "",
        "",
        "",
        hex(content),
        flags
    )
}

fn unsafe_member(path: &str, attrs: &str, symlink: &str, hardlink: &str, reparse: &str) -> String {
    format!(
        "F|{}|1|{}|{}|{}|{}||",
        path, attrs, symlink, hardlink, reparse
    )
}

fn utf16_le(text: &str) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

struct Fixture {
    base: PathBuf,
    library: PathBuf,
    staging: PathBuf,
    sub: PathBuf,
    video: PathBuf,
    video_name: String,
    season: Option<u32>,
    episode: Option<u32>,
}

impl Fixture {
    fn new(tag: &str, video_name: &str, season: Option<u32>, episode: Option<u32>) -> Self {
        let base = unique_test_dir(tag);
        let library = base.join("library");
        let staging = base.join("_subsource_batch_fixture");
        let sub = staging.join("sub");
        fs::create_dir_all(&library).unwrap();
        fs::create_dir_all(&sub).unwrap();
        let video = library.join(video_name);
        fs::write(&video, b"ORIGINAL_VIDEO_BYTES").unwrap();
        Self {
            base,
            library,
            staging,
            sub,
            video,
            video_name: video_name.to_string(),
            season,
            episode,
        }
    }

    fn source(&self) -> ArchiveSource {
        ArchiveSource {
            video_path: self.video.to_string_lossy().to_string(),
            video_file_name: self.video_name.clone(),
            staging_root: self.staging.to_string_lossy().to_string(),
            relative_video_path: self.video_name.clone(),
            relative_video_dir: String::new(),
            season: self.season,
            episode: self.episode,
        }
    }

    fn archive(&self, name: &str, magic: &[u8], members: &[String]) -> PathBuf {
        let mut bytes = magic.to_vec();
        bytes.extend_from_slice(members.join("\n").as_bytes());
        let path = self.sub.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn mode(&self, modes: &[&str]) {
        fs::write(self.sub.join(".fake_7z_mode"), modes.join("\n")).unwrap();
    }

    fn target(&self, ext: &str) -> PathBuf {
        let stem = Path::new(&self.video_name)
            .file_stem()
            .unwrap()
            .to_string_lossy();
        self.library.join(format!("{stem}.fa.{ext}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

fn run_plan(fake: &Path, fixture: &Fixture) -> ArchiveExtractionPlan {
    let source = fixture.source();
    let cancel = AtomicBool::new(false);
    plan_archives(fake, &[source], &cancel, &mut |_p| {})
}

fn run_execute(
    fake: &Path,
    fixture: &Fixture,
    selection: Option<ArchiveSelection>,
) -> ArchiveExecuteResult {
    let request = ArchiveExecuteRequest {
        source: fixture.source(),
        selection,
    };
    let cancel = AtomicBool::new(false);
    execute_archive_extraction(fake, &[request], &cancel, &mut |_p| {})
}

fn selection_for(candidate: &SubtitleCandidate) -> ArchiveSelection {
    ArchiveSelection {
        archive_file_name: candidate.archive_file_name.clone(),
        member_path: candidate.member_path.clone(),
    }
}

fn assert_no_extract_dirs(staging: &Path) {
    if let Ok(entries) = fs::read_dir(staging) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with("_subsource_extract_"),
                "pوشهٔ موقت استخراج باقی مانده است: {name}"
            );
        }
    }
}

fn statuses(result: &ArchiveExecuteResult) -> Vec<ArchiveItemStatus> {
    result.items.iter().map(|i| i.status).collect()
}

#[test]
fn plan_and_execute_places_unique_srt_and_preserves_sources() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("place", "Show.S01E01.mkv", Some(1), Some(1));
    let archive_path = fixture.archive(
        "Show.S01E01_sub_1.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let archive_before = fs::read(&archive_path).unwrap();
    let video_before = fs::read(&fixture.video).unwrap();

    let plan = run_plan(&fake, &fixture);
    assert_eq!(plan.ready, 1, "{:?}", plan.items);
    assert_eq!(plan.items[0].status, ArchivePlanStatus::Ready);
    let candidate = plan.items[0].selection.clone().expect("auto selection");
    assert_eq!(candidate.member_path, "Show.S01E01.fa.srt");
    assert_eq!(candidate.subtitle_extension, "srt");
    assert_no_extract_dirs(&fixture.staging);

    let result = run_execute(&fake, &fixture, None);
    assert!(result.success, "{result:?}");
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Placed]);
    assert_eq!(fs::read(fixture.target("srt")).unwrap(), SRT_FA.as_bytes());
    assert_eq!(fs::read(&archive_path).unwrap(), archive_before);
    assert_eq!(fs::read(&fixture.video).unwrap(), video_before);
    assert_no_extract_dirs(&fixture.staging);
}

#[test]
fn magic_detection_ignores_extension_and_reports_corrupt_or_unsupported() {
    let fake = get_fake_exe();

    // RAR5 با نام .bin — فقط magic مهم است
    let disguised = Fixture::new("magic-rar", "Show.S01E01.mkv", Some(1), Some(1));
    disguised.archive(
        "downloaded_payload.bin",
        RAR5,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let plan = run_plan(&fake, &disguised);
    assert_eq!(plan.ready, 1, "{:?}", plan.items);
    assert_eq!(plan.items[0].archives[0].format, "rar5");

    // فایل با پسوند zip اما بدون magic → هشدار و NoMatch
    let fake_zip = Fixture::new("magic-fake-zip", "Show.S01E01.mkv", Some(1), Some(1));
    fs::write(fake_zip.sub.join("fake.zip"), b"this is not a zip").unwrap();
    let plan = run_plan(&fake, &fake_zip);
    assert_eq!(plan.no_match, 1);
    assert!(
        plan.items[0].message.contains("امضای"),
        "{}",
        plan.items[0].message
    );

    // فرمت پشتیبانی‌نشده 7z → هشدار، بدون استخراج
    let unsupported = Fixture::new("magic-7z", "Show.S01E01.mkv", Some(1), Some(1));
    let mut seven = SEVEN_ZIP_MAGIC.to_vec();
    seven.extend_from_slice(b"whatever");
    fs::write(unsupported.sub.join("download.7z"), seven).unwrap();
    let plan = run_plan(&fake, &unsupported);
    assert_eq!(plan.no_match, 1);
    assert!(
        plan.items[0].message.contains("پشتیبانی‌نشده") || plan.items[0].message.contains("امضا"),
        "{}",
        plan.items[0].message
    );

    // magic معتبر اما محتوای خراب (7z واقعی فهرست خالی/خطا می‌دهد)
    let corrupt = Fixture::new("magic-corrupt", "Show.S01E01.mkv", Some(1), Some(1));
    corrupt.archive("broken.zip", ZIP, &["garbage".to_string()]);
    let plan = run_plan(&fake, &corrupt);
    assert_eq!(plan.no_match, 1);
    assert!(
        plan.items[0].message.contains("آرشیو"),
        "{}",
        plan.items[0].message
    );
    assert_no_extract_dirs(&corrupt.staging);
}

#[test]
fn unsafe_archive_members_are_rejected_before_extraction() {
    let fake = get_fake_exe();
    let cases: Vec<(&str, String)> = vec![
        ("traversal", file_member("../evil.srt", SRT_FA.as_bytes())),
        ("absolute", file_member("/abs/evil.srt", SRT_FA.as_bytes())),
        ("drive", file_member("C:\\evil.srt", SRT_FA.as_bytes())),
        (
            "unc",
            file_member("\\\\server\\share\\evil.srt", SRT_FA.as_bytes()),
        ),
        (
            "ads",
            file_member("Show.S01E01.fa.srt:stream", SRT_FA.as_bytes()),
        ),
        ("nul", file_member("bad\0name.srt", SRT_FA.as_bytes())),
        ("reserved", file_member("CON.srt", SRT_FA.as_bytes())),
        (
            "attrs-symlink",
            unsafe_member("Show.S01E01.fa.srt", "L", "", "", ""),
        ),
        (
            "symbolic-target",
            unsafe_member("Show.S01E01.fa.srt", "", "C:\\Windows", "", ""),
        ),
        (
            "hardlink",
            unsafe_member("Show.S01E01.fa.srt", "", "", "other.srt", ""),
        ),
        (
            "reparse",
            unsafe_member("Show.S01E01.fa.srt", "", "", "", "target"),
        ),
    ];

    for (tag, member) in cases {
        let fixture = Fixture::new(
            &format!("unsafe-{tag}"),
            "Show.S01E01.mkv",
            Some(1),
            Some(1),
        );
        fixture.archive("Show.S01E01_sub_1.zip", ZIP, &[member]);
        let plan = run_plan(&fake, &fixture);
        assert_eq!(plan.no_match, 1, "case {tag}: {:?}", plan.items);
        assert!(
            plan.items[0].message.contains("آرشیو"),
            "case {tag}: {}",
            plan.items[0].message
        );
        assert!(
            !fixture.target("srt").exists(),
            "case {tag}: unsafe archive must not be placed"
        );
        assert!(
            !fixture.staging.join("evil.srt").exists()
                && !fixture.base.join("evil.srt").exists()
                && !fixture.library.join("evil.srt").exists(),
            "case {tag}: path traversal escaped"
        );
        assert_no_extract_dirs(&fixture.staging);
    }
}

#[test]
fn strict_episode_matching_forms() {
    let fake = get_fake_exe();
    let cases = [
        ("Show.S01E01.mkv", "Show.S1E1.fa.srt", true),
        ("Show.S01E01.mkv", "Show.S01E010.fa.srt", false),
        ("Show.S02E05.mkv", "Show.S01E05.fa.srt", false),
        ("Show.2x05.mkv", "Show.S02E05.fa.srt", true),
        ("Show.S01E01.mkv", "Show.S01E01-12.fa.srt", false),
        ("Show.S01E01.mkv", "Show.S01E01E02.fa.srt", false),
        ("Show.S01E01E02.mkv", "Show.S01E01E02.fa.srt", true),
        ("Show.S01E01E02.mkv", "Show.S01E01-03.fa.srt", false),
        // زنجیرهٔ سه‌قسمتی نباید به دو قسمت کوتاه شود
        ("Show.S01E01E02.mkv", "Show.S01E01E02E03.fa.srt", false),
        ("Show.S01E01E02E03.mkv", "Show.S01E01E02E03.fa.srt", true),
        ("Show.S01E01E02E03.mkv", "Show.S1E1E2E3.fa.srt", true),
        // فهرست صریح [1, 12] با بازهٔ پیوستهٔ 1..12 یکی نیست
        ("Show.S01E01E12.mkv", "Show.S01E01-12.fa.srt", false),
        ("Show.S01E01-12.mkv", "Show.S01E01-E12.fa.srt", true),
        // نشانگر بازهٔ خراب/خارج از پشتیبانی نباید به Single تبدیل شود
        ("Show.S01E01.mkv", "Show.S01E01-E1000.fa.srt", false),
        ("Show.S01E01.mkv", "Show.S01E01-E02E03.fa.srt", false),
        // نشانگر E ناقص هم نباید تطبیق دهد
        ("Show.S01E01.mkv", "Show.S01E01-E.fa.srt", false),
        ("Show.S01E01.mkv", "Show.S01E01-12E.fa.srt", false),
        ("Show.S01E01E02.mkv", "Show.S01E01E02-E.fa.srt", false),
    ];

    for (idx, (video_name, member_path, should_match)) in cases.iter().enumerate() {
        let fixture = Fixture::new(&format!("match-{idx}"), video_name, Some(1), Some(1));
        fixture.archive(
            "show_sub.zip",
            ZIP,
            &[file_member(member_path, SRT_FA.as_bytes())],
        );
        let plan = run_plan(&fake, &fixture);
        if *should_match {
            assert_eq!(
                plan.ready, 1,
                "{video_name} / {member_path}: {:?}",
                plan.items
            );
        } else {
            assert_eq!(
                plan.no_match, 1,
                "{video_name} / {member_path}: {:?}",
                plan.items
            );
        }
    }
}

#[test]
fn ambiguous_candidates_require_explicit_selection_and_rank_1080p_first() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("ambiguous", "Show.S01E01.mkv", Some(1), Some(1));
    fixture.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    fixture.archive(
        "b.1080p.zip",
        ZIP,
        &[file_member(
            "Show.S01E01.fa.srt",
            "1\n00:00:01,000 --> 00:00:04,000\nزیرنویس ۱۰۸۰p\n".as_bytes(),
        )],
    );

    let plan = run_plan(&fake, &fixture);
    assert_eq!(plan.ambiguous, 1);
    assert_eq!(plan.items[0].candidates.len(), 2);
    assert_eq!(plan.items[0].candidates[0].archive_file_name, "b.1080p.zip");
    assert_eq!(plan.items[0].candidates[0].quality, "1080p");

    let no_selection = run_execute(&fake, &fixture, None);
    assert_eq!(statuses(&no_selection), vec![ArchiveItemStatus::Ambiguous]);
    assert!(!fixture.target("srt").exists());

    let chosen = plan.items[0].candidates[0].clone();
    let placed = run_execute(&fake, &fixture, Some(selection_for(&chosen)));
    assert_eq!(statuses(&placed), vec![ArchiveItemStatus::Placed]);
    assert!(fs::read_to_string(fixture.target("srt"))
        .unwrap()
        .contains("زیرنویس ۱۰۸۰p"));

    // انتخاب دستکاری‌شده هرگز اجرا نمی‌شود
    let tampered_fixture = Fixture::new("tampered", "Show.S01E01.mkv", Some(1), Some(1));
    tampered_fixture.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let tampered = run_execute(
        &fake,
        &tampered_fixture,
        Some(ArchiveSelection {
            archive_file_name: "a.zip".into(),
            member_path: "Show.S01E02.fa.srt".into(),
        }),
    );
    assert_eq!(statuses(&tampered), vec![ArchiveItemStatus::Failed]);
    assert!(!tampered_fixture.target("srt").exists());
}

#[test]
fn supported_formats_validate_and_preserve_original_extension() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("formats", "Movie.2023.1080p.mkv", None, None);
    let utf16 = utf16_le(SRT_FA);
    let binary_vobsub: &[u8] = &[0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x00, 0x00];
    let members = vec![
        file_member("Movie.2023.1080p.fa.srt", SRT_FA.as_bytes()),
        file_member("Movie.2023.1080p.fa-utf16.srt", &utf16),
        file_member("Movie.2023.1080p.fa.vtt", VTT_FA.as_bytes()),
        file_member("Movie.2023.1080p.fa.ass", ASS_FA.as_bytes()),
        file_member("Movie.2023.1080p.fa.ssa", SSA_FA.as_bytes()),
        file_member("Movie.2023.1080p.fa.sub", MICRODVD_FA.as_bytes()),
        file_member("Movie.2023.1080p.fa.invalid.srt", b"not a subtitle at all"),
        file_member("Movie.2023.1080p.fa.latin.srt", SRT_LATIN.as_bytes()),
        file_member("Movie.2023.1080p.fa.binary.sub", binary_vobsub),
        file_member("Movie.2023.1080p.fa.txt", SRT_FA.as_bytes()),
    ];
    fixture.archive("movie_pack.zip", ZIP, &members);

    let plan = run_plan(&fake, &fixture);
    assert_eq!(plan.ambiguous, 1);
    let mut extensions: Vec<String> = plan.items[0]
        .candidates
        .iter()
        .map(|c| c.subtitle_extension.clone())
        .collect();
    extensions.sort();
    assert_eq!(
        extensions,
        vec!["ass", "srt", "srt", "ssa", "sub", "vtt"],
        "{:?}",
        plan.items[0].candidates
    );
    let message = &plan.items[0].message;
    assert!(message.contains("معتبر نیست"), "{message}");
    assert!(message.contains("فارسی"), "{message}");
    assert!(
        message.contains("باینری") || message.contains("VobSub"),
        "{message}"
    );

    let pick = |ext: &str, needle: &str| -> SubtitleCandidate {
        plan.items[0]
            .candidates
            .iter()
            .find(|c| c.subtitle_extension == ext && c.member_path.contains(needle))
            .cloned()
            .unwrap_or_else(|| panic!("candidate {ext}/{needle} not found"))
    };

    for (ext, needle, expected) in [
        ("vtt", ".fa.vtt", VTT_FA.as_bytes().to_vec()),
        ("ass", ".fa.ass", ASS_FA.as_bytes().to_vec()),
        ("ssa", ".fa.ssa", SSA_FA.as_bytes().to_vec()),
        ("sub", ".fa.sub", MICRODVD_FA.as_bytes().to_vec()),
        ("srt", "utf16", utf16.clone()),
    ] {
        let candidate = pick(ext, needle);
        let result = run_execute(&fake, &fixture, Some(selection_for(&candidate)));
        assert_eq!(statuses(&result), vec![ArchiveItemStatus::Placed]);
        assert_eq!(
            fs::read(fixture.target(ext)).unwrap(),
            expected,
            "extension/content mismatch for {ext}"
        );
    }

    // هرگز ASS/SSA/SUB به SRT تغییر نام داده نشده است.
    assert!(fixture.target("ass").exists());
    assert!(fixture.target("ssa").exists());
    assert!(fixture.target("sub").exists());
    assert!(!fixture.library.join("Movie.2023.1080p.fa.txt").exists());
    assert_no_extract_dirs(&fixture.staging);
}

#[test]
fn invalid_and_non_persian_subtitles_are_rejected() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("non-persian", "Show.S01E01.mkv", Some(1), Some(1));
    fixture.archive(
        "bad.zip",
        ZIP,
        &[
            file_member("Show.S01E01.fa.invalid.srt", b"not a subtitle"),
            file_member("Show.S01E01.fa.latin.srt", SRT_LATIN.as_bytes()),
        ],
    );

    let plan = run_plan(&fake, &fixture);
    assert_eq!(plan.no_match, 1);
    assert!(
        plan.items[0].message.contains("معتبر نیست"),
        "{}",
        plan.items[0].message
    );
    assert!(
        plan.items[0].message.contains("فارسی"),
        "{}",
        plan.items[0].message
    );

    let result = run_execute(&fake, &fixture, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::NoMatch]);
    assert!(!fixture.target("srt").exists());
    assert_no_extract_dirs(&fixture.staging);
}

#[test]
fn sevenzip_exit_codes_warning_vs_failure_and_partial() {
    let fake = get_fake_exe();

    // خروج ۱ در فهرست → هشدار اما استخراج ادامه دارد
    let list_warning = Fixture::new("exit-list-warn", "Show.S01E01.mkv", Some(1), Some(1));
    list_warning.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    list_warning.mode(&["list-warning"]);
    let plan = run_plan(&fake, &list_warning);
    assert_eq!(plan.ready, 1);
    assert!(
        plan.items[0].message.contains("خروج ۱"),
        "{}",
        plan.items[0].message
    );
    let result = run_execute(&fake, &list_warning, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Placed]);
    assert!(!result.warnings.is_empty());

    // خروج ۱ در استخراج → هشدار اما جای‌گذاری انجام می‌شود
    let extract_warning = Fixture::new("exit-extract-warn", "Show.S01E01.mkv", Some(1), Some(1));
    extract_warning.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    extract_warning.mode(&["extract-warning"]);
    let result = run_execute(&fake, &extract_warning, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Placed]);
    assert!(!result.warnings.is_empty());

    // خروج ۲ در استخراج → شکست، بدون خروجی، پاک‌سازی کامل
    let extract_fail = Fixture::new("exit-extract-fail", "Show.S01E01.mkv", Some(1), Some(1));
    let archive_path = extract_fail.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let archive_before = fs::read(&archive_path).unwrap();
    extract_fail.mode(&["extract-fail"]);
    let plan = run_plan(&fake, &extract_fail);
    assert_eq!(plan.no_match, 1);
    let result = run_execute(&fake, &extract_fail, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Failed]);
    assert!(
        result.items[0].message.contains("کد خروج 2"),
        "{:?}",
        result.items
    );
    assert!(!extract_fail.target("srt").exists());
    assert_eq!(fs::read(&archive_path).unwrap(), archive_before);
    assert_no_extract_dirs(&extract_fail.staging);

    // خروج ۲ در فهرست → شکست
    let list_fail = Fixture::new("exit-list-fail", "Show.S01E01.mkv", Some(1), Some(1));
    list_fail.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    list_fail.mode(&["list-fail"]);
    let result = run_execute(&fake, &list_fail, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Failed]);
    assert!(result.items[0].message.contains("کد خروج 2"));

    // استخراج جزئی (خروج ۱): یک عضو رد می‌شود، عضو سالم جای‌گذاری می‌شود
    let partial = Fixture::new("exit-partial", "Show.S01E01.mkv", Some(1), Some(1));
    partial.archive(
        "a.zip",
        ZIP,
        &[
            file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes()),
            file_member_with("Show.S01E01.fa.alt.srt", SRT_FA.as_bytes(), "SKIP"),
        ],
    );
    partial.mode(&["partial"]);
    let result = run_execute(&fake, &partial, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Placed]);
    assert!(!result.warnings.is_empty());
    assert!(partial.target("srt").exists());

    // استخراج جزئی که هیچ عضو سالمی ندارد → NoMatch، بدون فایل
    let all_skipped = Fixture::new("exit-all-skipped", "Show.S01E01.mkv", Some(1), Some(1));
    all_skipped.archive(
        "a.zip",
        ZIP,
        &[file_member_with(
            "Show.S01E01.fa.srt",
            SRT_FA.as_bytes(),
            "SKIP",
        )],
    );
    all_skipped.mode(&["partial"]);
    let result = run_execute(&fake, &all_skipped, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::NoMatch]);
    assert!(!all_skipped.target("srt").exists());
    assert_no_extract_dirs(&all_skipped.staging);
}

#[test]
fn existing_target_is_never_overwritten_and_race_has_one_winner() {
    let fake = get_fake_exe();

    let existing = Fixture::new("no-overwrite", "Show.S01E01.mkv", Some(1), Some(1));
    let archive_path = existing.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let archive_before = fs::read(&archive_path).unwrap();
    fs::write(existing.target("srt"), b"OLD SUBTITLE").unwrap();
    let result = run_execute(&fake, &existing, None);
    // تعارض حالا تایپ‌شده است: Conflict — مقصد حفظ شده و هیچ بازنویسی‌ای رخ نداده
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Conflict]);
    assert_eq!(
        result.items[0].output_path.as_deref(),
        Some(existing.target("srt").to_str().unwrap())
    );
    assert_eq!(
        fs::read(existing.target("srt")).unwrap(),
        b"OLD SUBTITLE".to_vec()
    );
    assert_eq!(fs::read(&archive_path).unwrap(), archive_before);
    assert_no_extract_dirs(&existing.staging);

    let race = Fixture::new("race", "Show.S01E01.mkv", Some(1), Some(1));
    race.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let fake_for_threads = fake.clone();
    let source = race.source();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let fake = fake_for_threads.clone();
            let source = source.clone();
            thread::spawn(move || {
                let request = ArchiveExecuteRequest {
                    source,
                    selection: None,
                };
                let cancel = AtomicBool::new(false);
                execute_archive_extraction(&fake, &[request], &cancel, &mut |_p| {})
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let placed = results.iter().filter(|r| r.placed == 1).count();
    // بازندهٔ مسابقه حالا Conflict تایپ‌شده می‌دهد — نه Failed
    let conflicts = results.iter().filter(|r| r.conflicts == 1).count();
    assert_eq!(placed, 1, "{results:?}");
    assert_eq!(conflicts, 1, "{results:?}");
    assert_eq!(fs::read(race.target("srt")).unwrap(), SRT_FA.as_bytes());
    assert_no_extract_dirs(&race.staging);
}

#[test]
fn cancellation_cleans_owned_temp_and_leaves_archive() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("cancel", "Show.S01E01.mkv", Some(1), Some(1));
    let archive_path = fixture.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let archive_before = fs::read(&archive_path).unwrap();
    fixture.mode(&["slow"]);

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = Arc::clone(&cancel);
    let fake_thread = fake.clone();
    let request = ArchiveExecuteRequest {
        source: fixture.source(),
        selection: None,
    };
    let handle = thread::spawn(move || {
        execute_archive_extraction(&fake_thread, &[request], &cancel_thread, &mut |_p| {})
    });

    thread::sleep(Duration::from_millis(250));
    cancel.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap();

    assert!(result.cancelled, "{result:?}");
    assert_eq!(result.placed, 0);
    assert!(!fixture.target("srt").exists());
    assert_eq!(fs::read(&archive_path).unwrap(), archive_before);
    assert_no_extract_dirs(&fixture.staging);
}

#[test]
fn duplicate_requests_and_empty_source_are_rejected() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("duplicate", "Show.S01E01.mkv", Some(1), Some(1));
    fixture.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    let request = ArchiveExecuteRequest {
        source: fixture.source(),
        selection: None,
    };
    let cancel = AtomicBool::new(false);
    let result =
        execute_archive_extraction(&fake, &[request.clone(), request], &cancel, &mut |_p| {});
    assert_eq!(result.placed, 1);
    assert_eq!(result.failed, 1);
    assert!(result.items[1].message.contains("تکراری"));
}

#[test]
fn persian_member_paths_are_preserved_with_utf8_listing() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("persian-member", "Show.S01E01.mkv", Some(1), Some(1));
    let member_path = "فصل اول/زیرنویس فارسی/Show.S01E01.fa.srt";
    fixture.archive("a.zip", ZIP, &[file_member(member_path, SRT_FA.as_bytes())]);

    let plan = run_plan(&fake, &fixture);
    assert_eq!(plan.ready, 1, "{:?}", plan.items);
    assert_eq!(
        plan.items[0].selection.as_ref().unwrap().member_path,
        member_path
    );

    let result = run_execute(&fake, &fixture, None);
    assert_eq!(statuses(&result), vec![ArchiveItemStatus::Placed]);
    assert_eq!(fs::read(fixture.target("srt")).unwrap(), SRT_FA.as_bytes());
    assert_no_extract_dirs(&fixture.staging);
}

#[test]
fn invalid_utf8_listing_is_rejected_without_lossy_matching() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("invalid-utf8", "Show.S01E01.mkv", Some(1), Some(1));
    fixture.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    fixture.mode(&["invalid-utf8"]);

    let plan = run_plan(&fake, &fixture);
    assert_eq!(plan.no_match, 1, "{:?}", plan.items);
    assert!(
        plan.items[0].message.contains("UTF-8"),
        "{}",
        plan.items[0].message
    );
    assert!(!fixture.target("srt").exists());
    assert_no_extract_dirs(&fixture.staging);
}

#[test]
fn declared_member_sizes_are_fail_closed() {
    let fake = get_fake_exe();
    for (tag, declared, needle) in [
        ("missing", "-", "اندازهٔ اعلام‌شده ندارد"),
        ("malformed", "abc", "نامعتبر"),
        ("overflow", "18446744073709551616", "نامعتبر"),
        ("over-limit", "536870913", "حد مجاز"),
    ] {
        let fixture = Fixture::new(
            &format!("declared-size-{tag}"),
            "Show.S01E01.mkv",
            Some(1),
            Some(1),
        );
        fixture.archive(
            "a.zip",
            ZIP,
            &[file_member_declared(
                "Show.S01E01.fa.srt",
                declared,
                SRT_FA.as_bytes(),
                "",
            )],
        );
        let plan = run_plan(&fake, &fixture);
        assert_eq!(plan.no_match, 1, "{tag}: {:?}", plan.items);
        assert!(
            plan.items[0].message.contains(needle),
            "{tag}: {}",
            plan.items[0].message
        );
        assert!(!fixture.target("srt").exists());
        assert_no_extract_dirs(&fixture.staging);
    }
}

#[test]
fn actual_extracted_size_and_file_count_are_enforced() {
    let fake = get_fake_exe();

    // listing می‌گوید یک فایل کوچک است؛ دیسک بیش از 512 MiB واقعی دارد.
    let inflated = Fixture::new("actual-size", "Show.S01E01.mkv", Some(1), Some(1));
    inflated.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    inflated.mode(&["inflated"]);
    let plan = run_plan(&fake, &inflated);
    assert_eq!(plan.no_match, 1, "{:?}", plan.items);
    assert!(
        plan.items[0].message.contains("حجم واقعی"),
        "{}",
        plan.items[0].message
    );
    assert!(!inflated.target("srt").exists());
    assert_no_extract_dirs(&inflated.staging);

    // listing فقط یک عضو دارد؛ استخراج هزاران فایل اضافی می‌سازد.
    let many = Fixture::new("actual-count", "Show.S01E01.mkv", Some(1), Some(1));
    many.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    many.mode(&["many-files"]);
    let plan = run_plan(&fake, &many);
    assert_eq!(plan.no_match, 1, "{:?}", plan.items);
    assert!(
        plan.items[0].message.contains("تعداد فایل"),
        "{}",
        plan.items[0].message
    );
    assert!(!many.target("srt").exists());
    assert_no_extract_dirs(&many.staging);
}

#[test]
fn plan_can_be_cancelled_through_shared_flag_and_cleans_temp() {
    let fake = get_fake_exe();
    let fixture = Fixture::new("plan-cancel", "Show.S01E01.mkv", Some(1), Some(1));
    fixture.archive(
        "a.zip",
        ZIP,
        &[file_member("Show.S01E01.fa.srt", SRT_FA.as_bytes())],
    );
    fixture.mode(&["slow"]);

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_thread = Arc::clone(&cancel);
    let fake_thread = fake.clone();
    let source = fixture.source();
    let handle =
        thread::spawn(move || plan_archives(&fake_thread, &[source], &cancel_thread, &mut |_p| {}));

    thread::sleep(Duration::from_millis(250));
    cancel.store(true, Ordering::SeqCst);
    let plan = handle.join().unwrap();

    assert!(plan.cancelled, "{plan:?}");
    assert!(plan.ready == 0);
    assert_no_extract_dirs(&fixture.staging);
}
