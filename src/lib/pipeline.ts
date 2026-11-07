// مراحل سینک انسان‌پسند برای صفحهٔ خانه.
//
// تا وقتی orchestrator واقعی (فازهای بعدی: دانلود SubSource + سینک FFsubsync)
// وصل نشده، هیچ مسیری این وضعیت‌ها را تولید نمی‌کند — صفحهٔ خانه عمداً
// هیچ پیشرفت جعلی‌ای نشان نمی‌دهد.

export type SyncStage =
  | "scanning"
  | "finding"
  | "extracting"
  | "downloading"
  | "preparing"
  | "syncing"
  | "done";

export const SYNC_STAGE_LABELS: Record<SyncStage, string> = {
  scanning: "در حال بررسی فایل‌ها",
  finding: "در حال پیدا کردن زیرنویس فارسی",
  extracting: "در حال استخراج زیرنویس",
  downloading: "در حال دریافت زیرنویس فارسی",
  preparing: "در حال آماده‌سازی فایل‌ها",
  syncing: "در حال هماهنگ‌سازی زیرنویس",
  done: "تکمیل شد",
};

/** وضعیت یک اجرای سینک — فقط موتور واقعی این را تولید می‌کند. */
export interface SyncRunState {
  stage: SyncStage;
  /** نام فایل جاری برای نمایش (بدون مسیر کامل) */
  currentFile: string | null;
  completed: number;
  total: number;
}
