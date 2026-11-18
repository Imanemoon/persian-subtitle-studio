import { useEffect, useState } from "react";
import { Check, FileVideo, Folder, X } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { usePipeline } from "../hooks/usePipeline";
import { PipelinePanel } from "../components/pipeline/PipelinePanel";
import { fileBaseName, folderBaseName, isSupportedVideoFile } from "../lib/videoFiles";
import { api } from "../lib/api";
import { faNum } from "../lib/format";
import type { PageId } from "../types";

/**
 * صفحهٔ خانه — نقطهٔ شروع pipeline واقعی بک‌اند.
 * وقتی run فعالی جریان دارد (یا خلاصهٔ تازه‌ای هست) نمای pipeline نشان داده
 * می‌شود؛ در غیر این صورت نمای انتخاب. هیچ پیشرفت جعلی‌ای وجود ندارد.
 */
export function Home({ onNavigate }: { onNavigate: (page: PageId) => void }) {
  const { selection, setFolderSelection, setFileSelection, clearSelection } =
    useAppData();
  const { runActive, summary, busy, startPipeline } = usePipeline();
  const [notice, setNotice] = useState<string | null>(null);

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

  // شروع pipeline با دقیقاً همان فایل‌های انتخاب‌شده — هیچ اسکن والد انجام نمی‌شود
  const handleStartSync = () => {
    if (!selection || selection.kind !== "files") return;
    void startPipeline(selection.paths);
  };

  // نمای pipeline — run فعال یا خلاصهٔ پایان (تا «بستن خلاصه»)
  if (runActive || summary) {
    return (
      <div className="flex h-full flex-col items-center overflow-y-auto px-6 py-8">
        <PipelinePanel onNavigate={onNavigate} />
      </div>
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
            فایل‌هایت را انتخاب کن؛ بقیه کار با SubSync.
          </p>
        </div>

        {/* انتخاب ورودی */}
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
          <OptionCard
            icon={<Folder className="size-5" />}
            label="انتخاب پوشه"
            description="برای ساخت کتابخانه از سریال یا فصل"
            selected={folderSelected !== null}
            onClick={chooseFolder}
          />
          <OptionCard
            icon={<FileVideo className="size-5" />}
            label="انتخاب فایل"
            description="برای سینک خودکار یک یا چند فیلم"
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
                    setNotice(null);
                  }}
                  title="حذف انتخاب"
                  className="flex size-7 items-center justify-center rounded-lg text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink-1"
                >
                  <X className="size-3.5" />
                </button>
              </div>
            </div>
          </div>
        )}

        {notice && (
          <p className="text-center text-xs text-amber-400/90">{notice}</p>
        )}

        {/* کنش اصلی — pipeline فقط با انتخاب دقیق فایل‌ها شروع می‌شود */}
        <button
          onClick={handleStartSync}
          disabled={fileSelection === null || busy}
          title={
            fileSelection === null
              ? "برای شروع، فایل‌های ویدیویی را انتخاب کنید"
              : undefined
          }
          className="h-12 w-full rounded-xl bg-accent text-base font-bold text-zinc-950 transition-colors hover:bg-accent-strong disabled:cursor-not-allowed disabled:opacity-40"
        >
          شروع سینک
        </button>

        {folderSelected !== null && (
          <p className="text-center text-[11px] leading-5 text-ink-3">
            در این نسخه سینک خودکار با انتخاب دقیق فایل‌ها شروع می‌شود؛ برای
            پردازش کل پوشه، از صفحهٔ «پردازش» کتابخانه را بسازید.
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
