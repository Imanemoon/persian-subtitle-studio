import type { LucideIcon } from "lucide-react";
import { Clapperboard, Cpu, Film, LayoutDashboard, ScrollText, Settings } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import type { PageId } from "../types";

const ITEMS: { id: PageId; label: string; icon: LucideIcon }[] = [
  { id: "dashboard", label: "داشبورد", icon: LayoutDashboard },
  { id: "library", label: "کتابخانه", icon: Film },
  { id: "processing", label: "پردازش", icon: Cpu },
  { id: "logs", label: "لاگ‌ها", icon: ScrollText },
  { id: "settings", label: "تنظیمات", icon: Settings },
];

export function Sidebar({
  page,
  onNavigate,
}: {
  page: PageId;
  onNavigate: (page: PageId) => void;
}) {
  const { scanning } = useAppData();

  return (
    <aside className="flex h-full w-60 shrink-0 flex-col border-e border-line bg-surface-1">
      <div className="flex items-center gap-3 px-5 py-5">
        <div className="flex size-9 items-center justify-center rounded-lg bg-accent text-zinc-950">
          <Clapperboard className="size-5" />
        </div>
        <div className="min-w-0">
          <div className="text-sm font-bold text-ink-1">SubSync Studio</div>
          <div className="text-[11px] text-ink-3">مدیریت زیرنویس فارسی</div>
        </div>
      </div>

      <nav className="mt-2 flex-1 space-y-1 px-3">
        {ITEMS.map(({ id, label, icon: Icon }) => {
          const active = page === id;
          return (
            <button
              key={id}
              onClick={() => onNavigate(id)}
              className={`flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-sm transition-colors ${
                active
                  ? "bg-accent-dim font-semibold text-accent-strong"
                  : "text-ink-2 hover:bg-surface-2 hover:text-ink-1"
              }`}
            >
              <Icon className="size-4.5" />
              <span className="flex-1 text-start">{label}</span>
              {id === "processing" && scanning && (
                <span className="size-2 animate-pulse rounded-full bg-accent" />
              )}
            </button>
          );
        })}
      </nav>

      <div className="border-t border-line px-5 py-4 text-[11px] text-ink-3">
        نسخه ۰٫۱٫۰ — فاز ۱ (اسکن و شناسایی)
      </div>
    </aside>
  );
}
