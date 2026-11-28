import { useEffect, useState } from "react";
import {
  AlertTriangle,
  Check,
  FileVideo,
  Folder,
  Hand,
  Loader2,
  Square,
  X,
  Zap,
} from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { usePipeline } from "../hooks/usePipeline";
import { useHardsub } from "../hooks/useHardsub";
import { HardsubPanel } from "../components/hardsub/HardsubPanel";
import { useTaskFocus } from "../hooks/useTaskFocus";
import { PipelinePanel } from "../components/pipeline/PipelinePanel";
import {
  fileBaseName,
  folderBaseName,
  isPathInside,
  isSamePath,
  isSupportedVideoFile,
} from "../lib/videoFiles";
import { api } from "../lib/api";
import { faNum } from "../lib/format";
import type { AutomationMode, PageId, ScanSummary, VideoEntry } from "../types";
import metropolisBg from "../assets/bg/metropolis.svg";
import projectorBg from "../assets/bg/projector.svg";
import noirBg from "../assets/bg/noir.svg";
import deepSpaceBg from "../assets/bg/deepSpace.svg";

/** صحنه‌های سینمایی باندل‌شده — برداری و سبک؛ در هر اجرا یکی تصادفی انتخاب
 * و هر ۴۵ ثانیه با crossfade عوض می‌شود. همه به‌شدت تار و تیره رندر می‌شوند
 * تا متن کاملاً خوانا بماند. */
const BG_SCENES = [metropolisBg, projectorBg, noirBg, deepSpaceBg];

/** لایهٔ پس‌زمینهٔ سینمایی نمای انتخاب — زیر همهٔ محتوا با z منفی. */
function CinematicBackground() {
  const [index, setIndex] = useState(() =>
    Math.floor(Math.random() * BG_SCENES.length),
  );
  useEffect(() => {
    const timer = setInterval(
      () => setIndex((i) => (i + 1) % BG_SCENES.length),
      45_000,
    );
    return () => clearInterval(timer);
  }, []);
  return (
    <div className="fixed inset-0 -z-10 overflow-hidden" aria-hidden>
      {BG_SCENES.map((src, i) => (
        <img
          key={src}
          src={src}
          alt=""
          className={`animate-kenburns motion-reduce:animate-none absolute inset-0 h-full w-full object-cover transition-opacity duration-[1500ms] ${
            i === index ? "opacity-100" : "opacity-0"
          }`}
          style={{
            filter: "blur(10px) brightness(0.62) saturate(0.95)",
            willChange: "transform, opacity",
          }}
        />
      ))}
      {/* پردهٔ تیره برای خوانایی متن */}
      <div className="absolute inset-0 bg-gradient-to-b from-surface-0/25 via-surface-0/40 to-surface-0/85" />
    </div>
  );
}

/**
 * وضعیت شناسایی ویدیوهای پوشه — از اسکن واقعی همان پوشه می‌آید.
 * هیچ تعداد یا پیشرفتی ساختگی نیست؛ اسکن فعلی برنامه منبع حقیقت است.
 */
type FolderDetection =
  | { status: "idle" }
  | { status: "scanning" }
  /** اسکن فعالی برای پوشهٔ دیگری در جریان است */
  | { status: "elsewhere" }
  | { status: "done"; paths: string[] }
  | { status: "cancelled"; paths: string[] }
  | { status: "empty" };

/**
 * صفحهٔ خانه — نقطهٔ شروع pipeline واقعی بک‌اند.
 * وقتی run فعالی جریان دارد (یا خلاصهٔ تازه‌ای هست) نمای pipeline نشان داده
 * می‌شود؛ در غیر این صورت نمای انتخاب. هیچ پیشرفت جعلی‌ای وجود ندارد.
 */
export function Home({ onNavigate }: { onNavigate: (page: PageId) => void }) {
  const {
    selection,
    setFolderSelection,
    setFileSelection,
    clearSelection,
    scanning,
    progress,
    lastScan,
    library,
    startScan,
    stopScan,
    settings,
    reloadSettings,
  } = useAppData();
  const {
    runActive,
    summary,
    busy,
    startPipeline,
    actionError,
    clearActionError,
  } = usePipeline();
  const { focus, setFocus } = useTaskFocus();
  const hardsub = useHardsub();
  const [notice, setNotice] = useState<string | null>(null);
  /** ریشهٔ اسکنی که همین صفحه برای شناسایی شروع کرده — برای تفکیک از اسکن دیگر */
  const [detectingRoot, setDetectingRoot] = useState<string | null>(null);

  // هوک فقط توسعه (DEV) برای اعتبارسنجی واقعی وضعیت‌ها از طریق CDP —
  // در بیلد نهایی حذف می‌شود و هیچ دسترسی‌ای به کاربر نمی‌دهد
  useEffect(() => {
    if (import.meta.env.DEV) {
      (window as unknown as Record<string, unknown>).__subsync_dev = {
        setFolderSelection,
        setFileSelection,
        clearSelection,
        startPipeline,
      };
    }
  }, [setFolderSelection, setFileSelection, clearSelection, startPipeline]);

  // پس از پایان اسکن، ریشهٔ درخواستی پاک می‌شود؛ وضعیت واقعی از lastScan/library می‌آید
  useEffect(() => {
    if (!scanning) setDetectingRoot(null);
  }, [scanning]);

  const chooseFolder = async () => {
    if (scanning) {
      setNotice("یک اسکن در جریان است؛ پس از پایان دوباره تلاش کنید.");
      return;
    }
    setNotice(null);
    clearActionError();
    const picked = await api.pickFolder();
    if (!picked) return; // لغو دیالوگ — انتخاب فعلی می‌ماند
    setFolderSelection(picked);
    setDetectingRoot(picked);
    try {
      // همان اسکن واقعی کتابخانه: پیشرفت زنده، لغو و فیلترهای Scanner
      await startScan(picked);
    } catch (e) {
      setDetectingRoot(null);
      setNotice(`شناسایی پوشه شروع نشد: ${errorText(e)}`);
    }
  };

  const chooseFiles = async () => {
    if (scanning) {
      setNotice("یک اسکن در جریان است؛ پس از پایان دوباره تلاش کنید.");
      return;
    }
    setNotice(null);
    clearActionError();
    const picked = await api.pickVideos();
    if (!picked) return; // لغو دیالوگ
    const supported = picked.filter(isSupportedVideoFile);
    if (supported.length === 0) {
      setNotice("فایل ویدیویی پشتیبانی‌شده‌ای انتخاب نشد.");
      return;
    }
    const skipped = picked.length - supported.length;
    setFileSelection(supported);
    if (skipped > 0) {
      setNotice(`${faNum(skipped)} فایل غیر ویدیویی نادیده گرفته شد.`);
    }
  };

  // نمای تسک — فقط وقتی کاربر روی همان تسک فوکوس دارد؛ ترک آن با دکمهٔ
  // «خانه» (کار در پس‌زمینه ادامه می‌یابد و از جعبهٔ تسک‌ها دوباره باز می‌شود)
  if (focus === "pipeline" && (runActive || summary)) {
    return (
      <div className="flex h-full flex-col items-center overflow-y-auto px-6 py-8">
        <PipelinePanel onNavigate={onNavigate} />
      </div>
    );
  }
  if (focus === "hardsub" && (hardsub.runActive || hardsub.snapshot || hardsub.summary)) {
    return (
      <div className="flex h-full flex-col items-center overflow-y-auto px-6 py-8">
        <HardsubPanel onNavigate={onNavigate} />
      </div>
    );
  }

  // نمای انتخاب — روی بک‌گراند سینمایی با پنل شیشه‌ای بزرگ‌تر از قبل

  const folderSelected = selection?.kind === "folder" ? selection.path : null;
  const fileSelection = selection?.kind === "files" ? selection.paths : null;
  const folderDetection = resolveFolderDetection({
    folderSelected,
    scanning,
    detectingRoot,
    lastScan,
    library,
  });
  const detectedPaths =
    folderDetection.status === "done" ? folderDetection.paths : [];
  const canStart =
    !busy && !scanning && (fileSelection !== null || detectedPaths.length > 0);

  // شروع pipeline با دقیقاً همان فایل‌های شناسایی‌شده — هیچ اسکن دیگری انجام نمی‌شود
  const handleStartSync = () => {
    setNotice(null);
    if (selection?.kind === "files") {
      void startPipeline(selection.paths).then((ok) => {
        if (ok) setFocus("pipeline");
      });
      return;
    }
    if (folderDetection.status === "done" && folderDetection.paths.length > 0) {
      void startPipeline(folderDetection.paths).then((ok) => {
        if (ok) setFocus("pipeline");
      });
    }
  };

  // حالت پردازش — بک‌اند در شروعِ هر run از settings می‌خواند؛ فرانت فقط فرمان می‌دهد
  const automationMode: AutomationMode = settings?.automation.mode ?? "interactive";
  const setAutomationMode = async (mode: AutomationMode) => {
    if (!settings || settings.automation.mode === mode) return;
    setNotice(null);
    try {
      await api.saveSettings({
        ...settings,
        automation: { ...settings.automation, mode },
      });
      await reloadSettings();
    } catch (e) {
      setNotice(`ذخیرهٔ حالت پردازش ناموفق بود: ${errorText(e)}`);
    }
  };

  const startTitle = canStart
    ? undefined
    : busy
      ? "درخواست شروع در جریان است"
      : scanning
        ? "پس از پایان اسکن می‌توانید سینک را شروع کنید"
        : folderSelected === null && fileSelection === null
          ? "ابتدا ویدیوها یا یک پوشه را انتخاب کنید"
          : folderDetection.status === "cancelled"
            ? "شناسایی کامل نشد؛ پوشه را دوباره انتخاب کنید"
            : folderSelected !== null && detectedPaths.length === 0
              ? "در این پوشه ویدیوی قابل پردازشی پیدا نشد"
              : undefined;

  return (
    <div className="relative z-0 flex h-full flex-col items-center justify-center overflow-y-auto px-6 pb-10">
      <CinematicBackground />
      <div className="w-full max-w-2xl space-y-8 py-10">
        <div className="rounded-3xl border border-white/5 bg-zinc-900/55 p-8 shadow-2xl backdrop-blur-md sm:p-10">
          <div className="text-center">
            <h1 className="text-3xl font-bold text-ink-1 sm:text-4xl">
              زیرنویس فارسی، بدون دردسر.
            </h1>
            <p className="mt-3 text-base text-ink-2 sm:text-lg">
              فایل‌هایت را انتخاب کن؛ بقیه کار با SubSync.
            </p>
          </div>

          {/* انتخاب ورودی */}
          <div className="mt-8 grid grid-cols-1 gap-4 sm:grid-cols-2">
            <OptionCard
              icon={<Folder className="size-6" />}
              label="انتخاب پوشه"
              description="همهٔ ویدیوهای پوشه را پیدا و سینک کن"
              selected={folderSelected !== null}
              disabled={scanning}
              onClick={chooseFolder}
            />
            <OptionCard
              icon={<FileVideo className="size-6" />}
              label="انتخاب فایل"
              description="برای سینک خودکار یک یا چند فیلم"
              selected={fileSelection !== null}
              disabled={scanning}
              onClick={chooseFiles}
            />
          </div>

          {/* بازخورد انتخاب و شناسایی */}
          {selection && (
            <div className="mt-6 rounded-xl border border-white/10 bg-surface-1/80 px-5 py-4">
              <div className="flex items-start gap-3">
                <span className="mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-lg bg-accent-dim text-accent-strong">
                  <Check className="size-5" />
                </span>
                <div className="min-w-0 flex-1">
                  {folderSelected !== null && (
                    <>
                      <div
                        className="truncate text-sm font-semibold text-ink-1"
                        title={folderBaseName(folderSelected)}
                      >
                        {folderBaseName(folderSelected)}
                      </div>
                      <div
                        dir="ltr"
                        className="truncate text-start font-mono text-[11px] text-ink-3"
                        title={folderSelected}
                      >
                        {folderSelected}
                      </div>
                      <FolderDetectionStatus
                        detection={folderDetection}
                        videosFound={progress?.videosFound ?? null}
                        onStop={() => void stopScan()}
                      />
                    </>
                  )}
                  {fileSelection !== null && (
                    <>
                      <div className="text-sm font-semibold text-ink-1">
                        {faNum(fileSelection.length)} ویدیو انتخاب شد
                      </div>
                      <div
                        dir="ltr"
                        className="truncate text-start font-mono text-[11px] text-ink-3"
                        title={fileSelection.map(fileBaseName).join(" · ")}
                      >
                        {compactPreview(fileSelection)}
                      </div>
                    </>
                  )}
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  <button
                    onClick={folderSelected !== null ? chooseFolder : chooseFiles}
                    disabled={scanning}
                    className="rounded-lg px-2.5 py-1.5 text-xs text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink-1 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    تغییر
                  </button>
                  <button
                    onClick={() => {
                      clearSelection();
                      setNotice(null);
                    }}
                    disabled={scanning}
                    title="حذف انتخاب"
                    className="flex size-7 items-center justify-center rounded-lg text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink-1 disabled:cursor-not-allowed disabled:opacity-40"
                  >
                    <X className="size-3.5" />
                  </button>
                </div>
              </div>
            </div>
          )}

          {actionError && (
            <p className="mt-4 flex items-start gap-2 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-400">
              <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
              <span>{actionError}</span>
            </p>
          )}

          {notice && (
            <p className="mt-4 text-center text-sm text-amber-400/90">{notice}</p>
          )}

          {/* حالت پردازش — تصمیم با بک‌اند است؛ این فقط فرمان ذخیرهٔ تنظیم است */}
          <div className="mt-6">
            <div className="mb-2 text-center text-xs text-ink-3">حالت پردازش</div>
            <div className="grid grid-cols-2 gap-2 rounded-xl border border-white/10 bg-surface-1/60 p-1.5">
              <button
                onClick={() => void setAutomationMode("interactive")}
                disabled={!settings}
                title="هر ابهام با دیالوگ از شما پرسیده می‌شود"
                className={`flex items-center justify-center gap-2 rounded-lg px-3 py-2.5 text-xs font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
                  automationMode === "interactive"
                    ? "bg-accent text-zinc-950"
                    : "text-ink-2 hover:bg-surface-2 hover:text-ink-1"
                }`}
              >
                <Hand className="size-4" />
                تعاملی
              </button>
              <button
                onClick={() => void setAutomationMode("auto")}
                disabled={!settings}
                title="بدون توقف پردازش می‌کند؛ موارد نامطمئن به «نیازمند بررسی» می‌روند"
                className={`flex items-center justify-center gap-2 rounded-lg px-3 py-2.5 text-xs font-semibold transition-colors disabled:cursor-not-allowed disabled:opacity-40 ${
                  automationMode === "auto"
                    ? "bg-accent text-zinc-950"
                    : "text-ink-2 hover:bg-surface-2 hover:text-ink-1"
                }`}
              >
                <Zap className="size-4" />
                پردازش کامل خودکار
              </button>
            </div>
            <p className="mt-2 text-center text-[11px] leading-5 text-ink-3">
              {automationMode === "auto"
                ? "حالت خودکار: تصمیم‌های امن با امتیازدهی محتوا گرفته می‌شوند؛ فقط موارد نامطمئن برای بررسی می‌مانند."
                : "حالت تعاملی: هر جا ابهام باشد، پردازش متوقف و تصمیم از شما پرسیده می‌شود."}
            </p>
          </div>

          {/* کنش اصلی — pipeline فقط با فایل‌های واقعی شروع می‌شود */}
          <button
            onClick={handleStartSync}
            disabled={!canStart}
            title={startTitle}
            className="mt-8 h-14 w-full rounded-xl bg-accent text-lg font-bold text-zinc-950 transition-colors hover:bg-accent-strong disabled:cursor-not-allowed disabled:opacity-40"
          >
            {folderDetection.status === "scanning"
              ? "در حال شناسایی ویدیوها…"
              : scanning
                ? "اسکن در جریان است…"
                : "شروع سینک"}
          </button>
        </div>
      </div>
    </div>
  );
}

/** وضعیت شناسایی پوشه از اسکن واقعی — پوشهٔ فعلی، اسکن فعلی و library هم‌ریشه. */
function resolveFolderDetection({
  folderSelected,
  scanning,
  detectingRoot,
  lastScan,
  library,
}: {
  folderSelected: string | null;
  scanning: boolean;
  detectingRoot: string | null;
  lastScan: ScanSummary | null;
  library: VideoEntry[];
}): FolderDetection {
  if (folderSelected === null) return { status: "idle" };

  if (scanning) {
    return detectingRoot !== null && isSamePath(detectingRoot, folderSelected)
      ? { status: "scanning" }
      : { status: "elsewhere" };
  }

  // library فقط وقتی معتبر است که ریشهٔ آخرین اسکن همان پوشه باشد
  if (lastScan && isSamePath(lastScan.root, folderSelected)) {
    const paths = library
      .filter((entry) => isPathInside(entry.path, folderSelected))
      .map((entry) => entry.path);
    if (lastScan.cancelled) return { status: "cancelled", paths };
    if (paths.length === 0) return { status: "empty" };
    return { status: "done", paths };
  }

  return { status: "idle" };
}

function FolderDetectionStatus({
  detection,
  videosFound,
  onStop,
}: {
  detection: FolderDetection;
  videosFound: number | null;
  onStop: () => void;
}) {
  if (detection.status === "scanning") {
    return (
      <div className="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] leading-5 text-ink-3">
        <span className="inline-flex items-center gap-1.5 text-accent-strong">
          <Loader2 className="size-3 animate-spin" />
          در حال شناسایی ویدیوها…
        </span>
        {videosFound !== null && (
          <span>{faNum(videosFound)} ویدیو تا این لحظه</span>
        )}
        <button
          onClick={onStop}
          className="inline-flex items-center gap-1 text-red-400 transition-colors hover:text-red-300"
        >
          <Square className="size-2.5" />
          توقف
        </button>
      </div>
    );
  }

  if (detection.status === "done") {
    return (
      <div className="mt-1.5 text-[11px] leading-5 text-emerald-400">
        {faNum(detection.paths.length)} ویدیو برای سینک پیدا شد
        <span
          dir="ltr"
          className="block truncate text-start font-mono text-ink-3"
          title={detection.paths.map(fileBaseName).join(" · ")}
        >
          {compactPreview(detection.paths)}
        </span>
      </div>
    );
  }

  if (detection.status === "cancelled") {
    return (
      <p className="mt-1.5 text-[11px] leading-5 text-amber-400/90">
        شناسایی متوقف شد — {faNum(detection.paths.length)} ویدیو تا این لحظه
        پیدا شد. برای شروع، پوشه را دوباره انتخاب کنید.
      </p>
    );
  }

  if (detection.status === "empty") {
    return (
      <p className="mt-1.5 text-[11px] leading-5 text-amber-400/90">
        در این پوشه ویدیویی پیدا نشد.
      </p>
    );
  }

  if (detection.status === "elsewhere") {
    return (
      <p className="mt-1.5 text-[11px] leading-5 text-ink-3">
        اسکن دیگری در جریان است؛ پس از پایان دوباره تلاش کنید.
      </p>
    );
  }

  return (
    <p className="mt-1.5 text-[11px] leading-5 text-ink-3">
      ویدیویی شناسایی نشده است. برای شناسایی، پوشه را دوباره انتخاب کنید.
    </p>
  );
}

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

function compactPreview(paths: string[]): string {
  const names = paths.map(fileBaseName);
  if (names.length <= 3) return names.join(" · ");
  return `${names.slice(0, 3).join(" · ")} + ${faNum(names.length - 3)} مورد دیگر`;
}

function OptionCard({
  icon,
  label,
  description,
  selected,
  disabled = false,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  description: string;
  selected: boolean;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={`group flex w-full flex-col items-start gap-2 rounded-2xl border p-6 text-start transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
        selected
          ? "border-accent bg-accent-dim"
          : "border-white/10 bg-surface-1/80 hover:border-ink-3 hover:bg-surface-2/90"
      }`}
    >
      <span
        className={`flex size-12 items-center justify-center rounded-xl transition-colors ${
          selected ? "bg-accent text-zinc-950" : "bg-surface-2 text-ink-2 group-hover:text-ink-1"
        }`}
      >
        {icon}
      </span>
      <span className="mt-1 text-base font-semibold text-ink-1">{label}</span>
      <span className="text-sm leading-6 text-ink-3">{description}</span>
    </button>
  );
}
