// پسوندهای ویدیویی پشتیبانی‌شده — قرینه `naming::VIDEO_EXTENSIONS` در Rust

export const SUPPORTED_VIDEO_EXTENSIONS = [
  "mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts", "webm", "mpg",
  "mpeg", "vob", "wmv", "flv",
] as const;

export function isSupportedVideoFile(path: string): boolean {
  const name = path.split(/[\\/]/).pop() ?? "";
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = name.slice(dot + 1).toLowerCase();
  return (SUPPORTED_VIDEO_EXTENSIONS as readonly string[]).includes(ext);
}

/**
 * نام پایانی مسیر — ورودی ممکن است از بک‌اند ناقص برسد؛ هرگز نباید throw کند.
 * (یک فیلد گم‌شده نباید کل UI را از ریشه جدا کند.)
 */
export function fileBaseName(path: string | null | undefined): string {
  if (typeof path !== "string" || path.length === 0) return "";
  return path.split(/[\\/]/).pop() ?? path;
}

export function folderBaseName(path: string | null | undefined): string {
  if (typeof path !== "string" || path.length === 0) return "";
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.split(/[\\/]/).pop() ?? trimmed;
}
