import { AlertTriangle, Cpu, Play, RefreshCw, Square } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { Badge, Button, Card, ProgressBar, SectionTitle, StatCard } from "../components/ui";
import { faNum, formatDuration } from "../lib/format";
import type { VideoEntry } from "../types";

/** برچسب صادقانه برای فاز فعلی: فقط «وضعیت آینده» بر اساس داده موجود */
function futureStatus(v: VideoEntry) {
  if (v.siblingSubtitles.some((s) => s.nameSuggestsFa))
    return { tone: "success" as const, label: "آماده سینک (فاز ۴)" };
  if (v.siblingSubtitles.length > 0)
    return { tone: "info" as const, label: "بررسی مجوز سینک (فاز ۴)" };
  return { tone: "warn" as const, label: "نیازمند دانلود (فاز ۳)" };
}

export function Processing() {
  const { scanning, progress, lastScan, library, stopScan } = useAppData();

  const needsDownload = library.filter((v) => v.siblingSubtitles.length === 0).length;
  const readyForSync = library.filter((v) =>
    v.siblingSubtitles.some((s) => s.nameSuggestsFa),
  ).length;

  return (
    <div className="mx-auto max-w-6xl space-y-6 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">پردازش</h1>
          <p className="mt-1 text-sm text-ink-3">صف پردازش و وضعیت هر فایل</p>
        </div>
        <Button variant="primary" disabled title="در فازهای بعدی (دانلود و سینک) فعال می‌شود">
          <Play className="size-4" />
          شروع پردازش
        </Button>
      </div>

      <div className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3 text-sm text-amber-300">
        <AlertTriangle className="mt-0.5 size-4 shrink-0" />
        <span>
          در این نسخه (فاز ۱) اسکن و شناسایی فعال است. دانلود از SubSource (فاز ۳) و
          همگام‌سازی با FFsubsync (فاز ۴) پس از پیاده‌سازی، از همین صفحه کنترل می‌شوند.
          هیچ پردازش یا دانلودی به‌صورت خودکار شروع نمی‌شود.
        </span>
      </div>

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
        <>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard icon={Cpu} label="کل صف" value={faNum(lastScan.videoCount)} />
            <StatCard icon={Square} label="نیازمند دانلود (فاز ۳)" value={faNum(needsDownload)} tone="warn" />
            <StatCard icon={Play} label="آماده سینک (فاز ۴)" value={faNum(readyForSync)} tone="success" />
            <StatCard icon={RefreshCw} label="مدت آخرین اسکن" value={formatDuration(lastScan.durationMs)} />
          </div>
          <div className="text-xs text-ink-3">
            ریشه آخرین اسکن: <span dir="ltr" className="font-mono">{lastScan.root}</span>
          </div>
        </>
      )}

      <div>
        <SectionTitle title="صف پردازش" />
        {library.length === 0 ? (
          <Card className="px-4 py-10 text-center text-sm text-ink-3">
            صف خالی است — ابتدا از داشبورد اسکن بگیرید.
          </Card>
        ) : (
          <div className="max-h-[480px] overflow-y-auto rounded-xl border border-line bg-surface-1">
            <table className="w-full text-sm">
              <thead className="sticky top-0 bg-surface-1">
                <tr className="border-b border-line text-xs text-ink-3">
                  <th className="px-4 py-3 text-start font-medium">فایل</th>
                  <th className="px-4 py-3 text-start font-medium">سریال</th>
                  <th className="px-4 py-3 text-start font-medium">S/E</th>
                  <th className="px-4 py-3 text-start font-medium">مرحله بعدی</th>
                </tr>
              </thead>
              <tbody>
                {library.slice(0, 300).map((v) => {
                  const st = futureStatus(v);
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
    </div>
  );
}
