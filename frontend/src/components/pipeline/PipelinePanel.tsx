import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  Captions,
  CheckCircle2,
  Home,
  Play,
  FolderOpen,
  RotateCcw,
  ChevronDown,
  Film,
  Info,
  Loader2,
  Square,
  XCircle,
} from "lucide-react";
import { Badge, Button, Card, Modal } from "../ui";
import { usePipeline } from "../../hooks/usePipeline";
import { useHardsub } from "../../hooks/useHardsub";
import { ask } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { useTaskFocus } from "../../hooks/useTaskFocus";
import { api } from "../../lib/api";
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
import type { HardsubPlan } from "../../types/hardsub";
import type { OpsProgress, PageId } from "../../types";

/** آیتم‌هایی که منبع زیرنویس آماده دارند و می‌توانند هاردساب شوند. */
/** آیتم‌هایی که زیرنویس‌شان داخل تصویر حک شده — کاندید افزودن زیرنویس چسبیده */
function isHardsubbed(item: PipelineItemSnapshot): boolean {
  return item.outcome?.kind === "hardsubbed";
}

/** آیتم‌هایی که زیرنویس دارند و می‌توان برایشان «تنظیم دوباره» زد */
function redoEligible(item: PipelineItemSnapshot): boolean {
  return (
    item.outcome?.kind === "synced" ||
    item.outcome?.kind === "softSubbed" ||
    item.outcome?.kind === "alreadySynced" ||
    item.outcome?.kind === "hardsubbed"
  );
}

function hardsubEligible(item: PipelineItemSnapshot): boolean {
  return (
    item.outcome?.kind === "synced" ||
    item.outcome?.kind === "softSubbed" ||
    item.outcome?.kind === "alreadySynced"
  );
}

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
  const { startHardsub } = useHardsub();
  const [selectedOption, setSelectedOption] = useState<string | null>(null);
  // ---- هاردساب: انتخاب گروهی + پلن تأیید ----
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [plan, setPlan] = useState<HardsubPlan | null>(null);
  const [planLoading, setPlanLoading] = useState(false);
  const [planError, setPlanError] = useState<string | null>(null);
  // ---- افزودن زیرنویس چسبیده به هاردساب‌ها — مودال global از context ----
  const { setFocus, openSoftsub } = useTaskFocus();
  // ---- لیست بلند سریالی: ابتدا ۵ مورد، «نمایش همه» بقیه را باز می‌کند ----
  const [filesExpanded, setFilesExpanded] = useState(false);
  // ---- تنظیم دوباره زیرنویس ----
  const [redoBusy, setRedoBusy] = useState<Set<string>>(new Set());
  const [redoError, setRedoError] = useState<string | null>(null);
  // ---- ETA اجرا: شروع run + تیک یک‌ثانیه‌ای برای به‌روزرسانی تقریب ----
  const runStartedAt = useRef<number | null>(null);
  const [, setEtaTick] = useState(0);
  // خطای reset که داخل thread بک‌اند رخ داده (تا موفقیت دانلود هیچ چیز
  // خراب نمی‌شود) — از رویداد ops:done همان‌جا به کاربر نشان داده می‌شود
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    (async () => {
      const un = await listen<OpsProgress>("ops:done", (event) => {
        const p = event.payload;
        if (p.kind !== "subtitle-reset") return;
        if (p.message.includes("ناموفق")) setRedoError(p.message);
      });
      if (disposed) un();
      else unlisten = un;
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    runStartedAt.current = runActive ? (runStartedAt.current ?? Date.now()) : null;
    if (!runActive) return;
    const timer = setInterval(() => setEtaTick((x) => x + 1), 1000);
    return () => clearInterval(timer);
  }, [runActive]);

  if (!snapshot && !summary) return null;

  const eligiblePaths = items.filter(hardsubEligible).map((item) => item.videoPath);
  const eligibleCount = eligiblePaths.length;
  const checkedCount = eligiblePaths.filter((p) => checked.has(p)).length;
  const hardsubbedPaths = items.filter(isHardsubbed).map((item) => item.videoPath);

  const toggleChecked = (path: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const toggleAllChecked = () => {
    setChecked(() =>
      checkedCount === eligibleCount ? new Set() : new Set(eligiblePaths),
    );
  };

  /** پلن هاردساب از بک‌اند (بازبینی تازهٔ فایل‌سیستم) → دیالوگ تأیید */
  const requestHardsub = async (paths: string[]) => {
    setPlanError(null);
    setPlanLoading(true);
    try {
      const fresh = await api.planHardsub(paths);
      setPlan(fresh);
    } catch (e) {
      setPlanError(String(e));
    } finally {
      setPlanLoading(false);
    }
  };

  /** تنظیم دوباره: تأیید → حذف زیرنویس فعلی + دانلود تازه + سینک خودکار از اول */
  const redoSubtitle = async (videoPath: string) => {
    const confirmed = await ask(
      "زیرنویس فعلی این ویدیو حذف می‌شود؛ زیرنویس تازه از SubSource دانلود و از اول سینک می‌شود. ادامه؟",
      { title: "تنظیم دوباره زیرنویس", kind: "warning" },
    );
    if (!confirmed) return;
    setRedoError(null);
    setRedoBusy((prev) => new Set(prev).add(videoPath));
    try {
      await api.resetSubtitle(videoPath);
      // پایان و سینک خودکار از رویدادهای ops (subtitle-reset) و pipeline می‌آید
    } catch (e) {
      setRedoError(String(e));
    } finally {
      setRedoBusy((prev) => {
        const next = new Set(prev);
        next.delete(videoPath);
        return next;
      });
    }
  };

  const confirmHardsub = async () => {
    if (!plan || plan.items.length === 0) return;
    setPlanLoading(true);
    const ok = await startHardsub(plan.items.map((item) => item.videoPath));
    setPlanLoading(false);
    if (ok) {
      setPlan(null);
      setChecked(new Set());
    }
  };

  const status: PipelineStatus | undefined = summary?.status ?? snapshot?.status;
  const counts = summary?.counts ?? snapshot?.counts;
  const message = snapshot?.message;
  const currentFile = snapshot?.currentFile ?? null;
  const stage = snapshot?.stage;
  const total = summary?.items.length ?? snapshot?.total ?? 0;
  const completed = summary
    ? summary.items.filter((item) => item.outcome !== null).length
    : (snapshot?.completed ?? 0);
  const active = runActive ? (snapshot?.active ?? 0) : 0;
  const cancelling = Boolean(snapshot?.cancelling);
  const stageIndex = stage ? PIPELINE_STAGES.indexOf(stage) : -1;
  const waiting = status === "awaitingDecision" || decision !== null;

  // ---- یک عدد برای کل پیشرفت: (اندیس مرحله + کسر درون‌مرحله‌ای) / کل مراحل ----
  const stageInnerFrac =
    runActive && snapshot?.stageProgress && snapshot.stageProgress.total > 0
      ? Math.min(1, snapshot.stageProgress.completed / snapshot.stageProgress.total)
      : 0;
  const finishedAll = total > 0 && completed >= total;
  const overall = finishedAll
    ? 1
    : stage && stageIndex >= 0
      ? Math.min(0.99, (stageIndex + stageInnerFrac) / PIPELINE_STAGES.length)
      : total > 0
        ? Math.min(0.99, completed / total)
        : 0;
  const overallPct = Math.round(overall * 100);
  const etaSeconds =
    runActive && runStartedAt.current !== null && overall >= 0.05 && !waiting
      ? Math.max(
          0,
          Math.round(((Date.now() - runStartedAt.current) * (1 - overall)) / overall / 1000),
        )
      : null;
  const playablePath = items.find((item) => redoEligible(item))?.videoPath ?? null;
  const summaryHeadline = summary
    ? summary.status === "cancelled"
      ? "اجرا متوقف شد"
      : summary.status === "failed"
        ? "اجرا ناموفق بود"
        : counts && counts.synced + counts.softsubbed + counts.alreadySynced > 0
          ? "سینک انجام شد"
          : counts && counts.unchangedLowQuality > 0
            ? "سینک انجام نشد — کیفیت پایین"
            : "پایان اجرا"
    : "";

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
          <div className="flex shrink-0 items-center gap-2">
            <Button
              onClick={() => {
                // ترک نمای تسک — کار در پس‌زمینه ادامه می‌یابد و از جعبهٔ
                // تسک‌ها (کنار برنامه) دوباره باز می‌شود
                setFocus(null);
                onNavigate("home");
              }}
              title="بازگشت به خانه — تسک در پس‌زمینه ادامه می‌یابد"
            >
              <Home className="size-3.5" />
              خانه
            </Button>
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

        {/* یک نوار، یک درصد بولد، یک زمان تقریبی */}
        {(stage || total > 0) && (
          <div className="space-y-2">
            <div className="flex items-baseline justify-between">
              <span className="font-semibold text-ink-1">
                {stage ? PIPELINE_STAGE_LABELS[stage] : ""}
              </span>
              {etaSeconds !== null && (
                <span className="text-xs text-ink-3">
                  حدوداً {formatEta(etaSeconds)} دیگر
                </span>
              )}
            </div>
            <div className="flex items-center gap-3">
              <div className="h-3 flex-1 overflow-hidden rounded-full bg-zinc-800">
                <div
                  className={`h-full rounded-full transition-all ${
                    cancelling ? "bg-red-400" : "bg-accent"
                  }`}
                  style={{ width: `${overallPct}%` }}
                />
              </div>
              <span className="min-w-14 text-end text-lg font-bold text-ink-1">
                {faNum(overallPct)}٪
              </span>
            </div>
            {status === "cancelled" ? (
              <p className="text-xs text-ink-3">
                اجرا پیش از تکمیل متوقف شد — فایل‌های اصلی دست‌نخورده ماندند.
              </p>
            ) : status === "failed" ? (
              <p className="text-xs text-ink-3">اجرا ناموفق ماند — جزئیات در پایین صفحه.</p>
            ) : (
              currentFile &&
              !summary && (
                <p
                  dir="ltr"
                  className="truncate text-start font-mono text-[11px] text-ink-3"
                >
                  {fileBaseName(currentFile)}
                </p>
              )
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

      {/* ---- افزودن زیرنویس چسبیده به هاردساب‌ها — بالای لیست برای دسترسی راحت ---- */}
      {!runActive && hardsubbedPaths.length > 0 && (
        <Card className="flex flex-wrap items-center justify-between gap-3 border-accent/40 p-4">
          <div className="min-w-0">
            <div className="text-sm font-semibold text-ink-1">
              {faNum(hardsubbedPaths.length)} مورد زیرنویس داخل تصویر دارند
            </div>
            <p className="mt-0.5 text-xs leading-5 text-ink-3">
              با یک کلیک برای همه‌شان زیرنویس چسبیده دانلود کنید — هم‌نام هر ویدیو
              کنارش قرار می‌گیرد؛ سپس با اجرای مجدد سینک، هم‌تراز و SoftSub می‌شود.
            </p>
          </div>
          <Button
            variant="primary"
            className="shrink-0"
            disabled={planLoading}
            onClick={() => openSoftsub(hardsubbedPaths)}
          >
            <Captions className="size-4" />
            افزودن زیرنویس چسبیده به همه ({faNum(hardsubbedPaths.length)})
          </Button>
        </Card>
      )}

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

      {/* ---- فایل‌ها: فقط برای اجراهای چندفایلی — تک‌فایلی از خلاصه بالا پوشش داده می‌شود ---- */}
      {items.length > 1 && (
        <Card className="overflow-hidden">
          <div className="flex items-center justify-between border-b border-line px-5 py-3">
            <span className="text-sm font-semibold text-ink-1">فایل‌های این اجرا</span>
            <span className="text-xs text-ink-3">
              {faNum(items.length)} فایل
            </span>
          </div>
          {/* نوار هاردساب گروهی — فقط پس از پایان اجرا و برای موارد دارای زیرنویس */}
          {!runActive && eligibleCount > 0 && (
            <div className="flex flex-wrap items-center justify-between gap-2 border-b border-line bg-surface-2/40 px-5 py-2.5">
              <label className="flex cursor-pointer items-center gap-2 text-xs text-ink-2">
                <input
                  type="checkbox"
                  className="size-4 accent-emerald-500"
                  checked={eligibleCount > 0 && checkedCount === eligibleCount}
                  onChange={toggleAllChecked}
                />
                انتخاب همه ({faNum(eligibleCount)} مورد با زیرنویس آماده)
              </label>
              <Button
                variant="primary"
                className="h-8 px-3 text-xs"
                disabled={checkedCount === 0 || planLoading}
                loading={planLoading}
                onClick={() => void requestHardsub(eligiblePaths.filter((p) => checked.has(p)))}
              >
                <Film className="size-3.5" />
                هاردساب موارد انتخاب‌شده ({faNum(checkedCount)})
              </Button>
            </div>
          )}
          <div>
            {(filesExpanded ? items.slice(0, 300) : items.slice(0, 5)).map((item) => (
              <ItemRow
                key={item.videoPath}
                item={item}
                showActions={!runActive && eligibleCount > 0}
                checked={checked.has(item.videoPath)}
                onToggle={() => toggleChecked(item.videoPath)}
                onHardsub={() => void requestHardsub([item.videoPath])}
                showSoftsub={!runActive && isHardsubbed(item)}
                onSoftsub={() => openSoftsub([item.videoPath])}
                showRedo={!runActive && redoEligible(item)}
                redoBusy={redoBusy.has(item.videoPath)}
                onRedo={() => void redoSubtitle(item.videoPath)}
              />
            ))}
          </div>
          {items.length > 5 && (
            <button
              onClick={() => setFilesExpanded((prev) => !prev)}
              className="w-full border-t border-line px-5 py-2.5 text-center text-xs text-ink-3 transition-colors hover:bg-surface-2/40 hover:text-ink-1"
            >
              {filesExpanded
                ? "نمایش کمتر"
                : `نمایش همهٔ ${faNum(items.length)} مورد`}
            </button>
          )}
          {planError && (
            <p className="flex items-start gap-1.5 border-t border-line px-5 py-2.5 text-xs leading-5 text-red-300">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              {planError}
            </p>
          )}
          {redoError && (
            <p className="flex items-start gap-1.5 border-t border-line px-5 py-2.5 text-xs leading-5 text-red-300">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              {redoError}
            </p>
          )}
        </Card>
      )}

      {/* ---- افزودن زیرنویس چسبیده به هاردساب‌ها ---- */}
      {/* ---- دیالوگ تأیید هاردساب — پلن تازه از بک‌اند ---- */}
      <Modal
        open={plan !== null}
        title="تأیید هاردساب"
        onClose={() => {
          setPlan(null);
          setPlanError(null);
        }}
        footer={
          plan && plan.items.length > 0 ? (
            <>
              <Button
                onClick={() => {
                  setPlan(null);
                  setPlanError(null);
                }}
              >
                انصراف
              </Button>
              <Button
                variant="primary"
                loading={planLoading}
                onClick={() => void confirmHardsub()}
              >
                شروع هاردساب ({faNum(plan.items.length)} مورد)
              </Button>
            </>
          ) : (
            <Button
              onClick={() => {
                setPlan(null);
                setPlanError(null);
              }}
            >
              بستن
            </Button>
          )
        }
      >
        {plan && (
          <div className="space-y-3 text-sm">
            <div className="flex items-center justify-between rounded-lg border border-line bg-surface-2 px-3 py-2 text-xs">
              <span className="text-ink-3">انکودر تشخیص‌داده‌شده</span>
              <span dir="ltr" className="font-semibold text-ink-1">
                {plan.encoder}
              </span>
            </div>
            <p className="text-xs leading-6 text-ink-2">
              زیرنویس داخل تصویر حک می‌شود؛ خروجی فایل جدید کنار هر ویدیو ساخته
              می‌شود و فایل اصلی دست‌نخورده می‌ماند. انکود ویدیو زمان‌بر است و به
              ازای هر فایل جداگانه اجرا می‌شود.
            </p>
            {plan.items.length > 0 && (
              <div className="space-y-1">
                {plan.items.slice(0, 100).map((item) => (
                  <div
                    key={item.videoPath}
                    className="rounded-lg border border-line bg-surface-2/60 px-3 py-2"
                  >
                    <div dir="ltr" className="truncate font-mono text-xs text-ink-1">
                      {item.videoFileName}
                    </div>
                    <div dir="ltr" className="truncate font-mono text-[11px] text-ink-3">
                      → {fileBaseName(item.outputPath)}
                    </div>
                  </div>
                ))}
                {plan.items.length > 100 && (
                  <p className="text-[11px] text-ink-3">
                    {faNum(plan.items.length - 100)} مورد دیگر…
                  </p>
                )}
              </div>
            )}
            {plan.skipped.length > 0 && (
              <div className="space-y-1 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
                <div className="text-xs font-semibold text-amber-300">
                  رد شده ({faNum(plan.skipped.length)})
                </div>
                {plan.skipped.slice(0, 20).map((skip) => (
                  <div key={skip.path} className="text-[11px] leading-5">
                    <span dir="ltr" className="font-mono text-ink-1">
                      {fileBaseName(skip.path)}
                    </span>
                    <span className="text-ink-3"> — {skip.reason}</span>
                  </div>
                ))}
                {plan.skipped.length > 20 && (
                  <p className="text-[11px] text-ink-3">
                    {faNum(plan.skipped.length - 20)} مورد دیگر…
                  </p>
                )}
              </div>
            )}
            {plan.items.length === 0 && (
              <p className="text-xs text-amber-300">
                هیچ موردی قابل هاردساب نیست — فهرست رد شده را ببینید.
              </p>
            )}
          </div>
        )}
      </Modal>

      {/* ---- خلاصهٔ پایان: تیتر واضح + اقدام بعدی (پخش/تنظیم دوباره) ---- */}
      {summary && !runActive && counts && (
        <Card className="space-y-4 p-6">
          <div className="flex items-start justify-between gap-3">
            <div className="flex items-center gap-2">
              <StatusIcon status={summary.status} cancelling={false} />
              <div>
                <div className="text-base font-bold text-ink-1">
                  {summaryHeadline}
                </div>
                <div className="text-xs text-ink-3">
                  {formatDuration(summary.durationMs)}
                </div>
              </div>
            </div>
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

          {items.length > 1 && <CountsGrid counts={counts} />}

          {/* قضاوت سریع کاربر: پخش کن؛ سینک درست نبود تنظیم دوباره این بار
              با روش متفاوتی سینک می‌کند */}
          {playablePath && (
            <div className="flex flex-wrap items-center gap-2 rounded-lg border border-line bg-surface-2/60 p-3">
              <Button
                variant="primary"
                onClick={() => void api.playVideo(playablePath)}
                title="پخش با پخش‌کنندهٔ پیش‌فرض ویندوز"
              >
                <Play className="size-4" />
                پخش ویدیو
              </Button>
              <Button
                onClick={() => void api.revealInStorage(playablePath)}
                title="باز کردن پوشهٔ محل ذخیرهٔ فایل در Explorer"
              >
                <FolderOpen className="size-3.5" />
                محل ذخیره
              </Button>
              <Button
                disabled={redoBusy.has(playablePath)}
                loading={redoBusy.has(playablePath)}
                onClick={() => void redoSubtitle(playablePath)}
                title="دانلود تازه + سینک با روش متفاوت"
              >
                <RotateCcw className="size-3.5" />
                تنظیم دوباره
              </Button>
              <span className="text-[11px] leading-5 text-ink-3">
                سینک درست نیست؟ تنظیم دوباره این بار با روش دیگری سینک می‌کند.
              </span>
            </div>
          )}

          <NeedsReview summary={summary} />

          {/* هاردساب گروهی همهٔ موارد موفق — پس از پایان سینک */}
          {!runActive && eligibleCount > 0 && (
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-xs leading-5 text-ink-3">
                زیرنویس را داخل تصویر حک کن — خروجی فایل جدید کنار ویدیو، بدون
                تغییر فایل اصلی.
              </p>
              <Button
                variant="primary"
                disabled={planLoading}
                loading={planLoading}
                onClick={() => void requestHardsub(eligiblePaths)}
              >
                <Film className="size-3.5" />
                هاردساب همهٔ موارد موفق ({faNum(eligibleCount)})
              </Button>
            </div>
          )}

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

function ItemRow({
  item,
  showActions,
  checked,
  onToggle,
  onHardsub,
  showSoftsub,
  onSoftsub,
  showRedo,
  redoBusy,
  onRedo,
}: {
  item: PipelineItemSnapshot;
  showActions: boolean;
  checked: boolean;
  onToggle: () => void;
  onHardsub: () => void;
  showSoftsub: boolean;
  onSoftsub: () => void;
  showRedo: boolean;
  redoBusy: boolean;
  onRedo: () => void;
}) {
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
  const hardsubbable = hardsubEligible(item);
  // نتیجهٔ موفق سینک/SoftSub — دکمهٔ «پخش» برای قضاوت سریع کاربر
  const playable =
    item.outcome?.kind === "synced" ||
    item.outcome?.kind === "softSubbed" ||
    item.outcome?.kind === "alreadySynced";

  return (
    <details className="group border-b border-line/60 last:border-0">
      <summary className="flex cursor-pointer list-none items-center gap-2.5 px-5 py-3 transition-colors hover:bg-surface-2/40">
        {showActions && hardsubbable && (
          // داخل summary است — toggle جزئیات با stop/prevent خنثی می‌شود
          <input
            type="checkbox"
            className="size-4 shrink-0 accent-emerald-500"
            checked={checked}
            onClick={(e) => {
              e.stopPropagation();
              e.preventDefault();
            }}
            onChange={onToggle}
            aria-label={`انتخاب ${item.videoFileName} برای هاردساب`}
          />
        )}
        <span className={`size-2.5 shrink-0 rounded-full ${ROW_DOT[state.tone]}`} />
        <span
          dir="ltr"
          className="min-w-0 flex-1 truncate text-start font-mono text-sm text-ink-1"
          title={item.videoPath}
        >
          {item.videoFileName}
        </span>
        {playable && (
          <Button
            variant="ghost"
            className="h-7 shrink-0 px-2.5 text-xs"
            onClick={(e) => {
              e.stopPropagation();
              e.preventDefault();
              void api.playVideo(item.videoPath);
            }}
            title="پخش با پخش‌کنندهٔ پیش‌فرض ویندوز — قضاوت سریع سینک"
          >
            <Play className="size-3.5" />
            پخش
          </Button>
        )}
        {showActions && hardsubbable && (
          <Button
            variant="ghost"
            className="h-7 shrink-0 px-2.5 text-xs"
            onClick={(e) => {
              e.stopPropagation();
              e.preventDefault();
              onHardsub();
            }}
            title="حک کردن زیرنویس داخل تصویر — خروجی فایل جدید کنار ویدیو"
          >
            <Film className="size-3.5" />
            هاردساب
          </Button>
        )}
        {showSoftsub && (
          <Button
            variant="ghost"
            className="h-7 shrink-0 px-2.5 text-xs"
            onClick={(e) => {
              e.stopPropagation();
              e.preventDefault();
              onSoftsub();
            }}
            title="دانلود یا انتخاب فایل زیرنویس چسبیده — هم‌نام کنار ویدیو"
          >
            <Captions className="size-3.5" />
            زیرنویس چسبیده
          </Button>
        )}
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
        {showRedo && (
          <div className="pt-1">
            <Button
              variant="ghost"
              className="h-7 px-2.5 text-xs"
              disabled={redoBusy}
              loading={redoBusy}
              onClick={(e) => {
                e.stopPropagation();
                onRedo();
              }}
              title="زیرنویس فعلی حذف و زیرنویس تازه دانلود و از اول سینک می‌شود"
            >
              <RotateCcw className="size-3.5" />
              تنظیم دوباره
            </Button>
          </div>
        )}
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
  ["softsubbed", "SoftSub شده"],
  ["hardsubbed", "هاردساب (داخل تصویر)"],
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

/** نمایش تقریبی زمان باقی‌مانده — از نسبت elapsed/progress واقعی، بدون دقت کاذب. */
function formatEta(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${faNum(s)} ثانیه`;
  const m = Math.round(s / 60);
  if (m < 60) return `${faNum(m)} دقیقه`;
  const h = Math.floor(m / 60);
  return `${faNum(h)} ساعت و ${faNum(m % 60)} دقیقه`;
}
