import { useMemo, useState } from "react";
import { Film, Languages, RefreshCw, Search, X } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { Badge, Button, EmptyState } from "../components/ui";
import { faNum, formatBytes, formatTimestamp } from "../lib/format";
import type { VideoEntry } from "../types";

type StatusFilter = "all" | "fa" | "any" | "none" | "unparsed";

const STATUS_OPTIONS: { id: StatusFilter; label: string }[] = [
  { id: "all", label: "همه" },
  { id: "fa", label: "زیرنویس فارسی کناری" },
  { id: "any", label: "زیرنویس کناری (هر نوعی)" },
  { id: "none", label: "بدون زیرنویس کناری" },
  { id: "unparsed", label: "بدون شناسه S/E" },
];

function statusBadge(v: VideoEntry) {
  if (v.siblingSubtitles.some((s) => s.nameSuggestsFa))
    return { tone: "success" as const, label: "زیرنویس فارسی" };
  if (v.siblingSubtitles.length > 0) return { tone: "info" as const, label: "زیرنویس کناری" };
  return { tone: "warn" as const, label: "بدون زیرنویس کناری" };
}

function DetailsDrawer({
  video,
  onClose,
}: {
  video: VideoEntry;
  onClose: () => void;
}) {
  return (
    <div className="fixed inset-0 z-40 bg-black/60" onClick={onClose}>
      <div
        dir="rtl"
        className="absolute inset-y-0 start-0 w-[430px] overflow-y-auto border-e border-line bg-surface-1 p-5"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-start justify-between gap-3">
          <h3 className="break-all text-sm font-bold text-ink-1">{video.fileName}</h3>
          <button onClick={onClose} className="rounded-md p-1 text-ink-3 hover:bg-surface-2 hover:text-ink-1">
            <X className="size-4" />
          </button>
        </div>

        <dl className="space-y-3 text-sm">
          <div>
            <dt className="text-xs text-ink-3">نام سریال</dt>
            <dd className="mt-0.5">{video.showName ?? "—"}</dd>
          </div>
          <div className="flex gap-6">
            <div>
              <dt className="text-xs text-ink-3">فصل</dt>
              <dd className="mt-0.5">{video.season != null ? faNum(video.season) : "—"}</dd>
            </div>
            <div>
              <dt className="text-xs text-ink-3">قسمت</dt>
              <dd className="mt-0.5">{video.episode != null ? faNum(video.episode) : "—"}</dd>
            </div>
            <div>
              <dt className="text-xs text-ink-3">حجم</dt>
              <dd className="mt-0.5">{formatBytes(video.sizeBytes)}</dd>
            </div>
          </div>
          <div>
            <dt className="text-xs text-ink-3">تاریخ تغییر</dt>
            <dd className="mt-0.5">{formatTimestamp(video.modifiedMs)}</dd>
          </div>
          <div>
            <dt className="text-xs text-ink-3">مسیر کامل</dt>
            <dd dir="ltr" className="mt-1 break-all rounded-lg bg-surface-2 p-2 font-mono text-xs text-ink-2">
              {video.path}
            </dd>
          </div>
          <div>
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
                    <span dir="ltr" className="min-w-0 truncate font-mono text-xs text-ink-2">
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
  const { library, refreshLibrary, scanning } = useAppData();
  const [query, setQuery] = useState("");
  const [seasonFilter, setSeasonFilter] = useState<string>("all");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("all");
  const [selected, setSelected] = useState<VideoEntry | null>(null);

  const seasons = useMemo(
    () => [...new Set(library.map((v) => v.season).filter((s): s is number => s != null))].sort((a, b) => a - b),
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
        default:
          return true;
      }
    });
  }, [library, query, seasonFilter, statusFilter]);

  if (!scanning && library.length === 0) {
    return (
      <div className="mx-auto max-w-6xl p-6">
        <h1 className="mb-4 text-xl font-bold">کتابخانه</h1>
        <EmptyState
          icon={Film}
          title="کتابخانه خالی است"
          description="اول از داشبورد یک پوشه را اسکن کنید تا ویدیوها اینجا فهرست شوند."
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
      </div>

      <div className="overflow-x-auto rounded-xl border border-line bg-surface-1">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-line text-xs text-ink-3">
              <th className="px-4 py-3 text-start font-medium">نام فایل</th>
              <th className="px-4 py-3 text-start font-medium">سریال</th>
              <th className="px-4 py-3 text-start font-medium">فصل</th>
              <th className="px-4 py-3 text-start font-medium">قسمت</th>
              <th className="px-4 py-3 text-start font-medium">وضعیت</th>
              <th className="px-4 py-3 text-start font-medium">حجم</th>
            </tr>
          </thead>
          <tbody>
            {filtered.map((v) => {
              const badge = statusBadge(v);
              return (
                <tr
                  key={v.id}
                  onClick={() => setSelected(v)}
                  className="cursor-pointer border-b border-line/60 last:border-0 hover:bg-surface-2"
                >
                  <td className="max-w-72 px-4 py-2.5">
                    <div dir="ltr" className="truncate text-start font-mono text-xs text-ink-1" title={v.fileName}>
                      {v.fileName}
                    </div>
                    <div dir="ltr" className="truncate text-start font-mono text-[10px] text-ink-3" title={v.parentDir}>
                      {v.parentDir}
                    </div>
                  </td>
                  <td className="px-4 py-2.5 text-ink-2">{v.showName ?? "—"}</td>
                  <td className="px-4 py-2.5 text-ink-2">{v.season != null ? faNum(v.season) : "—"}</td>
                  <td className="px-4 py-2.5 text-ink-2">{v.episode != null ? faNum(v.episode) : "—"}</td>
                  <td className="px-4 py-2.5">
                    <div className="flex flex-wrap gap-1">
                      <Badge tone={badge.tone}>{badge.label}</Badge>
                      {v.season == null && v.episode == null && (
                        <Badge tone="neutral">بدون S/E</Badge>
                      )}
                    </div>
                  </td>
                  <td className="px-4 py-2.5 text-ink-2">{formatBytes(v.sizeBytes)}</td>
                </tr>
              );
            })}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={6} className="px-4 py-10 text-center text-sm text-ink-3">
                  موردی با این فیلترها پیدا نشد.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {selected && <DetailsDrawer video={selected} onClose={() => setSelected(null)} />}
    </div>
  );
}
