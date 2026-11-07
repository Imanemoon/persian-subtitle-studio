import { useMemo, useState } from "react";
import {
  Captions,
  FileVideo,
  Film,
  Languages,
  Loader2,
  RefreshCw,
  ScanLine,
  Search,
  Sparkles,
  Square,
  X,
} from "lucide-react";
import { ask } from "@tauri-apps/plugin-dialog";
import { useAppData } from "../hooks/useAppData";
import { Badge, Button, EmptyState, Modal } from "../components/ui";
import { faNum, formatBytes, formatTimestamp } from "../lib/format";
import type {
  AnalysisStatus,
  ExtractionPlan,
  SubtitleKind,
  SubtitleStream,
  TrackReview,
  VideoEntry,
} from "../types";

type StatusFilter = "all" | "fa" | "any" | "none" | "unparsed" | "analysis";

const STATUS_OPTIONS: { id: StatusFilter; label: string }[] = [
  { id: "all", label: "همه" },
  { id: "fa", label: "زیرنویس فارسی کناری" },
  { id: "any", label: "زیرنویس کناری (هر نوعی)" },
  { id: "none", label: "بدون زیرنویس کناری" },
  { id: "unparsed", label: "بدون شناسه S/E" },
  { id: "analysis", label: "فقط تحلیل‌شده‌ها" },
];

const ANALYSIS_BADGES: Record<
  AnalysisStatus,
  { tone: "neutral" | "success" | "warn" | "danger" | "info"; label: string }
> = {
  notAnalyzed: { tone: "neutral", label: "تحلیل نشده" },
  analyzing: { tone: "info", label: "در حال تحلیل…" },
  persianTextTrackFound: { tone: "success", label: "Track متنی فارسی" },
  imageSubtitleFound: { tone: "warn", label: "زیرنویس تصویری" },
  noEmbeddedSubtitle: { tone: "neutral", label: "بدون زیرنویس داخلی" },
  noPersianTrack: { tone: "neutral", label: "بدون Track فارسی" },
  extracting: { tone: "info", label: "در حال استخراج…" },
  extracted: { tone: "success", label: "استخراج شد" },
  invalidSubtitle: { tone: "danger", label: "زیرنویس نامعتبر" },
  needsTrackSelection: { tone: "warn", label: "نیازمند انتخاب Track" },
  failed: { tone: "danger", label: "خطا" },
};

function kindBadge(kind: SubtitleKind) {
  switch (kind) {
    case "text":
      return { tone: "success" as const, label: "متنی" };
    case "image":
      return { tone: "warn" as const, label: "تصویری (OCR)" };
    default:
      return { tone: "neutral" as const, label: "نامشخص" };
  }
}

function StreamRow({
  stream,
  busy,
  onExtract,
  onReview,
}: {
  stream: SubtitleStream;
  busy: boolean;
  onExtract: (streamIndex: number) => void;
  onReview: (streamIndex: number) => void;
}) {
  const kind = kindBadge(stream.kind);
  const faLike = stream.persianCandidate || stream.persianConfirmed;
  return (
    <div className="rounded-lg border border-line bg-surface-2 px-3 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        <span dir="ltr" className="font-mono text-xs text-ink-1">
          #{stream.streamIndex}
        </span>
        <span dir="ltr" className="font-mono text-xs text-ink-2">
          {stream.codecName}
        </span>
        <Badge tone={kind.tone}>{kind.label}</Badge>
        {stream.persianCandidate && (
          <Badge tone="success">
            <Languages className="size-3" />
            کاندید فارسی
          </Badge>
        )}
        {stream.persianConfirmed && (
          <Badge tone="success">
            <Languages className="size-3" />
            فارسی تأییدشده
          </Badge>
        )}
        {stream.defaultDisposition && <Badge tone="info">پیش‌فرض</Badge>}
        {stream.forcedDisposition && <Badge tone="info">Forced</Badge>}
        <span className="ms-auto text-xs text-ink-3">
          {stream.language ?? stream.languageIetf ?? "—"}
        </span>
      </div>
      {stream.title && (
        <div className="mt-1 truncate text-xs text-ink-2" title={stream.title}>
          {stream.title}
        </div>
      )}
      {stream.kind === "text" && (
        <div className="mt-2 flex flex-wrap gap-2">
          {faLike && (
            <Button
              className="h-8 px-3 text-xs"
              variant="primary"
              disabled={busy}
              onClick={() => onExtract(stream.streamIndex)}
            >
              <Sparkles className="size-3.5" />
              استخراج این Track
            </Button>
          )}
          <Button
            className="h-8 px-3 text-xs"
            disabled={busy}
            onClick={() => onReview(stream.streamIndex)}
            title="خروجی آزمایشی به .track-N.review.srt — با بررسی محتوای واقعی"
          >
            <ScanLine className="size-3.5" />
            استخراج آزمایشی برای بررسی
          </Button>
        </div>
      )}
      {stream.kind === "image" && stream.persianCandidate && (
        <div className="mt-2 text-xs text-amber-400">
          استخراج متن این Track به OCR نیاز دارد (در فازهای بعدی).
        </div>
      )}
    </div>
  );
}

function ReviewCard({
  review,
  busy,
  onPromote,
}: {
  review: TrackReview;
  busy: boolean;
  onPromote: (streamIndex: number) => void;
}) {
  return (
    <div
      className={`rounded-lg border px-3 py-2 text-xs ${
        review.persianConfirmed
          ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
          : "border-line bg-surface-2 text-ink-2"
      }`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span dir="ltr" className="font-mono">
          #{review.streamIndex}
        </span>
        <span>بررسی:</span>
        <span dir="ltr" className="min-w-0 truncate font-mono" title={review.reviewPath}>
          {review.reviewPath.split(/[\\/]/).pop()}
        </span>
        {review.persianConfirmed ? (
          <Badge tone="success">فارسی تأیید شد</Badge>
        ) : (
          <Badge tone="warn">فارسی تأیید نشد</Badge>
        )}
        {review.promoted && <Badge tone="info">تبدیل شد</Badge>}
      </div>
      <div className="mt-1 text-[11px] text-ink-3">
        {faNum(review.validation.cueCount)} cue — {review.validation.encoding} —{" "}
        {formatBytes(review.validation.sizeBytes)}
      </div>
      {review.persianConfirmed && !review.promoted && (
        <div className="mt-2">
          <Button
            className="h-8 px-3 text-xs"
            variant="primary"
            disabled={busy}
            onClick={() => onPromote(review.streamIndex)}
          >
            <Sparkles className="size-3.5" />
            تبدیل به .fa.srt استاندارد
          </Button>
        </div>
      )}
    </div>
  );
}

function DetailsDrawer({
  video,
  busy,
  onClose,
  onAnalyze,
  onExtract,
  onReview,
  onPromote,
}: {
  video: VideoEntry;
  busy: boolean;
  onClose: () => void;
  onAnalyze: (path: string) => void;
  onExtract: (path: string, streamIndex: number) => void;
  onReview: (path: string, streamIndex: number) => void;
  onPromote: (path: string, streamIndex: number) => void;
}) {
  const analysis = video.analysis;
  return (
    <div className="fixed inset-0 z-40 bg-black/60" onClick={onClose}>
      <div
        dir="rtl"
        className="absolute inset-y-0 start-0 w-[460px] overflow-y-auto border-e border-line bg-surface-1 p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-start justify-between gap-3">
          <h3 className="break-all text-sm font-bold text-ink-1">{video.fileName}</h3>
          <button
            onClick={onClose}
            className="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink-1"
          >
            <X className="size-4" />
          </button>
        </div>

        <dl className="space-y-3 text-sm">
          <div className="flex gap-6">
            <div>
              <dt className="text-xs text-ink-3">نام سریال</dt>
              <dd className="mt-0.5">{video.showName ?? "—"}</dd>
            </div>
            <div>
              <dt className="text-xs text-ink-3">فصل / قسمت</dt>
              <dd className="mt-0.5">
                {video.season != null ? faNum(video.season) : "—"} /{" "}
                {video.episode != null ? faNum(video.episode) : "—"}
              </dd>
            </div>
            <div>
              <dt className="text-xs text-ink-3">حجم</dt>
              <dd className="mt-0.5">{formatBytes(video.sizeBytes)}</dd>
            </div>
          </div>
          <div>
            <dt className="text-xs text-ink-3">مسیر کامل</dt>
            <dd
              dir="ltr"
              className="mt-1 break-all rounded-lg bg-surface-2 p-2 font-mono text-xs text-ink-2"
            >
              {video.path}
            </dd>
          </div>

          <div className="border-t border-line pt-3">
            <div className="mb-2 flex items-center justify-between">
              <dt className="text-xs font-semibold text-ink-1">زیرنویس‌های داخلی</dt>
              <Button
                className="h-8 px-3 text-xs"
                onClick={() => onAnalyze(video.path)}
                disabled={busy || analysis?.status === "analyzing"}
                loading={analysis?.status === "analyzing"}
              >
                <ScanLine className="size-3.5" />
                تحلیل با FFprobe
              </Button>
            </div>

            {!analysis && (
              <p className="text-xs text-ink-3">
                هنوز تحلیل نشده — با دکمه بالا Streamهای زیرنویس داخلی بررسی می‌شوند.
              </p>
            )}

            {analysis && (
              <div className="space-y-2">
                <div className="flex flex-wrap items-center gap-2">
                  <Badge tone={ANALYSIS_BADGES[analysis.status].tone}>
                    {ANALYSIS_BADGES[analysis.status].label}
                  </Badge>
                  <span className="text-[11px] text-ink-3">
                    {formatTimestamp(analysis.analyzedAtMs)}
                  </span>
                </div>
                {analysis.message && (
                  <p className="text-xs leading-5 text-ink-2">{analysis.message}</p>
                )}
                {analysis.streams.length === 0 && analysis.status !== "analyzing" && (
                  <p className="text-xs text-ink-3">هیچ Stream زیرنویسی یافت نشد.</p>
                )}
                {analysis.streams.map((s) => (
                  <StreamRow
                    key={s.streamIndex}
                    stream={s}
                    busy={busy}
                    onExtract={(idx) => onExtract(video.path, idx)}
                    onReview={(idx) => onReview(video.path, idx)}
                  />
                ))}
                {analysis.reviews.length > 0 && (
                  <div className="space-y-2 pt-1">
                    <div className="text-xs font-semibold text-ink-1">
                      استخراج‌های آزمایشی ({faNum(analysis.reviews.length)})
                    </div>
                    {analysis.reviews.map((r, i) => (
                      <ReviewCard
                        key={`${r.streamIndex}-${i}`}
                        review={r}
                        busy={busy}
                        onPromote={(idx) => onPromote(video.path, idx)}
                      />
                    ))}
                  </div>
                )}
                {analysis.lastExtraction && (
                  <div className="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs text-emerald-300">
                    <div dir="ltr" className="break-all font-mono">
                      {analysis.lastExtraction.targetPath}
                    </div>
                    <div className="mt-1">
                      {faNum(analysis.lastExtraction.validation.cueCount)} cue —{" "}
                      {analysis.lastExtraction.validation.encoding} —{" "}
                      {analysis.lastExtraction.persianConfirmed
                        ? "فارسی تأیید شد"
                        : "فارسی تأیید نشد"}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>

          <div className="border-t border-line pt-3">
            <dt className="mb-1 text-xs text-ink-3">
              زیرنویس‌های کناری ({faNum(video.siblingSubtitles.length)})
            </dt>
            <dd className="space-y-2">
              {video.siblingSubtitles.length === 0 && (
                <span className="text-xs text-ink-3">هیچ زیرنویس کناری کنار این ویدیو نیست.</span>
              )}
              {video.siblingSubtitles.map((s) => (
                <div key={s.path} className="rounded-lg border border-line bg-surface-2 px-3 py-2">
                  <div className="flex items-center justify-between gap-2">
                    <span
                      dir="ltr"
                      className="min-w-0 truncate font-mono text-xs text-ink-2"
                      title={s.fileName}
                    >
                      {s.fileName}
                    </span>
                    {s.nameSuggestsFa && (
                      <Badge tone="success">
                        <Languages className="size-3" />
                        فارسی
                      </Badge>
                    )}
                  </div>
                  <div className="mt-1 text-[11px] text-ink-3">
                    {s.ext.toUpperCase()} — {formatBytes(s.sizeBytes)}
                  </div>
                </div>
              ))}
            </dd>
          </div>
        </dl>
      </div>
    </div>
  );
}

export function Library() {
  const {
    library,
    refreshLibrary,
    scanning,
    opsRunning,
    opsProgress,
    opsSummary,
    error,
    setError,
    analyzeVideo,
    analyzeAll,
    extractTrack,
    extractTrackForReview,
    promoteReview,
    planExtractionAll,
    executeExtractionPlan,
    cancelOps,
  } = useAppData();
  const [query, setQuery] = useState("");
  const [seasonFilter, setSeasonFilter] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [plan, setPlan] = useState<ExtractionPlan | null>(null);
  const [planLoading, setPlanLoading] = useState(false);
  const [busyPaths, setBusyPaths] = useState<Set<string>>(new Set());

  // selected با به‌روزرسانی کتابخانه (رخداد library:entry) تازه می‌ماند
  const selectedEntry = selectedId
    ? (library.find((e) => e.id === selectedId) ?? null)
    : null;

  const seasons = useMemo(
    () =>
      [...new Set(library.map((v) => v.season).filter((s): s is number => s != null))].sort(
        (a, b) => a - b,
      ),
    [library],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return library.filter((v) => {
      if (q) {
        const haystack = `${v.fileName} ${v.showName ?? ""} ${v.path}`.toLowerCase();
        if (!haystack.includes(q)) return false;
      }
      if (seasonFilter === "none") {
        if (v.season != null) return false;
      } else if (seasonFilter !== "all" && v.season !== Number(seasonFilter)) {
        return false;
      }
      switch (statusFilter) {
        case "fa":
          return v.siblingSubtitles.some((s) => s.nameSuggestsFa);
        case "any":
          return v.siblingSubtitles.length > 0;
        case "none":
          return v.siblingSubtitles.length === 0;
        case "unparsed":
          return v.season == null && v.episode == null;
        case "analysis":
          return v.analysis != null;
        default:
          return true;
      }
    });
  }, [library, query, seasonFilter, statusFilter]);

  const withBusy = (path: string, fn: () => Promise<void>) => {
    setBusyPaths((prev) => new Set(prev).add(path));
    fn().finally(() =>
      setBusyPaths((prev) => {
        const next = new Set(prev);
        next.delete(path);
        return next;
      }),
    );
  };

  /** استخراج آزمایشی فقط با تأیید صریح کاربر */
  const confirmAndReview = async (path: string, streamIndex: number) => {
    const ok = await ask(
      `استخراج آزمایشی Track #${streamIndex} انجام شود؟\n\nخروجی در فایل جداگانهٔ .track-${streamIndex}.review.srt کنار ویدیو ساخته می‌شود و بعد از بررسی محتوای واقعی، در صورت تأیید فارسی بودن می‌توانید آن را به .fa.srt استاندارد تبدیل کنید.`,
      { title: "استخراج آزمایشی برای بررسی", kind: "info" },
    );
    if (!ok) return;
    await extractTrackForReview(path, streamIndex);
  };

  const openPlan = async () => {
    setPlanLoading(true);
    try {
      setPlan(await planExtractionAll());
    } finally {
      setPlanLoading(false);
    }
  };

  if (!scanning && library.length === 0) {
    return (
      <div className="mx-auto max-w-6xl p-6">
        <h1 className="mb-4 text-xl font-bold">کتابخانه</h1>
        <EmptyState
          icon={Film}
          title="کتابخانه خالی است"
          description="برای ساخت کتابخانه، از خانه پوشه یا فایل‌ها را انتخاب کنید؛ اسکن دستی هم در صفحهٔ پردازش (بخش پیشرفته) موجود است."
        />
      </div>
    );
  }

  return (
    <div className="mx-auto max-w-6xl space-y-4 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h1 className="text-xl font-bold">کتابخانه</h1>
        <span className="text-sm text-ink-3">
          {faNum(filtered.length)} از {faNum(library.length)} ویدیو
        </span>
      </div>

      {error && (
        <div className="flex items-center justify-between gap-3 rounded-lg border border-red-500/30 bg-red-500/10 px-4 py-3 text-sm text-red-300">
          <span className="min-w-0 break-all">{error}</span>
          <button onClick={() => setError(null)} className="shrink-0 hover:text-red-200">
            ✕
          </button>
        </div>
      )}

      {(opsRunning || opsSummary) && (
        <div className="space-y-2 rounded-lg border border-line bg-surface-1 px-4 py-3">
          <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
            <span className="flex items-center gap-2 text-ink-1">
              {opsRunning ? (
                <>
                  <Loader2 className="size-4 animate-spin text-accent" />
                  {opsProgress?.kind === "extract"
                    ? "استخراج گروهی…"
                    : opsProgress?.kind === "analyze"
                      ? "تحلیل گروهی…"
                      : "عملیات در جریان…"}
                  {opsProgress && (
                    <span className="text-xs text-ink-3">
                      {faNum(opsProgress.current)}/{faNum(opsProgress.total)}
                    </span>
                  )}
                </>
              ) : (
                <>
                  <Captions className="size-4 text-accent" />
                  {opsSummary}
                </>
              )}
            </span>
            {opsRunning && (
              <Button variant="danger" className="h-8 px-3 text-xs" onClick={cancelOps}>
                <Square className="size-3.5" />
                توقف
              </Button>
            )}
          </div>
          {opsProgress && (
            <div
              dir="ltr"
              className="truncate font-mono text-[11px] text-ink-3"
              title={opsProgress.videoPath}
            >
              {opsProgress.videoPath}
            </div>
          )}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-56 flex-1">
          <Search className="pointer-events-none absolute inset-y-0 start-3 my-auto size-4 text-ink-3" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="جستجو در نام فایل، سریال یا مسیر…"
            className="h-10 w-full rounded-lg border border-line bg-surface-1 ps-9 pe-3 text-sm outline-none placeholder:text-ink-3 focus:border-accent"
          />
        </div>
        <select
          value={seasonFilter}
          onChange={(e) => setSeasonFilter(e.target.value)}
          className="h-10 rounded-lg border border-line bg-surface-1 px-3 text-sm outline-none focus:border-accent"
        >
          <option value="all">همه فصل‌ها</option>
          <option value="none">بدون فصل</option>
          {seasons.map((s) => (
            <option key={s} value={String(s)}>
              فصل {s}
            </option>
          ))}
        </select>
        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value as StatusFilter)}
          className="h-10 rounded-lg border border-line bg-surface-1 px-3 text-sm outline-none focus:border-accent"
        >
          {STATUS_OPTIONS.map((o) => (
            <option key={o.id} value={o.id}>
              {o.label}
            </option>
          ))}
        </select>
        <Button onClick={refreshLibrary} loading={scanning} title="به‌روزرسانی فهرست">
          <RefreshCw className="size-4" />
        </Button>
        <Button
          onClick={() => analyzeAll()}
          disabled={opsRunning}
          title="تحلیل زیرنویس داخلی همه ویدیوها"
        >
          <ScanLine className="size-4" />
          تحلیل همه
        </Button>
        <Button
          variant="primary"
          onClick={openPlan}
          loading={planLoading}
          disabled={opsRunning}
          title="استخراج همه Trackهای متنی فارسیِ بدون ابهام — با تأیید"
        >
          <Sparkles className="size-4" />
          استخراج خودکار…
        </Button>
      </div>

      <div className="overflow-x-auto rounded-xl border border-line bg-surface-1">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-line text-xs text-ink-3">
              <th className="px-4 py-3 text-start font-medium">نام فایل</th>
              <th className="px-4 py-3 text-start font-medium">سریال</th>
              <th className="px-4 py-3 text-start font-medium">فصل</th>
              <th className="px-4 py-3 text-start font-medium">قسمت</th>
              <th className="px-4 py-3 text-start font-medium">زیرنویس داخلی</th>
              <th className="px-4 py-3 text-start font-medium">کناری</th>
              <th className="px-4 py-3 text-start font-medium">حجم</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((v) => {
              const analysis = v.analysis
                ? ANALYSIS_BADGES[v.analysis.status]
                : ANALYSIS_BADGES.notAnalyzed;
              return (
                <tr
                  key={v.id}
                  onClick={() => setSelectedId(v.id)}
                  className="cursor-pointer border-b border-line/60 last:border-0 hover:bg-surface-2"
                >
                  <td className="max-w-72 px-4 py-2.5">
                    <div
                      dir="ltr"
                      className="truncate text-start font-mono text-xs text-ink-1"
                      title={v.fileName}
                    >
                      {v.fileName}
                    </div>
                    <div
                      dir="ltr"
                      className="truncate text-start font-mono text-[10px] text-ink-3"
                      title={v.parentDir}
                    >
                      {v.parentDir}
                    </div>
                  </td>
                  <td className="px-4 py-2.5 text-ink-2">{v.showName ?? "—"}</td>
                  <td className="px-4 py-2.5 text-ink-2">
                    {v.season != null ? faNum(v.season) : "—"}
                  </td>
                  <td className="px-4 py-2.5 text-ink-2">
                    {v.episode != null ? faNum(v.episode) : "—"}
                  </td>
                  <td className="px-4 py-2.5">
                    <Badge tone={analysis.tone}>{analysis.label}</Badge>
                  </td>
                  <td className="px-4 py-2.5">
                    {v.siblingSubtitles.some((s) => s.nameSuggestsFa) ? (
                      <Badge tone="success">فارسی</Badge>
                    ) : v.siblingSubtitles.length > 0 ? (
                      <Badge tone="info">موجود</Badge>
                    ) : (
                      <span className="text-ink-3">—</span>
                    )}
                  </td>
                  <td className="px-4 py-2.5 text-ink-2">{formatBytes(v.sizeBytes)}</td>
                </tr>
              );
            })}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={7} className="px-4 py-10 text-center text-sm text-ink-3">
                  موردی با این فیلترها پیدا نشد.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {selectedEntry && (
        <DetailsDrawer
          video={selectedEntry}
          busy={busyPaths.has(selectedEntry.id) || opsRunning}
          onClose={() => setSelectedId(null)}
          onAnalyze={(path) => withBusy(path, () => analyzeVideo(path))}
          onExtract={(path, idx) => withBusy(path, () => extractTrack(path, idx))}
          onReview={(path, idx) => withBusy(path, () => confirmAndReview(path, idx))}
          onPromote={(path, idx) => withBusy(path, () => promoteReview(path, idx))}
        />
      )}

      <Modal
        open={plan !== null}
        title="استخراج خودکار — خلاصه عملیات"
        onClose={() => setPlan(null)}
        wide
        footer={
          <>
            <Button onClick={() => setPlan(null)}>انصراف</Button>
            <Button
              variant="primary"
              disabled={!plan || plan.items.length === 0 || opsRunning}
              onClick={() => {
                if (plan && plan.items.length > 0) {
                  const items = plan.items;
                  setPlan(null);
                  void executeExtractionPlan(items);
                }
              }}
            >
              <FileVideo className="size-4" />
              شروع استخراج ({plan ? faNum(plan.items.length) : "۰"} مورد)
            </Button>
          </>
        }
      >
        {plan && (
          <div className="space-y-4 text-sm">
            <p className="text-xs leading-5 text-ink-3">
              فقط ویدیوهایی با «دقیقاً یک Track متنی فارسی» و بدون فایل مقصد موجود، استخراج
              می‌شوند. مقصد هر مورد{" "}
              <span dir="ltr" className="font-mono">
                &lt;base&gt;.fa.srt
              </span>{" "}
              است و هیچ فایل موجودی بازنویسی نمی‌شود.
            </p>
            <div>
              <div className="mb-2 text-xs font-semibold text-ink-1">
                آماده استخراج ({faNum(plan.items.length)})
              </div>
              {plan.items.length === 0 ? (
                <p className="text-xs text-ink-3">موردی برای استخراج خودکار نیست.</p>
              ) : (
                <div className="max-h-56 space-y-1.5 overflow-y-auto">
                  {plan.items.map((it) => (
                    <div
                      key={`${it.videoPath}#${it.streamIndex}`}
                      className="rounded-lg border border-line bg-surface-2 px-3 py-2"
                    >
                      <div
                        dir="ltr"
                        className="truncate font-mono text-xs text-ink-1"
                        title={it.videoFileName}
                      >
                        {it.videoFileName}
                      </div>
                      <div className="mt-1 flex flex-wrap items-center gap-2 text-[11px] text-ink-3">
                        <span dir="ltr" className="font-mono">
                          {it.trackLabel}
                        </span>
                        <span>→</span>
                        <span dir="ltr" className="truncate font-mono" title={it.targetPath}>
                          {it.targetPath.split(/[\\/]/).pop()}
                        </span>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
            {plan.skipped.length > 0 && (
              <div>
                <div className="mb-2 text-xs font-semibold text-ink-1">
                  ردشده‌ها ({faNum(plan.skipped.length)})
                </div>
                <div className="max-h-56 space-y-1.5 overflow-y-auto">
                  {plan.skipped.map((s) => (
                    <div key={s.videoPath} className="rounded-lg border border-line bg-surface-2 px-3 py-2">
                      <div
                        dir="ltr"
                        className="truncate font-mono text-xs text-ink-2"
                        title={s.videoFileName}
                      >
                        {s.videoFileName}
                      </div>
                      <div className="mt-0.5 text-[11px] text-ink-3">{s.reason}</div>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
        )}
      </Modal>
    </div>
  );
}
