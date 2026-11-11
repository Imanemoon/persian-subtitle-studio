//! تشخیص فرمت آرشیو از روی magic bytes و کشف آرشیوهای معتبر داخل `sub/`.
//!
//! پسوند فایل هیچ نقشی در تشخیص ندارد؛ یک فایل با پسوند `.zip` و magic
//! ناشناخته «خراب/پشتیبانی‌نشده» گزارش می‌شود و یک فایل بدون پسوند آشنا با
//! magic معتبر ZIP/RAR پذیرفته می‌شود.

use std::fs;
use std::path::{Path, PathBuf};

/// فرمت‌های آرشیوی پشتیبانی‌شده.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    Rar4,
    Rar5,
}

impl ArchiveFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            ArchiveFormat::Zip => "zip",
            ArchiveFormat::Rar4 => "rar4",
            ArchiveFormat::Rar5 => "rar5",
        }
    }
}

const ZIP_LOCAL: &[u8] = &[0x50, 0x4B, 0x03, 0x04];
const ZIP_EMPTY: &[u8] = &[0x50, 0x4B, 0x05, 0x06];
const ZIP_SPANNED: &[u8] = &[0x50, 0x4B, 0x07, 0x08];
const RAR4: &[u8] = &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00];
const RAR5: &[u8] = &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00];

/// تشخیص فرمت روی بایت‌های آغازین فایل.
pub fn detect_format(bytes: &[u8]) -> Option<ArchiveFormat> {
    if bytes.starts_with(ZIP_LOCAL)
        || bytes.starts_with(ZIP_EMPTY)
        || bytes.starts_with(ZIP_SPANNED)
    {
        return Some(ArchiveFormat::Zip);
    }
    if bytes.starts_with(RAR5) {
        return Some(ArchiveFormat::Rar5);
    }
    if bytes.starts_with(RAR4) {
        return Some(ArchiveFormat::Rar4);
    }
    None
}

/// خواندن magic یک فایل؛ خطای خواندن → `None` (فایل در گزارش هشدار می‌آید).
pub fn read_format(path: &Path) -> Option<ArchiveFormat> {
    let mut head = [0u8; 8];
    let mut file = fs::File::open(path).ok()?;
    use std::io::Read;
    let n = file.read(&mut head).ok()?;
    detect_format(&head[..n])
}

/// یک آرشیو کشف‌شده داخل `sub/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredArchive {
    pub file_name: String,
    pub path: PathBuf,
    pub format: ArchiveFormat,
    pub size_bytes: u64,
}

fn extension_lower(file_name: &str) -> String {
    Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// جست‌وجوی مستقیم (غیربازگشتی) آرشیوها در یک پوشهٔ `sub/`.
///
/// فقط فایل‌های معمولی بررسی می‌شوند؛ symlink/reparse اصلاً باز نمی‌شود.
/// فایل‌های با پسوند آرشیوی اما magic نامعتبر در `warnings` گزارش می‌شوند.
pub fn discover_archives(sub_dir: &Path) -> (Vec<DiscoveredArchive>, Vec<String>) {
    let mut archives = Vec::new();
    let mut warnings = Vec::new();

    let entries = match fs::read_dir(sub_dir) {
        Ok(e) => e,
        Err(e) => {
            warnings.push(format!(
                "خواندن پوشهٔ «{}» ناموفق بود: {e}",
                sub_dir.display()
            ));
            return (archives, warnings);
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!(
                    "یکی از entries پوشهٔ «{}» خوانده نشد: {e}",
                    sub_dir.display()
                ));
                continue;
            }
        };
        let file_name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        // file_type() روی DirEntry لینک را دنبال نمی‌کند.
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("نوع فایل «{file_name}» خوانده نشد: {e}"));
                continue;
            }
        };
        if file_type.is_symlink() {
            warnings.push(format!("«{file_name}» یک symlink است و نادیده گرفته شد"));
            continue;
        }
        if !file_type.is_file() {
            continue;
        }

        match read_format(&path) {
            Some(format) => {
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                archives.push(DiscoveredArchive {
                    file_name,
                    path,
                    format,
                    size_bytes,
                });
            }
            None => {
                let ext = extension_lower(&file_name);
                if ext == "zip" || ext == "rar" || ext == "7z" {
                    warnings.push(format!(
                        "«{file_name}» پسوند آرشیو دارد اما امضای ZIP/RAR معتبر ندارد — احتمال خرابی یا فرمت پشتیبانی‌نشده"
                    ));
                }
            }
        }
    }

    // ترتیب قطعی برای تست‌ها/پلن
    archives.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    (archives, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_magic_not_extension() {
        assert_eq!(
            detect_format(&[0x50, 0x4B, 0x03, 0x04, 0x00]),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            detect_format(&[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00]),
            Some(ArchiveFormat::Rar4)
        );
        assert_eq!(
            detect_format(&[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x01, 0x00]),
            Some(ArchiveFormat::Rar5)
        );
        assert_eq!(detect_format(b"7z\xBC\xAF\x27\x1C"), None);
        assert_eq!(detect_format(b"not an archive"), None);
    }
}
