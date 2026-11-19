// نمایش pipeline — فقط برچسب و توصیف فارسی؛ هیچ منطق pipeline اینجا نیست.
// همهٔ منطق در بک‌اند است؛ فرانت فقط snapshot/رویدادها را رندر می‌کند و
// شناسه‌های مات (decisionId/optionId) را بدون تفسیر برمی‌گرداند.

import { fileBaseName } from "./videoFiles";
import type {
  DecisionKind,
  DecisionOptionPayload,
  ItemOutcome,
  PipelineStage,
  PipelineStatus,
  RegisteredOption,
  StageStatus,
} from "../types/pipeline";

/** ترتیب مراحل — قرینه barrier بک‌اند؛ برای ریل مرحله‌ها استفاده می‌شود. */
export const PIPELINE_STAGES: PipelineStage[] = [
  "validating",
  "scanning",
  "evaluatingExisting",
  "analyzing",
  "extracting",
  "downloading",
  "planningArchives",
  "extractingArchives",
  "selectingSyncInput",
  "syncing",
  "finalizing",
  "done",
];

export const PIPELINE_STAGE_LABELS: Record<PipelineStage, string> = {
  validating: "اعتبارسنجی",
  scanning: "اسکن فایل‌ها",
  evaluatingExisting: "بررسی زیرنویس‌های موجود",
  analyzing: "تحلیل زیرنویس داخلی",
  extracting: "استخراج زیرنویس",
  downloading: "دانلود از SubSource",
  planningArchives: "بررسی آرشیوها",
  extractingArchives: "استخراج از آرشیو",
  selectingSyncInput: "انتخاب ورودی سینک",
  syncing: "همگام‌سازی",
  finalizing: "جانمایی خروجی",
  done: "پایان",
};

export const PIPELINE_STATUS_LABELS: Record<PipelineStatus, string> = {
  running: "در حال اجرا",
  awaitingDecision: "منتظر تصمیم شما",
  completed: "کامل شد",
  cancelled: "لغو شد",
  failed: "ناموفق",
};

type BadgeTone = "neutral" | "success" | "warn" | "danger" | "info";

export function stageStatusBadge(status: StageStatus): {
  tone: BadgeTone;
  label: string;
} {
  switch (status.kind) {
    case "pending":
      return { tone: "neutral", label: "در انتظار" };
    case "inProgress":
      return { tone: "info", label: "در حال انجام" };
    case "done":
      return { tone: "success", label: "انجام شد" };
    case "skipped":
      return { tone: "neutral", label: "پرش" };
    case "failed":
      return { tone: "danger", label: "شکست مرحله" };
    case "needsDecision":
      return { tone: "warn", label: "نیازمند تصمیم شما" };
  }
}

/** outcome نهایی — برچسب صادقانه؛ هیچ حالت موفقی ساختگی نیست. */
export function outcomeBadge(outcome: ItemOutcome): {
  tone: BadgeTone;
  label: string;
} {
  switch (outcome.kind) {
    case "synced":
      return { tone: "success", label: "سینک شد" };
    case "softSubbed":
      return { tone: "success", label: "SoftSub شد" };
    case "alreadySynced":
      return { tone: "success", label: "از قبل سینک‌شده" };
    case "unchangedLowQuality":
      return { tone: "info", label: "کیفیت پایین — دست‌نخورده ماند" };
    case "needsApiKey":
      return { tone: "warn", label: "نیازمند کلید API" };
    case "keyStoreUnavailable":
      return { tone: "danger", label: "خطای مخزن کلید" };
    case "outputConflict":
      return { tone: "warn", label: "تعارض — هر دو فایل حفظ شد" };
    case "skipped":
      return { tone: "neutral", label: "پرش شد" };
    case "failed":
      return { tone: "danger", label: "ناموفق" };
    case "cancelled":
      return { tone: "neutral", label: "لغو شد" };
  }
}

export const DECISION_KIND_TEXT: Record<
  DecisionKind,
  { title: string; description: string; confirm: string }
> = {
  embeddedTrack: {
    title: "چند Track فارسی پیدا شد",
    description: "کدام Track زیرنویس داخلی استخراج شود؟",
    confirm: "استخراج این Track",
  },
  reviewTrack: {
    title: "Track متنی بدون نشانهٔ فارسی",
    description:
      "محتوای کدام Track برای تشخیص فارسی بودن بررسی شود؟ Track اشتباه ذخیره نمی‌شود.",
    confirm: "بررسی این Track",
  },
  archiveCandidate: {
    title: "چند زیرنویس منطبق در آرشیو",
    description: "کدام عضو آرشیو به‌عنوان زیرنویس ویدیو جانمایی شود؟",
    confirm: "انتخاب این زیرنویس",
  },
  syncSidecar: {
    title: "چند زیرنویس فارسی کنار ویدیو",
    description: "کدام زیرنویس مبنای همگام‌سازی قرار گیرد؟",
    confirm: "سینک با این زیرنویس",
  },
};

/** رشتهٔ غیرخالی از payload — فیلد گم‌شده هرگز باعث خطا یا دادهٔ ساختگی نمی‌شود. */
function text(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/** برچسب انسانی گزینه — فقط نام فایل/برچسب Track؛ هیچ مسیر کامل نمایش داده نمی‌شود. */
export function optionLabel(option: RegisteredOption): string {
  const payload: DecisionOptionPayload = option;
  switch (payload.kind) {
    case "embeddedTrack":
    case "reviewTrack":
      return (
        text(payload.label) ??
        (typeof payload.streamIndex === "number"
          ? `Track ${payload.streamIndex}`
          : "Track زیرنویس")
      );
    case "archiveCandidate": {
      const archive = fileBaseName(text(payload.archiveFileName));
      const member = text(payload.memberPath);
      if (archive && member) return `${archive} ← ${member}`;
      return archive || member || "عضو آرشیو";
    }
    case "sidecarPath":
      return fileBaseName(payload.path) || "زیرنویس کنار ویدیو";
  }
}

/** توضیح تکمیلی گزینه — مسیرهای موقت هرگز نشان داده نمی‌شوند. */
export function optionDetail(option: RegisteredOption): string | null {
  const payload: DecisionOptionPayload = option;
  if (payload.kind === "archiveCandidate") {
    const member = text(payload.memberPath);
    return member ? `عضو آرشیو: ${member}` : null;
  }
  return null;
}

/** توضیح outcome نهایی برای زیر ردیف — مسیرهای workspace موقت هرگز نمایش داده نمی‌شوند. */
export function outcomeDetail(outcome: ItemOutcome): string | null {
  switch (outcome.kind) {
    case "synced": {
      const name = fileBaseName(outcome.outputPath);
      return name ? `خروجی: ${name}` : null;
    }
    case "softSubbed": {
      const name = fileBaseName(outcome.videoPath);
      return name ? `زیرنویس فارسی داخل ${name} قرار گرفت` : "زیرنویس فارسی داخل ویدیو قرار گرفت";
    }
    case "outputConflict": {
      const name = fileBaseName(outcome.preservedPath);
      return name
        ? `فایل حفظ‌شده: ${name} — نسخهٔ سینک‌شدهٔ موقت هم نگه داشته شد`
        : "فایل موجود حفظ شد؛ نسخهٔ سینک‌شدهٔ موقت هم نگه داشته شد";
    }
    case "keyStoreUnavailable": {
      const reason = text(outcome.reason);
      return reason ? `مخزن کلید خطا داد: ${reason}` : "مخزن کلید خطا داد.";
    }
    case "skipped":
    case "failed":
      return text(outcome.reason);
    case "needsApiKey":
      return "برای دانلود از SubSource، کلید API را در تنظیمات ذخیره کنید و دوباره تلاش کنید.";
    default:
      return null;
  }
}

/** آفست گزارش‌شدهٔ سینک — مقدار واقعی بک‌اند؛ وقتی نیست چیزی ساخته نمی‌شود. */
export function formatOffset(offset: number): string {
  if (!Number.isFinite(offset)) return "—";
  const sign = offset >= 0 ? "+" : "−";
  const value = Math.abs(offset)
    .toFixed(3)
    .replace(".", "٫")
    .replace(/\d/g, (digit) => "۰۱۲۳۴۵۶۷۸۹"[Number(digit)]);
  return `${sign}${value} ثانیه`;
}
