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

export function fileBaseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function folderBaseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.split(/[\\/]/).pop() ?? trimmed;
}
