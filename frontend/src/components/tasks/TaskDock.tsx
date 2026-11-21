import {
  Captions,
  CheckCircle2,
  Film,
  Loader2,
  ListTree,
} from "lucide-react";
import { usePipeline } from "../../hooks/usePipeline";
import { useHardsub } from "../../hooks/useHardsub";
import { useTaskFocus } from "../../hooks/useTaskFocus";
import { faNum } from "../../lib/format";
import type { PageId } from "../../types";

interface DockItem {
  key: string;
  icon: React.ReactNode;
  title: string;
  detail: string;
  running: boolean;
  onClick: () => void;
}

/**
 * جعبهٔ شناور تسک‌ها — همیشه در کنار برنامه؛ وقتی کاربر نمای یک تسک را با
 * «خانه» ترک می‌کند، وضعیت همهٔ تسک‌های فعال اینجا دیده می‌شود و با کلیک،
 * نمای همان تسک دوباره باز می‌شود.
 */
export function TaskDock({
  onNavigate,
  page,
}: {
  onNavigate: (page: PageId) => void;
  page: PageId;
}) {
  const { runActive, snapshot, summary } = usePipeline();
  const hardsub = useHardsub();
  const { focus, setFocus, softsubPaths, softsubTask, openSoftsub } = useTaskFocus();

  /** آیا کاربر همین الان نمای این تسک را باز دارد؟ — آنگاه از dock حذف می‌شود */
  const viewingPipelineView = page === "home" && focus === "pipeline";
  const viewingHardsubView = page === "home" && focus === "hardsub";

  const items: DockItem[] = [];

  // ---- تسک سینک (pipeline) ----
  if (!viewingPipelineView && runActive && snapshot) {
    const percent =
      snapshot.total > 0
        ? Math.min(100, Math.round((snapshot.completed / snapshot.total) * 100))
        : 0;
    items.push({
      key: "pipeline",
      icon: <ListTree className="size-4 shrink-0 text-emerald-400" />,
      title: "سینک در جریان است",
      detail: `${faNum(snapshot.completed)} از ${faNum(snapshot.total)} — ${faNum(percent)}٪`,
      running: true,
      onClick: () => {
        setFocus("pipeline");
        onNavigate("home");
      },
    });
  } else if (summary && !viewingPipelineView) {
    items.push({
      key: "pipeline-summary",
      icon: <CheckCircle2 className="size-4 shrink-0 text-emerald-400" />,
      title: "نتیجهٔ سینک",
      detail: "باز کردن خلاصه",
      running: false,
      onClick: () => {
        setFocus("pipeline");
        onNavigate("home");
      },
    });
  }

  // ---- تسک هاردساب ----
  if (!viewingHardsubView && hardsub.runActive && hardsub.snapshot) {
    const percent =
      hardsub.snapshot.total > 0
        ? Math.min(
            100,
            Math.round((hardsub.snapshot.completed / hardsub.snapshot.total) * 100),
          )
        : 0;
    items.push({
      key: "hardsub",
      icon: <Film className="size-4 shrink-0 text-sky-400" />,
      title: "هاردساب در جریان است",
      detail: `${faNum(hardsub.snapshot.completed)} از ${faNum(hardsub.snapshot.total)} — ${faNum(percent)}٪`,
      running: true,
      onClick: () => {
        setFocus("hardsub");
        onNavigate("home");
      },
    });
  } else if (hardsub.summary && !viewingHardsubView) {
    items.push({
      key: "hardsub-summary",
      icon: <CheckCircle2 className="size-4 shrink-0 text-sky-400" />,
      title: "نتیجهٔ هاردساب",
      detail: "باز کردن خلاصه",
      running: false,
      onClick: () => {
        setFocus("hardsub");
        onNavigate("home");
      },
    });
  }

  // ---- تسک افزودن زیرنویس چسبیده ----
  if (softsubTask.active && softsubPaths === null) {
    items.push({
      key: "softsub",
      icon: <Captions className="size-4 shrink-0 text-amber-400" />,
      title: "افزودن زیرنویس چسبیده",
      detail:
        softsubTask.total > 0
          ? `${faNum(softsubTask.current)} از ${faNum(softsubTask.total)}`
          : "در جریان…",
      running: true,
      onClick: () => {
        if (softsubTask.lastPaths.length > 0) {
          openSoftsub(softsubTask.lastPaths);
        } else {
          setFocus("pipeline");
          onNavigate("home");
        }
      },
    });
  }

  if (items.length === 0) return null;

  return (
    <div
      className="fixed bottom-4 start-4 z-40 w-72 space-y-1.5"
      role="region"
      aria-label="تسک‌های فعال"
    >
      <div className="rounded-xl border border-line bg-surface-1/95 p-2 shadow-2xl backdrop-blur">
        <div className="px-2 pb-1 pt-0.5 text-[11px] font-semibold text-ink-3">
          تسک‌ها ({faNum(items.length)})
        </div>
        {items.map((item) => (
          <button
            key={item.key}
            onClick={item.onClick}
            className="flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-start transition-colors hover:bg-surface-2"
          >
            {item.icon}
            <span className="min-w-0 flex-1">
              <span className="flex items-center gap-1.5">
                <span className="truncate text-xs font-semibold text-ink-1">
                  {item.title}
                </span>
                {item.running && (
                  <Loader2 className="size-3 shrink-0 animate-spin text-ink-3" />
                )}
                {!item.running && (
                  <span className="shrink-0 text-[10px] text-accent">
                    پایان‌یافته
                  </span>
                )}
              </span>
              <span className="block truncate text-[11px] text-ink-3">
                {item.detail}
              </span>
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
