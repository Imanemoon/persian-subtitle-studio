//! پارس خروجی `7z l -slt` و اعتبارسنجی امنیتی همهٔ اعضای آرشیو پیش از
//! استخراج.
//!
//! قواعد رد (بدون استثنا): مسیر مطلق/rooted، drive، UNC، `..`، ADS (`:`)،
//! بایت NUL، نام‌های رزرو ویندوز، symlink/hardlink/reparse، عضو رمزگذاری‌شده
//! و اندازهٔ غیرمعقول.

use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use super::types::MAX_ARCHIVE_UNPACKED_BYTES;

/// یک عضو خام از خروجی `-slt`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArchiveMember {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub encrypted: bool,
    pub attributes: String,
    pub symbolic_link: Option<String>,
    pub hard_link: Option<String>,
    pub reparse: Option<String>,
}

/// عضو امن پس از اعتبارسنجی مسیر.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeMember {
    /// مسیر اصلی گزارش‌شده توسط 7-Zip.
    pub path: String,
    /// مسیر نرمال‌شده با جداکننده `/` و بدون اجزای خطرناک.
    pub normalized: String,
    pub size: u64,
    pub is_dir: bool,
}

impl SafeMember {
    /// مسیر نسبی قابل‌join ایمن (هر جزء جداگانه).
    pub fn relative_path(&self) -> PathBuf {
        let mut out = PathBuf::new();
        for part in self.normalized.split('/') {
            out.push(part);
        }
        out
    }

    pub fn file_name(&self) -> &str {
        self.normalized
            .rsplit('/')
            .next()
            .unwrap_or(&self.normalized)
    }
}

/// پارس خروجی فنی 7-Zip. بلوک سربرگ آرشیو (دارای `Type`) نادیده گرفته می‌شود.
pub fn parse_listing(output: &str) -> Result<Vec<ArchiveMember>, String> {
    let mut members = Vec::new();
    let mut current: Option<ArchiveMember> = None;
    let mut is_header = false;

    let flush = |current: &mut Option<ArchiveMember>,
                 is_header: &mut bool,
                 members: &mut Vec<ArchiveMember>| {
        if let Some(member) = current.take() {
            if !member.path.is_empty() && !*is_header {
                members.push(member);
            }
        }
        *is_header = false;
    };

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(&mut current, &mut is_header, &mut members);
            continue;
        }
        if trimmed.starts_with("----------") {
            flush(&mut current, &mut is_header, &mut members);
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();

        if key == "path" {
            // اگر بلوک قبلی بدون خط خالی تمام شده، آن را ثبت کن.
            if current.as_ref().is_some_and(|m| !m.path.is_empty()) {
                flush(&mut current, &mut is_header, &mut members);
            }
            current.get_or_insert_with(ArchiveMember::default).path = value.to_string();
            continue;
        }

        if key == "type" {
            is_header = true;
            continue;
        }

        let Some(member) = current.as_mut() else {
            continue;
        };
        match key.as_str() {
            "size" => member.size = value.parse().unwrap_or(0),
            "folder" => member.is_dir = is_truthy(value),
            "encrypted" => member.encrypted = is_truthy(value),
            "attributes" => member.attributes = value.to_string(),
            "symbolic link" => member.symbolic_link = Some(value.to_string()),
            "hard link" => member.hard_link = Some(value.to_string()),
            "reparse" => member.reparse = Some(value.to_string()),
            _ => {}
        }
    }
    flush(&mut current, &mut is_header, &mut members);

    if members.is_empty() {
        return Err("خروجی فهرست 7-Zip هیچ عضوی نداشت".into());
    }
    Ok(members)
}

fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "+" | "1" | "true" | "yes"
    )
}

/// اعتبارسنجی همهٔ اعضا. اگر حتی یک عضو ناامن باشد، کل آرشیو رد می‌شود.
pub fn validate_members(members: &[ArchiveMember]) -> Result<Vec<SafeMember>, String> {
    if members.len() > 10_000 {
        return Err(format!(
            "آرشیو بیش از حد اعضا دارد ({} عضو) — پردازش متوقف شد",
            members.len()
        ));
    }

    let mut safe = Vec::with_capacity(members.len());
    let mut total_size: u64 = 0;

    for member in members {
        if member.encrypted {
            return Err(format!(
                "عضو «{}» رمزگذاری‌شده است — آرشیوهای رمزدار پشتیبانی نمی‌شوند",
                member.path
            ));
        }
        if member.symbolic_link.is_some() {
            return Err(format!(
                "عضو «{}» symlink است — به دلایل امنیتی رد شد",
                member.path
            ));
        }
        if member.hard_link.is_some() {
            return Err(format!(
                "عضو «{}» hard link است — به دلایل امنیتی رد شد",
                member.path
            ));
        }
        if member.reparse.is_some() {
            return Err(format!(
                "عضو «{}» reparse point است — به دلایل امنیتی رد شد",
                member.path
            ));
        }
        if member.attributes.to_ascii_uppercase().contains('L') {
            return Err(format!(
                "عضو «{}» نشانهٔ symlink/reparse دارد — رد شد",
                member.path
            ));
        }

        let normalized = validate_member_path(&member.path)?;
        total_size = total_size.saturating_add(member.size);
        if total_size > MAX_ARCHIVE_UNPACKED_BYTES {
            return Err(format!(
                "حجم بازشدهٔ آرشیو از حد مجاز ({} MiB) بیشتر است",
                MAX_ARCHIVE_UNPACKED_BYTES / (1024 * 1024)
            ));
        }

        safe.push(SafeMember {
            path: member.path.clone(),
            normalized,
            size: member.size,
            is_dir: member.is_dir,
        });
    }

    Ok(safe)
}

/// اعتبارسنجی یک مسیر عضو و برگرداندن شکل نرمال‌شده (`/`).
pub fn validate_member_path(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("عضو با مسیر خالی در آرشیو وجود دارد".into());
    }
    if path.contains('\0') {
        return Err("مسیر عضو شامل بایت NUL است — رد شد".into());
    }
    if path.len() > 1024 {
        return Err("مسیر عضو بیش از حد بلند است — رد شد".into());
    }

    // هر دو جداکننده ویندوز/یونیکس برای تحلیل یکسان می‌شوند.
    let unified = path.replace('\\', "/");

    if unified.starts_with('/') {
        return Err(format!("مسیر مطلق «{path}» در آرشیو رد شد"));
    }
    if unified.contains(':') {
        return Err(format!("مسیر «{path}» شامل drive/ADS است — رد شد"));
    }
    if unified
        .chars()
        .any(|c| matches!(c, '*' | '?' | '<' | '>' | '|' | '"'))
    {
        return Err(format!(
            "مسیر «{path}» شامل کاراکتر غیرمجاز ویندوز است — رد شد"
        ));
    }

    let mut parts = Vec::new();
    for part in unified.split('/') {
        if part.is_empty() {
            return Err(format!(
                "مسیر «{path}» جزء خالی/جداکنندهٔ تکراری دارد — رد شد"
            ));
        }
        if part == "." || part == ".." {
            return Err(format!(
                "مسیر «{path}» شامل «{part}» است — عبور از مسیر رد شد"
            ));
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return Err(format!("مسیر «{path}» با نقطه/فاصله تمام می‌شود — رد شد"));
        }
        if part.trim().is_empty() {
            return Err(format!("مسیر «{path}» جزء خالی دارد — رد شد"));
        }
        if part.len() > 255 {
            return Err(format!("مسیر «{path}» جزء بیش از حد بلند دارد — رد شد"));
        }
        if is_reserved_windows_name(part) {
            return Err(format!(
                "مسیر «{path}» از نام رزرو ویندوز استفاده می‌کند — رد شد"
            ));
        }
        parts.push(part);
    }

    if parts.is_empty() {
        return Err(format!("مسیر «{path}» معتبر نیست"));
    }
    Ok(parts.join("/"))
}

fn is_reserved_windows_name(part: &str) -> bool {
    let base = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && base.as_bytes()[3].is_ascii_digit())
}

/// آیا این متادیتا symlink/reparse است؟
pub fn is_link_or_reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
        if meta.file_type().is_symlink() {
            return true;
        }
        false
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

/// بررسی درخت استخراج‌شده: هیچ symlink/reparse و هیچ مسیر خارج از ریشه.
/// همهٔ فایل‌های معمولی برگردانده می‌شوند.
pub fn verify_extracted_tree(root: &Path) -> Result<Vec<PathBuf>, String> {
    let canonical_root = fs::canonicalize(root)
        .map_err(|e| format!("canonicalize ریشهٔ استخراج «{}» ناموفق: {e}", root.display()))?;
    let mut files = Vec::new();

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|e| format!("پیمایش درخت استخراج ناموفق: {e}"))?;
        let path = entry.path();
        let meta = fs::symlink_metadata(path)
            .map_err(|e| format!("خواندن متادیتای «{}» ناموفق: {e}", path.display()))?;
        if is_link_or_reparse(&meta) {
            return Err(format!(
                "فایل استخراج‌شده «{}» symlink/reparse است — رد شد",
                path.display()
            ));
        }
        if meta.is_file() {
            let canonical = fs::canonicalize(path).map_err(|e| {
                format!(
                    "canonicalize فایل استخراج‌شده «{}» ناموفق: {e}",
                    path.display()
                )
            })?;
            if !canonical.starts_with(&canonical_root) {
                return Err(format!(
                    "فایل استخراج‌شده «{}» از ریشهٔ مجاز خارج شده است",
                    path.display()
                ));
            }
            files.push(canonical);
        }
    }

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(path: &str) -> ArchiveMember {
        ArchiveMember {
            path: path.into(),
            ..Default::default()
        }
    }

    #[test]
    fn rejects_traversal_absolute_drive_unc_ads_and_nul() {
        for bad in [
            "../evil.srt",
            "a/../../evil.srt",
            "/etc/passwd",
            "\\windows\\system32\\evil.srt",
            "C:\\evil.srt",
            "C:evil.srt",
            "//server/share/evil.srt",
            "\\\\server\\share\\evil.srt",
            "file.srt:stream",
            ":stream",
            "bad\0name.srt",
            "CON.srt",
            "LPT1.srt",
            "trailing.srt.",
            "trailing.srt ",
            "a//b.srt",
            "a/./b.srt",
            "bad?.srt",
        ] {
            assert!(validate_member_path(bad).is_err(), "باید رد شود: {bad:?}");
        }
    }

    #[test]
    fn accepts_safe_nested_paths() {
        assert_eq!(
            validate_member_path("Season 01/Show.S01E01.fa.srt").unwrap(),
            "Season 01/Show.S01E01.fa.srt"
        );
        assert_eq!(
            validate_member_path("folder\\sub.srt").unwrap(),
            "folder/sub.srt"
        );
    }

    #[test]
    fn rejects_links_and_encrypted_members_even_with_safe_paths() {
        let mut link = member("safe.srt");
        link.symbolic_link = Some("C:\\Windows".into());
        assert!(validate_members(&[link]).is_err());

        let mut hard = member("safe.srt");
        hard.hard_link = Some("other.srt".into());
        assert!(validate_members(&[hard]).is_err());

        let mut reparse = member("safe.srt");
        reparse.reparse = Some("target".into());
        assert!(validate_members(&[reparse]).is_err());

        let mut encrypted = member("safe.srt");
        encrypted.encrypted = true;
        assert!(validate_members(&[encrypted]).is_err());

        let mut attr = member("safe.srt");
        attr.attributes = "A_ lrwxrwxrwx".into();
        assert!(validate_members(&[attr]).is_err());
    }

    #[test]
    fn parses_slt_blocks_and_ignores_header() {
        let output = "\
Path = archive.zip
Type = zip
Physical Size = 100

----------
Path = Show.S01E01.fa.srt
Size = 42
Folder = -
Attributes = A

Path = Show.S01E02.fa.srt
Size = 43
";
        let members = parse_listing(output).unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].path, "Show.S01E01.fa.srt");
        assert_eq!(members[0].size, 42);
        assert!(!members[0].is_dir);
        assert_eq!(members[1].path, "Show.S01E02.fa.srt");
    }

    #[test]
    fn parses_folder_flag_and_rejects_unsafe_listing() {
        let output = "Path = nested\nFolder = +\n\nPath = ../evil.srt\nSize = 1\n";
        let members = parse_listing(output).unwrap();
        assert!(members[0].is_dir);
        assert!(!members[0].encrypted);
        assert!(validate_members(&members).is_err());
    }
}
