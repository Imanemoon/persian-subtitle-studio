import { useState } from "react";
import { AlertTriangle, CheckCircle2, Info, Square, XCircle } from "lucide-react";
import { Badge, Button, Card } from "../ui";
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
  stageStatusBadge,
} from "../../lib/pipeline";
import { faNum, formatDuration } from "../../lib/format";
import { fileBaseName } from "../../lib/videoFiles";
import type { PipelineCounts, PipelineItemSnapshot } from "../../types/pipeline";
import type { PageId } from "../../types";

/** نمای run pipeline — همهٔ داده‌ها از بک‌اند می‌آید؛ هیچ درصد یا موفقیتی ساختگی نیست. */
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

  const status = summary?.status ?? snapshot?.status;
  const counts = summary?.counts ?? snapshot?.counts;
  const message = snapshot?.message;
  const currentFile = snapshot?.currentFile ?? null;
  const stage = snapshot?.stage;
  const total = summary?.items.length ?? snapshot?.total ?? 0;
  const completed = runActive ? (snapshot?.completed ?? 0) : (summary?.items.length ?? snapshot?.completed ?? 0);

  return (
    <div className="w-full max-w-2xl space-y-4">
      {/* سربرگ run — وضعیت + پیام واقعی بک‌اند */}
      <Card className="space-y-4 p-5">
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            {status === "completed" && (
              <CheckCircle2 className="size-5 text-emerald-400" />
            )}
            {status === "cancelled" && <Info className="size-5 text-ink-2" />}
            {status === "failed" && (
              <XCircle className="size-5 text-red-400" />
            )}
            {(status === "running" || status === "awaitingDecision") && (
              <span className="size-2.5 animate-pulse rounded-full bg-accent" />
            )}
            <span className="text-sm font-bold text-ink-1">
              {status ? PIPELINE_STATUS_LABELS[status] : ""}
            </span>
          </div>
          {runActive && (
            <Button
              variant="danger"
              onClick={() => void cancelPipeline()}
              disabled={busy}
              loading={busy}
              title="لغو pipeline — فایل‌های اصلی دست‌نخورده می‌مانند"
            >
              <Square className="size-3.5" />
              توقف
            </Button>
          )}
        </div>

        {message && <p className="text-sm text-ink-2">{message}</p>}
        {currentFile && (
          <div
            dir="ltr"
            className="truncate font-mono text-xs text-ink-3"
            title={currentFile}
          >
            {fileBaseName(currentFile)}
          </div>
        )}

        {/* ریل مراحل — نشان می‌دهد الان کجای فرایند هستیم؛ ادعای تکمیل مراحل
            قبلی نمی‌کند چون بک‌اند تکمیل سراسری مرحله را گزارش نمی‌کند */}
        {stage && (
          <div className="flex flex-wrap gap-1.5">
            {PIPELINE_STAGES.map((s) => (
              <span
                key={s}
                className={`rounded-md px-2 py-0.5 text-[11px] ${
                  s === stage
                    ? "bg-accent-dim font-semibold text-accent-strong"
                    : "bg-zinc-800/60 text-ink-3"
                }`}
              >
                {PIPELINE_STAGE_LABELS[s]}
              </span>
            ))}
          </div>
        )}

        {/* پیشرفت واقعی — فقط شمارش آیتم‌های نهایی‌شده؛ جدا از ریل مراحل */}
        {total > 0 && (
          <div className="space-y-1.5">
            <div className="h-2 w-full overflow-hidden rounded-full bg-zinc-800">
              <div
                className="h-full rounded-full bg-accent transition-all"
                style={{ width: `${Math.min(100, Math.round((completed / total) * 100))}%` }}
              />
            </div>
            <div className="flex items-center justify-between text-xs text-ink-3">
              <span>پیشرفت بر اساس موارد نهایی‌شده</span>
              <span>
                {faNum(completed)} از {faNum(total)} ویدیو
              </span>
            </div>
          </div>
        )}

        {actionError && (
          <p className="flex items-start gap-1.5 text-xs leading-5 text-red-300">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            {actionError}
          </p>
        )}
      </Card>

      {/* دیالوگ تصمیم — گزینه‌های backend-owned با شناسهٔ مات */}
      {decision && (
        <Card className="border-amber-500/40 space-y-3 p-5">
          <div>
            <div className="text-sm font-bold text-amber-300">
              {DECISION_KIND_TEXT[decision.kind].title} — {decision.videoFileName}
            </div>
            <p className="mt-1 text-xs leading-5 text-ink-2">
              {DECISION_KIND_TEXT[decision.kind].description}
            </p>
          </div>
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
          <div className="flex justify-end gap-2">
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

      {/* وضعیت هر ویدیو — مرحله/گزینهٔ تصمیم/نتیجهٔ نهایی تفکیک‌شده */}
      {items.length > 0 && (
        <div className="overflow-hidden rounded-xl border border-line bg-surface-1">
          <table className="w-full text-sm">
            <thead className="bg-surface-1">
              <tr className="border-b border-line text-xs text-ink-3">
                <th className="px-4 py-2.5 text-start font-medium">فایل</th>
                <th className="px-4 py-2.5 text-start font-medium">مرحله</th>
                <th className="px-4 py-2.5 text-start font-medium">نتیجه</th>
              </tr>
            </thead>
            <tbody>
              {items.slice(0, 300).map((item) => (
                <ItemRow key={item.videoPath} item={item} />
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* خلاصهٔ نهایی — فقط وقتی بک‌اند آن را فرستاده؛ هیچ چیز حدسی نیست */}
      {summary && !runActive && counts && (
        <Card className="space-y-4 p-5">
          <div className="text-sm font-semibold text-ink-1">
            خلاصهٔ اجرا ({formatDuration(summary.durationMs)})
          </div>
          <CountsGrid counts={counts} />
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
            <div className="space-y-1 text-xs leading-5 text-ink-3">
              {summary.warnings.map((w, i) => (
                <p key={i}>{w}</p>
              ))}
            </div>
          )}
          {summary.stagedPaths.length > 0 && (
            <p className="text-xs text-ink-3">
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

function ItemRow({ item }: { item: PipelineItemSnapshot }) {
  const stageBadge = stageStatusBadge(item.stageStatus);
  const detail = item.outcome ? outcomeDetail(item.outcome) : null;
  const isAwaiting = item.stageStatus.kind === "needsDecision";
  return (
    <tr className="border-b border-line/60 last:border-0">
      <td dir="ltr" className="max-w-72 px-4 py-2.5 text-start">
        <span className="block truncate font-mono text-xs" title={item.videoFileName}>
          {item.videoFileName}
        </span>
        {detail && (
          <span className="block truncate text-[11px] text-ink-3" title={detail}>
            {detail}
          </span>
        )}
        {item.offsetSeconds != null && (
          <span dir="ltr" className="block font-mono text-[11px] text-ink-3">
            offset {formatOffset(item.offsetSeconds)}
          </span>
        )}
      </td>
      <td className="px-4 py-2.5">
        <Badge tone={stageBadge.tone}>{stageBadge.label}</Badge>
      </td>
      <td className="px-4 py-2.5">
        {item.outcome ? (
          (() => {
            const b = outcomeBadge(item.outcome);
            return <Badge tone={b.tone}>{b.label}</Badge>;
          })()
        ) : isAwaiting ? (
          <Badge tone="warn">منتظر تصمیم شما</Badge>
        ) : (
          <span className="text-xs text-ink-3">—</span>
        )}
      </td>
    </tr>
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
