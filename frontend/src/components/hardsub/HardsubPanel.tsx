import { useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Film,
  Info,
  Loader2,
  Square,
} from "lucide-react";
import { Button, Card } from "../ui";
import { useHardsub } from "../../hooks/useHardsub";
import { faNum, formatDuration } from "../../lib/format";
import type { HardsubCounts, HardsubItemStatus } from "../../types/hardsub";

type Tone = "neutral" | "success" | "warn" | "danger" | "info";

const TONE_CHIP: Record<Tone, string> = {
  neutral: "bg-zinc-800/60 text-ink-3",
  success: "bg-emerald-500/10 text-emerald-400",
  warn: "bg-amber-500/10 text-amber-400",
  danger: "bg-red-500/10 text-red-400",
  info: "bg-sky-500/10 text-sky-400",
};

const TONE_DOT: Record<Tone, string> = {
  neutral: "bg-zinc-600",
  success: "bg-emerald-400",
  warn: "bg-amber-400",
  danger: "bg-red-400",
  info: "animate-pulse bg-sky-400",
};

/** برچسب صادقانهٔ وضعیت — درصد فقط وقتی مدت ویدیو معلوم است. */
function statusView(status: HardsubItemStatus): { label: string; tone: Tone } {
  switch (status.kind) {
    case "pending":
      return { label: "در انتظار", tone: "neutral" };
    case "extracting":
      return { label: "استخراج زیرنویس داخلی", tone: "info" };
    case "encoding":
      return {
        label:
          status.percent != null
            ? `در حال انکود (${faNum(status.percent)}٪)`
            : "در حال انکود",
        tone: "info",
      };
    case "done":
      return { label: "هاردساب شد", tone: "success" };
    case "conflict":
      return { label: "تعارض — خروجی موجود بود", tone: "warn" };
    case "failed":
      return { label: "ناموفق", tone: "danger" };
    case "cancelled":
      return { label: "لغو شد", tone: "neutral" };
  }
}

function StatusIcon({
  running,
  cancelling,
  cancelled,
}: {
  running: boolean;
  cancelling: boolean;
  cancelled: boolean;
}) {
  if (cancelling) return <Loader2 className="size-5 shrink-0 animate-spin text-red-400" />;
  if (running) return <Film className="size-5 shrink-0 text-sky-400" />;
  if (cancelled) return <Info className="size-5 shrink-0 text-ink-2" />;
  return <CheckCircle2 className="size-5 shrink-0 text-emerald-400" />;
}

const COUNT_LABELS: Array<[keyof HardsubCounts, string]> = [
  ["done", "هاردساب شد"],
  ["failed", "ناموفق"],
  ["conflicts", "تعارض"],
  ["cancelled", "لغو"],
];

/**
 * نمای هاردساب — پیشرفت واقعی از بک‌اند (خروجی `-progress` FFmpeg)؛
 * درصد فقط وقتی مدت ویدیو معلوم است و هرگز ۱۰۰ نمایش داده نمی‌شود تا
 * اعتبارسنجی نهایی و نصب، وضعیت Done بسازد.
 */
export function HardsubPanel() {
  const { snapshot, summary, runActive, busy, actionError, cancelHardsub, dismiss } =
    useHardsub();
  const [cancelBusy, setCancelBusy] = useState(false);

  if (!snapshot && !summary) return null;

  const data = summary
    ? {
        total: summary.items.length,
        completed: summary.items.filter((i) => i.status.kind !== "pending" && i.status.kind !== "extracting" && i.status.kind !== "encoding").length,
        counts: summary.counts,
        items: summary.items,
        cancelling: false,
        cancelled: summary.cancelled,
      }
    : snapshot
      ? {
          total: snapshot.total,
          completed: snapshot.completed,
          counts: snapshot.counts,
          items: snapshot.items,
          cancelling: snapshot.cancelling,
          cancelled: false,
        }
      : null;
  if (!data) return null;

  const percent =
    data.total > 0 ? Math.min(100, Math.round((data.completed / data.total) * 100)) : 0;

  return (
    <div className="w-full max-w-4xl space-y-5">
      <Card className="space-y-4 p-6">
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 items-center gap-2">
            <StatusIcon
              running={runActive}
              cancelling={data.cancelling}
              cancelled={data.cancelled}
            />
            <div className="min-w-0">
              <div className="text-base font-bold text-ink-1">
                {data.cancelling
                  ? "هاردساب در حال توقف…"
                  : runActive
                    ? "هاردساب در جریان است"
                    : data.cancelled
                      ? "هاردساب لغو شد"
                      : "هاردساب به پایان رسید"}
              </div>
              <div className="mt-0.5 text-xs text-ink-3">
                زیرنویس داخل تصویر حک می‌شود؛ خروجی فایل جدید کنار هر ویدیو است و
                فایل اصلی دست‌نخورده می‌ماند.
              </div>
            </div>
          </div>
          {runActive && (
            <Button
              variant="danger"
              onClick={async () => {
                setCancelBusy(true);
                await cancelHardsub();
                setCancelBusy(false);
              }}
              disabled={busy || data.cancelling || cancelBusy}
              loading={busy || cancelBusy}
              title="توقف هاردساب — فایل‌های اصلی دست‌نخورده می‌مانند"
            >
              <Square className="size-3.5" />
              {data.cancelling ? "در حال توقف" : "توقف"}
            </Button>
          )}
        </div>

        {data.total > 0 && (
          <div className="space-y-2">
            {runActive && (
              <div className="h-2 w-full overflow-hidden rounded-full bg-zinc-800">
                <div
                  className="h-full rounded-full bg-sky-500/70 transition-all"
                  style={{ width: `${percent}%` }}
                />
              </div>
            )}
            <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-sm text-ink-3">
              <span>
                {faNum(data.completed)} از {faNum(data.total)} فایل
                {runActive ? " بسته شده" : ""}
              </span>
              <span>
                {COUNT_LABELS.filter(([key]) => data.counts[key] > 0)
                  .map(([key, label]) => `${label}: ${faNum(data.counts[key])}`)
                  .join(" · ")}
              </span>
            </div>
            {runActive && (
              <p className="text-xs leading-5 text-ink-3">
                انکود ویدیو زمان‌بر است؛ درصد هر فایل از خروجی واقعی FFmpeg می‌آید و
                وقتی مدت ویدیو معلوم نباشد نشان داده نمی‌شود.
              </p>
            )}
          </div>
        )}

        {actionError && (
          <p className="flex items-start gap-1.5 text-xs leading-5 text-red-300">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            {actionError}
          </p>
        )}

        {!runActive && (
          <div className="flex justify-end">
            <Button onClick={dismiss}>بستن</Button>
          </div>
        )}
      </Card>

      {data.items.length > 0 && (
        <Card className="overflow-hidden">
          <div className="flex items-center justify-between border-b border-line px-5 py-3">
            <span className="text-sm font-semibold text-ink-1">فایل‌های هاردساب</span>
            <span className="text-xs text-ink-3">{faNum(data.items.length)} فایل</span>
          </div>
          <div>
            {data.items.slice(0, 300).map((item) => (
              <HardsubItemRow key={item.videoPath} item={item} />
            ))}
          </div>
        </Card>
      )}

      {summary && !runActive && summary.durationMs > 0 && (
        <p className="text-center text-xs text-ink-3">
          مدت اجرا: {formatDuration(summary.durationMs)}
        </p>
      )}
    </div>
  );
}

function HardsubItemRow({ item }: { item: { videoPath: string; videoFileName: string; status: HardsubItemStatus; encoder?: string } }) {
  const view = statusView(item.status);
  const reason =
    item.status.kind === "failed" ? item.status.reason : null;
  return (
    <div className="border-b border-line/60 px-5 py-3 last:border-0">
      <div className="flex items-center gap-2.5">
        <span className={`size-2.5 shrink-0 rounded-full ${TONE_DOT[view.tone]}`} />
        <span
          dir="ltr"
          className="min-w-0 flex-1 truncate text-start font-mono text-sm text-ink-1"
          title={item.videoPath}
        >
          {item.videoFileName}
        </span>
        {item.encoder && (
          <span className="hidden shrink-0 text-[11px] text-ink-3 sm:block" dir="ltr">
            {item.encoder}
          </span>
        )}
        <span className={`shrink-0 rounded-md px-2.5 py-1 text-xs ${TONE_CHIP[view.tone]}`}>
          {view.label}
        </span>
      </div>
      {reason && <div className="mt-1 ps-[18px] text-[11px] leading-5 text-ink-3">{reason}</div>}
    </div>
  );
}
