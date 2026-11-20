// انواع هاردساب — قرینه دقیق serde structs بک‌اند (src-tauri/src/hardsub/types.rs).
// هاردساب عملیاتی اختیاری پس از سینک است: خروجی همیشه فایل جدید
// (<stem>.fa.hardsubbed.<ext>) است و فایل اصلی دست‌نخورده می‌ماند.

/** آیتم پلن هاردساب — یک ویدیو با منبع زیرنویس مشخص */
export interface HardsubPlanItem {
  videoPath: string;
  videoFileName: string;
  /** زیرنویس sidecar؛ null = منبع داخلی */
  subtitlePath: string | null;
  /** Track متنی فارسی داخلی (index گلوبال) — استخراج در زمان اجرا */
  embeddedStreamIndex?: number;
  outputPath: string;
}

/** مورد ردشده در پلن — با دلیل صادقانه */
export interface HardsubPlanSkip {
  path: string;
  reason: string;
}

/** پلن هاردساب — توصیه است؛ اجرا پلن تازهٔ خودش را می‌سازد */
export interface HardsubPlan {
  items: HardsubPlanItem[];
  skipped: HardsubPlanSkip[];
  /** انکودر نهایی انتخاب‌شده (اثبات‌شده با probe در حالت Auto) */
  encoder: string;
}

/** وضعیت یک آیتم در طول اجرا */
export type HardsubItemStatus =
  | { kind: "pending" }
  | { kind: "extracting" }
  /** درصد فقط وقتی مدت ویدیو معلوم است — هرگز ساختگی نیست */
  | { kind: "encoding"; percent: number | null }
  | { kind: "done"; outputPath: string }
  | { kind: "conflict" }
  | { kind: "failed"; reason: string }
  | { kind: "cancelled" };

/** snapshot یک آیتم — انکودر واقعی مصرف‌شدهٔ همان آیتم (بعد از fallback CPU) */
export interface HardsubItemSnapshot {
  videoPath: string;
  videoFileName: string;
  status: HardsubItemStatus;
  encoder?: string;
}

/** شمارنده‌های واقعی بک‌اند */
export interface HardsubCounts {
  done: number;
  failed: number;
  conflicts: number;
  cancelled: number;
}

/** snapshot کامل اجرا — برای رویداد hardsub:state و بازیابی get_hardsub_state */
export interface HardsubSnapshot {
  /** شمارهٔ یکنوای snapshot — رویداد قدیمی‌تر نباید وضعیت جدیدتر را overwrite کند */
  seq: number;
  runId: string;
  running: boolean;
  cancelling: boolean;
  total: number;
  completed: number;
  items: HardsubItemSnapshot[];
  counts: HardsubCounts;
}

/** خلاصهٔ نهایی اجرا — رویداد hardsub:done */
export interface HardsubSummary {
  runId: string;
  cancelled: boolean;
  counts: HardsubCounts;
  items: HardsubItemSnapshot[];
  durationMs: number;
}
