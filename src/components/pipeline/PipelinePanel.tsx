import { useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  ChevronDown,
  Info,
  Loader2,
  Square,
  XCircle,
} from "lucide-react";
import { Badge, Button, Card, ProgressBar } from "../ui";
import { usePipeline } from "../../hooks/usePipeline";
import {
  DECISION_KIND_TEXT,
  PIPELINE_STAGES,
  PIPELINE_STAGE_LABELS,
  PIPELINE_STATUS_LABELS,
  formatOffset,
  optionDetail,
  optionLabel,
  outcomeBadge,
  outcomeDetail,
} from "../../lib/pipeline";
import { faNum, formatDuration } from "../../lib/format";
import { fileBaseName } from "../../lib/videoFiles";
import type {
  PipelineCounts,
  PipelineItemSnapshot,
  PipelineStatus,
  PipelineSummary,
} from "../../types/pipeline";
import type { PageId } from "../../types";

/**
 * نمای اجرای pipeline — همهٔ داده‌ها از بک‌اند می‌آید؛ هیچ درصد یا موفقیتی
 * ساختگی نیست. تمرکز این نما: «الان چه خبر است؟» و «چه چیزی از من می‌خواهد؟»
 */
export function PipelinePanel({
  onNavigate,
}: {
  onNavigate: (page: PageId) => void;
}) {
  const {
    snapshot,
    items,
    decision,
    summary,
    runActive,
    busy,
    actionError,
    resolveDecision,
    cancelPipeline,
    dismissSummary,
  } = usePipeline();
  const [selectedOption, setSelectedOption] = useState<string | null>(null);

  if (!snapshot && !summary) return null;

  const status: PipelineStatus | undefined = summary?.status ?? snapshot?.status;
  const counts = summary?.counts ?? snapshot?.counts;
  const message = snapshot?.message;
  const currentFile = snapshot?.currentFile ?? null;
  const stage = snapshot?.stage;
  const total = summary?.items.length ?? snapshot?.total ?? 0;
  const completed = summary
    ? summary.items.filter((item) => item.outcome !== null).length
    : (snapshot?.completed ?? 0);
  const remaining = Math.max(0, total - completed);
  const active = runActive ? (snapshot?.active ?? 0) : 0;
  const cancelling = Boolean(snapshot?.cancelling);
  const percent = total > 0 ? Math.min(100, Math.round((completed / total) * 100)) : 0;
  const stageIndex = stage ? PIPELINE_STAGES.indexOf(stage) : -1;
  const waiting = status === "awaitingDecision" || decision !== null;

  return (
    <div className="w-full max-w-4xl space-y-5">
      {/* ---- وضعیت اصلی: الان چه کار می‌کند؟ ---- */}
      <Card className="space-y-4 p-6">
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 items-center gap-2">
            <StatusIcon status={status} cancelling={cancelling} />
            <div className="min-w-0">
              <div className="text-base font-bold text-ink-1">
                {cancelling
                  ? "در حال توقف…"
                  : status
                    ? PIPELINE_STATUS_LABELS[status]
                    : ""}
              </div>
              {runActive && (
                <div className="mt-0.5 text-xs text-ink-3">
                  {cancelling
                    ? "درخواست توقف ثبت شد؛ فایل‌های اصلی دست‌نخورده می‌مانند."
                    : waiting
                      ? "پردازش متوقف است تا انتخاب شما ثبت شود."
                      : "در حال پردازش — نشانگر فعالیت یعنی برنامه زنده است."}
                </div>
              )}
            </div>
          </div>
          {runActive && (
            <Button
              variant="danger"
              onClick={() => void cancelPipeline()}
              disabled={busy || cancelling}
              loading={busy}
              title="توقف pipeline — فایل‌های اصلی دست‌نخورده می‌مانند"
            >
              <Square className="size-3.5" />
              {cancelling ? "در حال توقف" : "توقف"}
            </Button>
          )}
        </div>

        {message && !summary && (
          <p className="text-sm leading-7 text-ink-2">{message}</p>
        )}
        {summary && (
          <p className="text-sm leading-7 text-ink-2">
            {summary.status === "cancelled"
              ? "اجرا با درخواست شما متوقف شد؛ ورودی‌ها دست‌نخورده ماندند."
              : summary.status === "failed"
                ? "اجرا با خطا متوقف شد؛ جزئیات در بخش «نیازمند بررسی» آمده است."
                : "همهٔ فایل‌ها به نتیجهٔ نهایی رسیدند."}
          </p>
        )}

        {currentFile && !summary && (
          <div className="flex items-center gap-2 rounded-lg border border-line bg-surface-2/60 px-3 py-2.5">
            <span className="shrink-0 text-xs text-ink-3">فایل جاری</span>
            <span
              dir="ltr"
              className="min-w-0 flex-1 truncate text-start font-mono text-xs text-ink-1"
              title={currentFile}
            >
              {fileBaseName(currentFile)}
            </span>
            {active > 1 && (
              <span className="shrink-0 text-xs text-ink-3">
                {faNum(active)} فایل هم‌زمان
              </span>
            )}
          </div>
        )}

        {/* ریل فشردهٔ مراحل — فقط یک نگاه، بدون ادعای تکمیل نتیجه */}
        {stage && !summary && (
          <div className="space-y-2">
            <div className="flex items-center justify-between text-sm">
              <span className="font-semibold text-ink-1">
                {PIPELINE_STAGE_LABELS[stage]}
              </span>
              <span className="text-ink-3">
                مرحلهٔ {faNum(stageIndex + 1)} از {faNum(PIPELINE_STAGES.length)}
              </span>
            </div>
            <div className="flex gap-1.5" role="img" aria-label="مراحل اجرا">
              {PIPELINE_STAGES.map((s, index) => (
                <span
                  key={s}
                  title={PIPELINE_STAGE_LABELS[s]}
                  className={`h-2 flex-1 rounded-full ${
                    index < stageIndex
                      ? "bg-emerald-500/70"
                      : index === stageIndex
                        ? cancelling
                          ? "bg-red-400"
                          : "animate-pulse bg-accent"
                        : "bg-zinc-800"
                  }`}
                />
              ))}
            </div>
            {/* شمارش واقعی آیتم‌های مرحلهٔ جاری — بک‌اند منبع حقیقت است */}
            {runActive &&
              !waiting &&
              snapshot?.stageProgress &&
              snapshot.stageProgress.total > 0 && (
                <StageProgressBar
                  completed={snapshot.stageProgress.completed}
                  total={snapshot.stageProgress.total}
                />
              )}
          </div>
        )}

        {/* شمارش صادقانه — تکمیل فقط با outcome نهایی */}
        {total > 0 && (
          <div className="space-y-2">
            {(runActive || status === "completed") && (
              <div className="h-2 w-full overflow-hidden rounded-full bg-zinc-800">
                <div
                  className="h-full rounded-full bg-accent transition-all"
                  style={{ width: `${percent}%` }}
                />
              </div>
            )}
            <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-sm text-ink-3">
              {status === "cancelled" ? (
                <span>
                  {faNum(completed)} از {faNum(total)} فایل بسته شد — اجرا پیش از
                  تکمیل متوقف شد
                </span>
              ) : status === "failed" ? (
                <span>
                  {faNum(completed)} از {faNum(total)} فایل به نتیجه رسید — اجرا
                  ناموفق ماند
                </span>
              ) : (
                <>
                  <span>{faNum(percent)}٪ بر اساس نتیجهٔ نهایی</span>
                  <span>
                    {faNum(completed)} از {faNum(total)} تکمیل شد
                    {" · "}
                    {faNum(remaining)} باقی‌مانده
                    {active > 0 ? ` · ${faNum(active)} در حال پردازش` : ""}
                  </span>
                </>
              )}
            </div>
            {runActive && !waiting && (
              <div className="space-y-1 pt-0.5">
                <ProgressBar active={!cancelling} />
                <p className="text-xs leading-5 text-ink-3">
                  {cancelling
                    ? "در حال خاتمهٔ امن عملیات جاری…"
                    : stage === "syncing"
                      ? "FFsubsync درصد دقیقی گزارش نمی‌کند؛ تا پایان هر فایل نشانگر فعالیت ادامه دارد."
                      : "این مرحله ممکن است چند دقیقه طول بکشد."}
                </p>
              </div>
            )}
          </div>
        )}

        {actionError && (
          <p className="flex items-start gap-1.5 text-xs leading-5 text-red-300">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            {actionError}
          </p>
        )}
      </Card>

      {/* ---- تصمیم باز: مهم‌ترین کنش کاربر ---- */}
      {decision && (
        <Card className="space-y-3 border-amber-500/40 p-5">
          <div className="flex items-start gap-2">
            <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-400" />
            <div>
              <div className="text-sm font-bold text-amber-300">
                اجرا متوقف شده — منتظر انتخاب شماست
              </div>
              <div className="mt-1 text-xs text-ink-2">
                {DECISION_KIND_TEXT[decision.kind].title} — {decision.videoFileName}
              </div>
            </div>
          </div>
          <p className="text-xs leading-5 text-ink-2">
            {DECISION_KIND_TEXT[decision.kind].description}
          </p>
          <div className="space-y-1.5">
            {decision.options.map((option) => (
              <button
                key={option.optionId}
                onClick={() => setSelectedOption(option.optionId)}
                className={`flex w-full flex-col gap-0.5 rounded-lg border px-3 py-2.5 text-start transition-colors ${
                  selectedOption === option.optionId
                    ? "border-accent bg-accent-dim"
                    : "border-line bg-surface-2 hover:border-ink-3"
                }`}
              >
                <span
                  dir={option.kind === "archiveCandidate" ? "ltr" : undefined}
                  className={`text-sm text-ink-1 ${option.kind === "archiveCandidate" ? "font-mono text-xs" : ""}`}
                >
                  {optionLabel(option)}
                </span>
                {optionDetail(option) && (
                  <span
                    dir="ltr"
                    className="truncate text-start font-mono text-[11px] text-ink-3"
                  >
                    {optionDetail(option)}
                  </span>
                )}
              </button>
            ))}
          </div>
          <div className="flex items-center justify-end gap-2">
            <Button
              variant="primary"
              disabled={!selectedOption || busy}
              loading={busy}
              onClick={() => {
                if (selectedOption) void resolveDecision(selectedOption);
              }}
            >
              {DECISION_KIND_TEXT[decision.kind].confirm}
            </Button>
          </div>
        </Card>
      )}

      {/* ---- فایل‌ها: خلاصه در یک نگاه، جزئیات فقط در صورت نیاز ---- */}
      {items.length > 0 && (
        <Card className="overflow-hidden">
          <div className="flex items-center justify-between border-b border-line px-5 py-3">
            <span className="text-sm font-semibold text-ink-1">فایل‌های این اجرا</span>
            <span className="text-xs text-ink-3">
              {faNum(items.length)} فایل
            </span>
          </div>
          <div>
            {items.slice(0, 300).map((item) => (
              <ItemRow key={item.videoPath} item={item} />
            ))}
          </div>
        </Card>
      )}

      {/* ---- خلاصهٔ پایان: شمارش واقعی + موارد نیازمند بررسی ---- */}
      {summary && !runActive && counts && (
        <Card className="space-y-4 p-6">
          <div className="flex items-center justify-between gap-3">
            <span className="text-base font-semibold text-ink-1">
              خلاصهٔ اجرا ({formatDuration(summary.durationMs)})
            </span>
            <Badge
              tone={
                summary.status === "completed"
                  ? "success"
                  : summary.status === "failed"
                    ? "danger"
                    : "neutral"
              }
            >
              {PIPELINE_STATUS_LABELS[summary.status]}
            </Badge>
          </div>

          <CountsGrid counts={counts} />

          <NeedsReview summary={summary} />

          {counts.needsApiKey > 0 && (
            <Button variant="primary" onClick={() => onNavigate("settings")}>
              تنظیم کلید API SubSource
            </Button>
          )}
          {summary.keyStoreError && (
            <p className="flex items-start gap-1.5 text-xs leading-5 text-red-300">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              خطای مخزن کلید: {summary.keyStoreError}
            </p>
          )}
          {summary.invalidSelections.length > 0 && (
            <div className="space-y-1 text-xs text-amber-300/90">
              {summary.invalidSelections.map((issue, i) => (
                <p key={i}>
                  «{fileBaseName(issue.path)}» انتخاب معتبری نبود: {issue.detail}
                </p>
              ))}
            </div>
          )}
          {summary.warnings.length > 0 && (
            <details className="group text-xs">
              <summary className="cursor-pointer text-ink-3 hover:text-ink-2">
                هشدارها ({faNum(summary.warnings.length)})
              </summary>
              <div className="mt-1.5 space-y-1 leading-5 text-ink-3">
                {summary.warnings.map((w, i) => (
                  <p key={i}>{w}</p>
                ))}
              </div>
            </details>
          )}
          {summary.stagedPaths.length > 0 && (
            <p className="text-xs leading-5 text-ink-3">
              {faNum(summary.stagedPaths.length)} پوشهٔ موقت دانلود حفظ شده است؛
              محتوای آن دست‌نخورده مانده و تا بازبینی شما پاک نمی‌شود.
            </p>
          )}
          <div className="flex justify-end">
            <Button onClick={dismissSummary}>بستن خلاصه</Button>
          </div>
        </Card>
      )}
    </div>
  );
}

/** نوار پیشرفت مرحلهٔ جاری — فقط از شمارش واقعی بک‌اند؛ هیچ درصدی ساختگی نیست. */
function StageProgressBar({
  completed,
  total,
}: {
  completed: number;
  total: number;
}) {
  const clamped = Math.min(completed, total);
  const percent = Math.round((clamped / total) * 100);
  return (
    <div className="space-y-1">
      <div className="h-2 w-full overflow-hidden rounded-full bg-zinc-800">
        <div
          className="h-full rounded-full bg-emerald-500/60 transition-all"
          style={{ width: `${percent}%` }}
        />
      </div>
      <div className="text-xs text-ink-3">
        در این مرحله: {faNum(clamped)} از {faNum(total)} فایل
      </div>
    </div>
  );
}

function StatusIcon({
  status,
  cancelling,
}: {
  status?: PipelineStatus;
  cancelling: boolean;
}) {
  if (cancelling) {
    return <Loader2 className="size-5 shrink-0 animate-spin text-red-400" />;
  }
  switch (status) {
    case "completed":
      return <CheckCircle2 className="size-5 shrink-0 text-emerald-400" />;
    case "cancelled":
      return <Info className="size-5 shrink-0 text-ink-2" />;
    case "failed":
      return <XCircle className="size-5 shrink-0 text-red-400" />;
    default:
      return <span className="size-2.5 shrink-0 animate-pulse rounded-full bg-accent" />;
  }
}

type RowTone = "success" | "warn" | "danger" | "info" | "neutral";

const ROW_CHIP: Record<RowTone, string> = {
  success: "bg-emerald-500/10 text-emerald-400",
  warn: "bg-amber-500/10 text-amber-400",
  danger: "bg-red-500/10 text-red-400",
  info: "bg-sky-500/10 text-sky-400",
  neutral: "bg-zinc-800/60 text-ink-3",
};

const ROW_DOT: Record<RowTone, string> = {
  success: "bg-emerald-400",
  warn: "bg-amber-400",
  danger: "bg-red-400",
  info: "animate-pulse bg-sky-400",
  neutral: "bg-zinc-600",
};

/** وضعیت نمایشی هر فایل — outcome نهایی مقدم بر وضعیت مرحله است. */
function itemRowState(item: PipelineItemSnapshot): {
  label: string;
  tone: RowTone;
} {
  if (item.outcome) {
    const badge = outcomeBadge(item.outcome);
    const tone: RowTone =
      badge.tone === "success"
        ? "success"
        : badge.tone === "danger"
          ? "danger"
          : badge.tone === "warn"
            ? "warn"
            : badge.tone === "info"
              ? "info"
              : "neutral";
    return { label: badge.label, tone };
  }
  if (item.stageStatus.kind === "needsDecision") {
    return { label: "منتظر تصمیم شما", tone: "warn" };
  }
  switch (item.stageStatus.kind) {
    case "inProgress":
      return { label: "در حال پردازش", tone: "info" };
    case "failed":
      return { label: "خطا در این مرحله", tone: "danger" };
    case "skipped":
      return { label: "از این مرحله رد شد", tone: "neutral" };
    case "done":
      // نتیجهٔ واقعی همین مرحله — نه «بدون نتیجهٔ نهایی» بی‌معنا
      if (item.stage === "extracting") return { label: "استخراج شد", tone: "info" };
      if (item.stage === "analyzing") return { label: "تحلیل شد", tone: "info" };
      if (item.stage === "syncing") return { label: "ورودی سینک پردازش شد", tone: "info" };
      return { label: "بدون نتیجهٔ نهایی", tone: "neutral" };
    default:
      return { label: "در انتظار", tone: "neutral" };
  }
}

function ItemRow({ item }: { item: PipelineItemSnapshot }) {
  const state = itemRowState(item);
  const detail = item.outcome ? outcomeDetail(item.outcome) : null;
  const reason =
    item.stageStatus.kind === "failed" || item.stageStatus.kind === "skipped"
      ? item.stageStatus.reason
      : null;
  const hasDetails =
    Boolean(detail) ||
    Boolean(reason) ||
    item.offsetSeconds != null ||
    item.placedPath != null ||
    (item.sidecarConflicts?.length ?? 0) > 0;

  return (
    <details className="group border-b border-line/60 last:border-0">
      <summary className="flex cursor-pointer list-none items-center gap-2.5 px-5 py-3 transition-colors hover:bg-surface-2/40">
        <span className={`size-2.5 shrink-0 rounded-full ${ROW_DOT[state.tone]}`} />
        <span
          dir="ltr"
          className="min-w-0 flex-1 truncate text-start font-mono text-sm text-ink-1"
          title={item.videoPath}
        >
          {item.videoFileName}
        </span>
        <span
          className={`shrink-0 rounded-md px-2.5 py-1 text-xs ${ROW_CHIP[state.tone]}`}
        >
          {state.label}
        </span>
        {hasDetails && (
          <ChevronDown className="size-3.5 shrink-0 text-ink-3 transition-transform group-open:rotate-180" />
        )}
      </summary>
      <div className="space-y-1 px-5 pb-3 ps-9 text-xs leading-6 text-ink-3">
        <div>مرحله: {PIPELINE_STAGE_LABELS[item.stage]}</div>
        {detail && <div>{detail}</div>}
        {reason && <div>{reason}</div>}
        {item.offsetSeconds != null && (
          <div dir="ltr" className="font-mono text-start">
            آفست: {formatOffset(item.offsetSeconds)}
          </div>
        )}
        {item.sidecarConflicts?.map((conflict, index) => (
          <div key={index}>
            تعارض حفظ‌شده: {fileBaseName(conflict.path)} — {conflict.reason}
          </div>
        ))}
      </div>
    </details>
  );
}

/** مواردی که کاربر باید ببیند: شکست، تعارض، کیفیت پایین، نیاز به کلید. */
function NeedsReview({ summary }: { summary: PipelineSummary }) {
  const flagged = summary.items.filter((item) => {
    switch (item.outcome?.kind) {
      case "failed":
      case "outputConflict":
      case "unchangedLowQuality":
      case "needsApiKey":
      case "keyStoreUnavailable":
        return true;
      default:
        return false;
    }
  });
  if (flagged.length === 0) return null;
  return (
    <div className="space-y-1.5 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
      <div className="text-xs font-semibold text-amber-300">
        نیازمند بررسی ({faNum(flagged.length)})
      </div>
      {flagged.slice(0, 20).map((item) => {
        const badge = item.outcome ? outcomeBadge(item.outcome) : null;
        const detail = item.outcome ? outcomeDetail(item.outcome) : null;
        return (
          <div key={item.videoPath} className="text-[11px] leading-5">
            <span dir="ltr" className="font-mono text-ink-1">
              {item.videoFileName}
            </span>
            {badge && <span className="text-ink-2"> — {badge.label}</span>}
            {detail && <div className="text-ink-3">{detail}</div>}
          </div>
        );
      })}
      {flagged.length > 20 && (
        <div className="text-[11px] text-ink-3">
          {faNum(flagged.length - 20)} مورد دیگر — جزئیات در فهرست فایل‌ها.
        </div>
      )}
    </div>
  );
}

const COUNT_LABELS: Array<[keyof PipelineCounts, string]> = [
  ["synced", "سینک شد"],
  ["alreadySynced", "از قبل سینک‌شده"],
  ["unchangedLowQuality", "کیفیت پایین"],
  ["conflicts", "تعارض"],
  ["needsApiKey", "نیازمند کلید API"],
  ["keyStoreUnavailable", "خطای مخزن کلید"],
  ["skipped", "پرش"],
  ["failed", "ناموفق"],
  ["extracted", "استخراج‌شده"],
  ["downloaded", "دانلودشده"],
  ["archivesPlaced", "از آرشیو"],
  ["analyzed", "تحلیل‌شده"],
];

function CountsGrid({ counts }: { counts: PipelineCounts }) {
  const visible = COUNT_LABELS.filter(([key]) => counts[key] > 0);
  if (visible.length === 0) return null;
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
      {visible.map(([key, label]) => (
        <div
          key={key}
          className="flex items-center justify-between rounded-lg border border-line bg-surface-2 px-3 py-2 text-xs"
        >
          <span className="text-ink-3">{label}</span>
          <span className="font-bold text-ink-1">{faNum(counts[key])}</span>
        </div>
      ))}
    </div>
  );
}
