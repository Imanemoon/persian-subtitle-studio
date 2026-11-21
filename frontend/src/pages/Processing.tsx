import { useState } from "react";
import { AlertTriangle, FolderOpen, RefreshCw, Square } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { usePipeline } from "../hooks/usePipeline";
import { useHardsub } from "../hooks/useHardsub";
import { HardsubPanel } from "../components/hardsub/HardsubPanel";
import { PipelinePanel } from "../components/pipeline/PipelinePanel";
import { Badge, Button, Card, ProgressBar, SectionTitle } from "../components/ui";
import { api } from "../lib/api";
import { faNum, formatDuration } from "../lib/format";
import type { PageId } from "../types";

export function Processing({ onNavigate }: { onNavigate: (page: PageId) => void }) {
  const { scanning, progress, lastScan, library, stopScan, startScan } = useAppData();
  const { runActive, snapshot } = usePipeline();
  const hardsub = useHardsub();
  const [scanRoot, setScanRoot] = useState<string | null>(null);

  const pickAndScan = async () => {
    if (scanRoot) {
      await startScan(scanRoot).catch(() => undefined);
      return;
    }
    const picked = await api.pickFolder();
    if (!picked) return;
    setScanRoot(picked);
    await startScan(picked).catch(() => undefined);
  };

  return (
    <div className="mx-auto max-w-6xl space-y-6 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">پردازش</h1>
          <p className="mt-1 text-sm text-ink-3">
            pipeline کامل و وضعیت هر فایل — سینک از صفحهٔ خانه شروع می‌شود
          </p>
        </div>
      </div>

      {/* pipeline فعال — همان نمای صفحهٔ خانه؛ بازیابی از snapshot/رویدادها */}
      {runActive && snapshot && (
        <div className="flex justify-center">
          <PipelinePanel onNavigate={onNavigate} />
        </div>
      )}

      {/* هاردساب فعال یا نتیجهٔ آن — مستقل از pipeline */}
      {(hardsub.runActive || hardsub.snapshot || hardsub.summary) && (
        <div className="flex justify-center">
          <HardsubPanel onNavigate={onNavigate} />
        </div>
      )}

      {/* اسکن کتابخانه — قابلیت پیشرفته (از داشبورد قبلی به اینجا منتقل شده) */}
      <Card className="flex flex-wrap items-center gap-3 p-4">
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-ink-1">اسکن کتابخانه</div>
          <div
            dir="ltr"
            className="truncate text-start font-mono text-[11px] text-ink-3"
            title={scanRoot ?? undefined}
          >
            {scanRoot ?? "پوشه‌ای انتخاب نشده — با دکمهٔ روبه‌رو انتخاب کنید"}
          </div>
        </div>
        <Button onClick={() => setScanRoot(null)} disabled={scanning} title="پاک‌کردن پوشهٔ انتخابی">
          <RefreshCw className="size-4" />
          تغییر پوشه
        </Button>
        <Button
          variant="primary"
          onClick={pickAndScan}
          disabled={scanning}
          loading={scanning}
        >
          <FolderOpen className="size-4" />
          انتخاب پوشه و اسکن
        </Button>
      </Card>

      {scanning && (
        <Card className="space-y-3 p-4">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2 text-sm font-semibold text-accent-strong">
              <RefreshCw className="size-4 animate-spin" />
              اسکن در حال اجرا…
            </div>
            <Button variant="danger" onClick={stopScan}>
              <Square className="size-4" />
              توقف
            </Button>
          </div>
          <ProgressBar active />
          {progress && (
            <div className="flex flex-wrap items-center gap-x-6 gap-y-1 text-xs text-ink-3">
              <span>ویدیوها: {faNum(progress.videosFound)}</span>
              <span>فایل‌ها: {faNum(progress.filesSeen)}</span>
              <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-left" title={progress.currentPath}>
                {progress.currentPath}
              </span>
            </div>
          )}
        </Card>
      )}

      {lastScan && (
        <div className="text-xs text-ink-3">
          ریشه آخرین اسکن: <span dir="ltr" className="font-mono">{lastScan.root}</span>
          {" · "}مدت: {formatDuration(lastScan.durationMs)}
        </div>
      )}

      <div>
        <SectionTitle title="کتابخانه" />
        {library.length === 0 ? (
          <Card className="px-4 py-10 text-center text-sm text-ink-3">
            کتابخانه خالی است — با «انتخاب پوشه و اسکن» در همین صفحه بسازید؛
            برای سینک خودکار، فایل‌ها یا پوشه را از صفحهٔ خانه انتخاب کنید.
          </Card>
        ) : (
          <div className="max-h-[480px] overflow-y-auto rounded-xl border border-line bg-surface-1">
            <table className="w-full text-sm">
              <thead className="sticky top-0 bg-surface-1">
                <tr className="border-b border-line text-xs text-ink-3">
                  <th className="px-4 py-3 text-start font-medium">فایل</th>
                  <th className="px-4 py-3 text-start font-medium">سریال</th>
                  <th className="px-4 py-3 text-start font-medium">S/E</th>
                  <th className="px-4 py-3 text-start font-medium">وضعیت زیرنویس کناری</th>
                </tr>
              </thead>
              <tbody>
                {library.slice(0, 300).map((v) => {
                  const st = sidecarStatus(v.siblingSubtitles.some((s) => s.nameSuggestsFa), v.siblingSubtitles.length);
                  return (
                    <tr key={v.id} className="border-b border-line/60 last:border-0">
                      <td dir="ltr" className="max-w-80 px-4 py-2.5 text-start">
                        <span className="block truncate font-mono text-xs" title={v.fileName}>
                          {v.fileName}
                        </span>
                      </td>
                      <td className="px-4 py-2.5 text-ink-2">{v.showName ?? "—"}</td>
                      <td dir="ltr" className="px-4 py-2.5 text-start font-mono text-xs text-ink-2">
                        {v.season != null && v.episode != null
                          ? `S${String(v.season).padStart(2, "0")}E${String(v.episode).padStart(2, "0")}`
                          : "—"}
                      </td>
                      <td className="px-4 py-2.5">
                        <Badge tone={st.tone}>{st.label}</Badge>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      <div className="flex items-start gap-2 rounded-lg border border-sky-500/30 bg-sky-500/10 px-4 py-3 text-sm text-sky-300">
        <AlertTriangle className="mt-0.5 size-4 shrink-0" />
        <span>
          سینک کامل (دانلود، استخراج، همگام‌سازی) با انتخاب فایل‌ها یا پوشه از
          صفحهٔ خانه شروع می‌شود و نتیجهٔ هر فایل همان‌جا و همین‌جا دیده
          می‌شود. هیچ فایل اصلی یا زیرنویس موجود بازنویسی نمی‌شود.
        </span>
      </div>
    </div>
  );
}

/** وضعیت صادقانهٔ sidecar کنار هر ویدیو — فقط دادهٔ واقعی اسکن */
function sidecarStatus(hasFa: boolean, count: number): {
  tone: "success" | "info" | "neutral";
  label: string;
} {
  if (hasFa) return { tone: "success", label: "زیرنویس فارسی کنار فایل" };
  if (count > 0) return { tone: "info", label: "زیرنویس غیرفارسی/نامشخص" };
  return { tone: "neutral", label: "بدون زیرنویس کناری" };
}
