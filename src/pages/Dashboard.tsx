import { useMemo } from "react";
import {
  AlertTriangle,
  Captions,
  Clock,
  Film,
  FolderOpen,
  HardDrive,
  HelpCircle,
  Languages,
  Layers,
  Play,
  RefreshCw,
  Square,
  Tv,
} from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  ProgressBar,
  SectionTitle,
  StatCard,
} from "../components/ui";
import { api } from "../lib/api";
import { faNum, formatBytes, formatDuration } from "../lib/format";
import type { PageId, ToolInfo, ToolStatus } from "../types";

function toolBadge(status: ToolStatus) {
  switch (status) {
    case "found":
      return { tone: "success" as const, label: "آماده" };
    case "missing":
      return { tone: "warn" as const, label: "پیدا نشد" };
    case "broken":
      return { tone: "danger" as const, label: "خراب" };
  }
}

function ToolRow({ tool }: { tool: ToolInfo }) {
  const badge = toolBadge(tool.status);
  return (
    <div className="flex items-center gap-3 rounded-lg border border-line bg-surface-2 px-3 py-2.5">
      <span
        className={`size-2 shrink-0 rounded-full ${
          tool.status === "found"
            ? "bg-emerald-500"
            : tool.status === "missing"
              ? "bg-amber-500"
              : "bg-red-500"
        }`}
      />
      <span className="w-44 shrink-0 text-sm text-ink-1">{tool.name}</span>
      <Badge tone={badge.tone}>{badge.label}</Badge>
      <span
        dir="ltr"
        className="min-w-0 flex-1 truncate font-mono text-xs text-ink-3"
        title={tool.path ?? tool.detail ?? undefined}
      >
        {tool.path ?? tool.detail ?? "—"}
      </span>
      {tool.version && (
        <span dir="ltr" className="hidden shrink-0 font-mono text-[11px] text-ink-3 xl:block">
          {tool.version}
        </span>
      )}
    </div>
  );
}

export function Dashboard({ onNavigate }: { onNavigate: (page: PageId) => void }) {
  const {
    library,
    lastScan,
    progress,
    scanning,
    tools,
    settings,
    error,
    setError,
    startScan,
    stopScan,
    refreshTools,
    toolsLoading,
  } = useAppData();

  const stats = useMemo(() => {
    const shows = new Set<string>();
    const seasons = new Set<string>();
    let totalBytes = 0;
    let withAny = 0;
    let withFa = 0;
    let unparsed = 0;
    for (const v of library) {
      if (v.showName) shows.add(v.showName.toLowerCase());
      if (v.showName && v.season != null) seasons.add(`${v.showName.toLowerCase()}|${v.season}`);
      totalBytes += v.sizeBytes;
      if (v.siblingSubtitles.length > 0) withAny += 1;
      if (v.siblingSubtitles.some((s) => s.nameSuggestsFa)) withFa += 1;
      if (v.season == null && v.episode == null) unparsed += 1;
    }
    return { total: library.length, shows: shows.size, seasons: seasons.size, totalBytes, withAny, withFa, unparsed };
  }, [library]);

  const scanWithPicker = async () => {
    setError(null);
    const picked = await api.pickFolder();
    if (picked) await startScan(picked).catch(() => undefined);
  };

  const scanRoot = async () => {
    if (!settings?.rootFolder) {
      await scanWithPicker();
      return;
    }
    await startScan(settings.rootFolder).catch(() => undefined);
  };

  return (
    <div className="mx-auto max-w-6xl space-y-6 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold text-ink-1">داشبورد</h1>
          <p className="mt-1 text-sm text-ink-3">
            {settings?.rootFolder
              ? "پوشه ریشه ثبت‌شده را اسکن کنید یا یک پوشه جدید انتخاب کنید."
              : "برای شروع، یک پوشه ریشه انتخاب و اسکن کنید."}
          </p>
        </div>
        <div className="flex gap-2">
          <Button onClick={scanWithPicker}>
            <FolderOpen className="size-4" />
            انتخاب پوشه…
          </Button>
          <Button variant="primary" onClick={scanRoot} disabled={scanning} loading={scanning}>
            <Play className="size-4" />
            اسکن
          </Button>
        </div>
      </div>

      {error && (
        <div className="flex items-center justify-between gap-3 rounded-lg border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-300">
          <span className="flex items-center gap-2">
            <AlertTriangle className="size-4 shrink-0" />
            {error}
          </span>
          <button onClick={() => setError(null)} className="text-red-300 hover:text-red-200">
            ✕
          </button>
        </div>
      )}

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
              <span>فایل‌های بررسی‌شده: {faNum(progress.filesSeen)}</span>
              <span>پوشه‌ها: {faNum(progress.dirsScanned)}</span>
              <span dir="ltr" className="min-w-0 flex-1 truncate font-mono text-left" title={progress.currentPath}>
                {progress.currentPath}
              </span>
            </div>
          )}
        </Card>
      )}

      {!scanning && library.length === 0 ? (
        <EmptyState
          icon={Film}
          title="هنوز کتابخانه‌ای وجود ندارد"
          description="یک پوشه ریشه (مثلاً پوشه سریال‌ها) انتخاب کنید تا همه ویدیوهای داخل آن به‌صورت بازگشتی شناسایی شوند. پوشه‌های مستثنا از تنظیمات اعمال می‌شوند."
          action={
            <>
              <Button variant="primary" onClick={scanWithPicker}>
                <FolderOpen className="size-4" />
                انتخاب پوشه و اسکن
              </Button>
              <Button onClick={() => onNavigate("settings")}>تنظیمات</Button>
            </>
          }
        />
      ) : (
        <>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard icon={Film} label="کل ویدیوها" value={faNum(stats.total)} sub={`حجم کل: ${formatBytes(stats.totalBytes)}`} />
            <StatCard icon={Tv} label="سریال‌ها" value={faNum(stats.shows)} tone="info" />
            <StatCard icon={Layers} label="فصل‌ها" value={faNum(stats.seasons)} tone="info" />
            <StatCard
              icon={Captions}
              label="دارای زیرنویس کناری"
              value={faNum(stats.withAny)}
              tone={stats.withAny > 0 ? "success" : "neutral"}
              sub={`${faNum(stats.withFa)} مورد با نشانه فارسی`}
            />
            <StatCard icon={Languages} label="زیرنویس فارسی کناری" value={faNum(stats.withFa)} tone="success" />
            <StatCard icon={HelpCircle} label="بدون شناسه فصل/قسمت" value={faNum(stats.unparsed)} tone={stats.unparsed > 0 ? "warn" : "neutral"} />
            <StatCard
              icon={Clock}
              label="آخرین اسکن"
              value={lastScan ? formatDuration(lastScan.durationMs) : "—"}
              sub={
                lastScan
                  ? `${faNum(lastScan.dirsScanned)} پوشه، ${faNum(lastScan.filesSeen)} فایل${lastScan.cancelled ? " (لغوشده)" : ""}`
                  : "اسکنی ثبت نشده"
              }
            />
            <StatCard icon={HardDrive} label="وضعیت ابزارها" value={`${faNum(tools.filter((t) => t.status === "found").length)}/${faNum(tools.length)}`} tone={tools.every((t) => t.status === "found") ? "success" : "warn"} />
          </div>

          {lastScan && (
            <Card className="flex flex-wrap items-center gap-x-6 gap-y-1 px-4 py-3 text-xs text-ink-3">
              <span dir="ltr" className="min-w-0 max-w-full truncate font-mono">
                {lastScan.root}
              </span>
              {lastScan.cancelled && <Badge tone="warn">لغوشده</Badge>}
              <span className="ms-auto">اسکن بعدی فقط با کلیک شما اجرا می‌شود.</span>
            </Card>
          )}
        </>
      )}

      <div>
        <SectionTitle
          title="ابزارهای خارجی"
          extra={
            <Button onClick={refreshTools} loading={toolsLoading} className="h-8 px-3 text-xs">
              <RefreshCw className="size-3.5" />
              بررسی دوباره
            </Button>
          }
        />
        <div className="space-y-2">
          {tools.map((t) => (
            <ToolRow key={t.id} tool={t} />
          ))}
        </div>
      </div>
    </div>
  );
}
