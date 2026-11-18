import { useEffect, useMemo, useRef, useState } from "react";
import { ScrollText } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { Button, EmptyState } from "../components/ui";
import type { LogLevel } from "../types";

const LEVEL_STYLES: Record<LogLevel, string> = {
  debug: "text-ink-3",
  info: "text-sky-400",
  warn: "text-amber-400",
  error: "text-red-400",
};

const LEVEL_LABELS: Record<LogLevel, string> = {
  debug: "دیباگ",
  info: "اطلاع",
  warn: "هشدار",
  error: "خطا",
};

export function Logs() {
  const { logs, refreshLogs } = useAppData();
  const [filter, setFilter] = useState<"all" | LogLevel>("all");
  const [autoScroll, setAutoScroll] = useState(true);
  const containerRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(
    () => (filter === "all" ? logs : logs.filter((l) => l.level === filter)),
    [logs, filter],
  );

  useEffect(() => {
    if (autoScroll && containerRef.current) {
      containerRef.current.scrollTop = containerRef.current.scrollHeight;
    }
  }, [filtered.length, autoScroll]);

  return (
    <div className="mx-auto flex h-full max-w-6xl flex-col gap-4 p-6">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-xl font-bold">لاگ‌ها</h1>
          <p className="mt-1 text-sm text-ink-3">رخدادهای برنامه به‌صورت زنده</p>
        </div>
        <div className="flex gap-2">
          <Button onClick={refreshLogs}>به‌روزرسانی</Button>
          <Button onClick={() => setAutoScroll((v) => !v)}>
            {autoScroll ? "توقف اسکرول خودکار" : "اسکرول خودکار"}
          </Button>
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        {(["all", "info", "warn", "error", "debug"] as const).map((f) => (
          <button
            key={f}
            onClick={() => setFilter(f)}
            className={`h-8 rounded-lg px-3 text-xs transition-colors ${
              filter === f
                ? "bg-accent-dim font-semibold text-accent-strong"
                : "border border-line bg-surface-2 text-ink-2 hover:bg-zinc-800"
            }`}
          >
            {f === "all" ? "همه" : LEVEL_LABELS[f]}
          </button>
        ))}
        <span className="ms-auto self-center text-xs text-ink-3">{filtered.length} رخداد</span>
      </div>

      {filtered.length === 0 ? (
        <EmptyState icon={ScrollText} title="لاگی ثبت نشده" description="رخدادهای برنامه پس از اجرا در همین صفحه نمایش داده می‌شوند. لاگ متنی هم در پوشه logs برنامه ذخیره می‌شود." />
      ) : (
        <div
          ref={containerRef}
          className="min-h-0 flex-1 overflow-y-auto rounded-xl border border-line bg-surface-1"
        >
          <div className="divide-y divide-line/60 font-mono text-xs">
            {filtered.map((l) => (
              <div key={l.id} className="flex items-start gap-3 px-4 py-2">
                <span dir="ltr" className="shrink-0 pt-0.5 text-ink-3">
                  {l.timestamp}
                </span>
                <span className={`w-12 shrink-0 pt-0.5 ${LEVEL_STYLES[l.level]}`}>
                  {LEVEL_LABELS[l.level]}
                </span>
                <span className="w-16 shrink-0 pt-0.5 text-ink-3">[{l.source}]</span>
                <span dir="auto" className="min-w-0 flex-1 break-words text-ink-2">
                  {l.message}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
