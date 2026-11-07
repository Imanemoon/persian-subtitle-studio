import { useEffect, useState } from "react";
import {
  Check,
  FileVideo,
  Folder,
  Info,
  Square,
  X,
} from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { SYNC_STAGE_LABELS, type SyncRunState, type SyncStage } from "../lib/pipeline";
import { fileBaseName, folderBaseName, isSupportedVideoFile } from "../lib/videoFiles";
import { api } from "../lib/api";
import { faNum } from "../lib/format";
import type { PageId } from "../types";

// ---------------------------------------------------------------------------
// نمای پردازش خانه — طراحی برای اتصال به orchestrator آینده.
// فقط وقتی syncRun واقعی وجود داشته باشد رندر می‌شود؛ تا آن زمان هیچ
// مسیری آن را تولید نمی‌کند و هیچ پیشرفت جعلی‌ای نشان داده نمی‌شود.
// ---------------------------------------------------------------------------

function SyncProcessingView({
  run,
  onStop,
  onDetails,
}: {
  run: SyncRunState;
  onStop: () => void;
  onDetails: () => void;
}) {
  const percent = run.total > 0 ? Math.round((run.completed / run.total) * 100) : 0;
  const stageLabel: string = SYNC_STAGE_LABELS[run.stage as SyncStage];
  return (
    <div className="flex h-full flex-col items-center justify-center px-6">
      <div className="w-full max-w-md space-y-5 text-center">
        <div className="text-sm font-semibold text-accent-strong">
          {stageLabel}
          {run.stage !== "done" && (
            <span className="ms-1 inline-block h-4 w-2 animate-pulse rounded-sm bg-accent align-middle" />
          )}
        </div>
        {run.currentFile && (
          <div dir="ltr" className="truncate font-mono text-xs text-ink-2" title={run.currentFile}>
            {run.currentFile}
          </div>
        )}
        <div className="h-2 w-full overflow-hidden rounded-full bg-zinc-800">
          <div
            className="h-full rounded-full bg-accent transition-all"
            style={{ width: `${percent}%` }}
          />
        </div>
        <div className="flex items-center justify-between text-xs text-ink-3">
          <span>{faNum(percent)}٪</span>
          <span>
            {faNum(run.completed)} از {faNum(run.total)} فایل
          </span>
        </div>
        <div className="flex items-center justify-center gap-2 pt-2">
          <button
            onClick={onStop}
            className="inline-flex h-9 items-center gap-2 rounded-lg border border-red-500/30 bg-red-500/10 px-4 text-xs text-red-300 transition-colors hover:bg-red-500/20"
          >
            <Square className="size-3.5" />
            توقف
          </button>
          <button
            onClick={onDetails}
            className="inline-flex h-9 items-center rounded-lg px-3 text-xs text-ink-3 transition-colors hover:text-ink-1"
          >
            مشاهده جزئیات
          </button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// صفحهٔ خانه مینیمال
// ---------------------------------------------------------------------------

export function Home({ onNavigate }: { onNavigate: (page: PageId) => void }) {
  const {
    selection,
    setFolderSelection,
    setFileSelection,
    clearSelection,
    syncRun,
    startScan,
  } = useAppData();
  const [syncHintShown, setSyncHintShown] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  // هوک فقط توسعه (DEV) برای اعتبارسنجی واقعی وضعیت‌ها از طریق CDP —
  // در بیلد نهایی حذف می‌شود و هیچ دسترسی‌ای به کاربر نمی‌دهد
  useEffect(() => {
    if (import.meta.env.DEV) {
      (window as unknown as Record<string, unknown>).__subsync_dev = {
        setFolderSelection,
        setFileSelection,
        clearSelection,
      };
    }
  }, [setFolderSelection, setFileSelection, clearSelection]);

  const chooseFolder = async () => {
    setNotice(null);
    const picked = await api.pickFolder();
    if (!picked) return; // لغو دیالوگ — انتخاب فعلی می‌ماند
    setFolderSelection(picked);
  };

  const chooseFiles = async () => {
    setNotice(null);
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

  // کنش اصلی برنامه — تا اتصال orchestrator واقعی فقط توضیح صادقانه نشان می‌دهد
  const handleStartSync = () => {
    if (!selection) return;
    setSyncHintShown(true);
  };

  if (syncRun) {
    return (
      <SyncProcessingView
        run={syncRun}
        onStop={() => undefined}
        onDetails={() => onNavigate("processing")}
      />
    );
  }

  const folderSelected = selection?.kind === "folder" ? selection.path : null;
  const fileSelection = selection?.kind === "files" ? selection.paths : null;

  return (
    <div className="flex h-full flex-col items-center justify-center overflow-y-auto px-6 pb-10">
      <div className="w-full max-w-lg space-y-7 py-8">
        <div className="text-center">
          <h1 className="text-2xl font-bold text-ink-1">زیرنویس فارسی، بدون دردسر.</h1>
          <p className="mt-2 text-sm text-ink-2">
            پوشه یا فایل‌هایت را انتخاب کن؛ بقیه کار با SubSync.
          </p>
        </div>

        {/* انتخاب ورودی */}
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <OptionCard
            icon={<Folder className="size-5" />}
            label="انتخاب پوشه"
            description="برای سریال، فصل یا مجموعه‌ای از ویدیوها"
            selected={folderSelected !== null}
            onClick={chooseFolder}
          />
          <OptionCard
            icon={<FileVideo className="size-5" />}
            label="انتخاب فایل"
            description="برای یک فیلم یا چند فایل ویدیویی"
            selected={fileSelection !== null}
            onClick={chooseFiles}
          />
        </div>

        {/* بازخورد انتخاب */}
        {selection && (
          <div className="rounded-xl border border-line bg-surface-1 px-4 py-3">
            <div className="flex items-center gap-3">
              <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-accent-dim text-accent-strong">
                <Check className="size-4" />
              </span>
              <div className="min-w-0 flex-1">
                {folderSelected !== null && (
                  <>
                    <div className="truncate text-sm font-semibold text-ink-1" title={folderBaseName(folderSelected)}>
                      {folderBaseName(folderSelected)}
                    </div>
                    <div
                      dir="ltr"
                      className="truncate text-start font-mono text-[11px] text-ink-3"
                      title={folderSelected}
                    >
                      {folderSelected}
                    </div>
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
                  className="rounded-lg px-2.5 py-1.5 text-xs text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink-1"
                >
                  تغییر
                </button>
                <button
                  onClick={() => {
                    clearSelection();
                    setSyncHintShown(false);
                  }}
                  title="حذف انتخاب"
                  className="flex size-7 items-center justify-center rounded-lg text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink-1"
                >
                  <X className="size-3.5" />
                </button>
              </div>
            </div>
            {/* امکان اسکن واقعی فقط در بخش پیشرفته — از جریان عادی حذف شده */}
            {folderSelected !== null && (
              <button
                onClick={() => {
                  setNotice(null);
                  startScan(folderSelected).catch(() => undefined);
                }}
                className="mt-2 text-[11px] text-ink-3 transition-colors hover:text-accent-strong"
              >
                اسکن این پوشه در کتابخانه (پیشرفته)
              </button>
            )}
          </div>
        )}

        {notice && (
          <p className="text-center text-xs text-amber-400/90">{notice}</p>
        )}

        {/* کنش اصلی برنامه */}
        <button
          onClick={handleStartSync}
          disabled={!selection}
          className="h-12 w-full rounded-xl bg-accent text-base font-bold text-zinc-950 transition-colors hover:bg-accent-strong disabled:cursor-not-allowed disabled:opacity-40"
        >
          شروع سینک
        </button>

        {syncHintShown && (
          <p className="flex items-start justify-center gap-1.5 text-center text-[11px] leading-5 text-ink-3">
            <Info className="mt-0.5 size-3.5 shrink-0" />
            موتور کامل سینک (دانلود و هماهنگ‌سازی خودکار) در نسخه‌های بعدی فعال
            می‌شود. فعلاً از بخش پیشرفته می‌توانید کتابخانه را بسازید و زیرنویس
            داخلی را تحلیل و استخراج کنید.
          </p>
        )}
      </div>
    </div>
  );
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
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  description: string;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`group flex w-full flex-col items-start gap-1.5 rounded-xl border p-4 text-start transition-colors ${
        selected
          ? "border-accent bg-accent-dim"
          : "border-line bg-surface-1 hover:border-ink-3 hover:bg-surface-2"
      }`}
    >
      <span
        className={`flex size-10 items-center justify-center rounded-lg transition-colors ${
          selected ? "bg-accent text-zinc-950" : "bg-surface-2 text-ink-2 group-hover:text-ink-1"
        }`}
      >
        {icon}
      </span>
      <span className="text-sm font-semibold text-ink-1">{label}</span>
      <span className="text-xs leading-5 text-ink-3">{description}</span>
    </button>
  );
}
