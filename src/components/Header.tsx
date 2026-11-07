import { useState } from "react";
import type { LucideIcon } from "lucide-react";
import {
  Clapperboard,
  Cpu,
  Film,
  House,
  ScrollText,
  Settings,
} from "lucide-react";
import type { PageId } from "../types";

const ADVANCED_ITEMS: { id: PageId; label: string; icon: LucideIcon }[] = [
  { id: "library", label: "کتابخانه", icon: Film },
  { id: "processing", label: "پردازش", icon: Cpu },
  { id: "logs", label: "لاگ‌ها", icon: ScrollText },
  { id: "settings", label: "تنظیمات", icon: Settings },
];

function IconButton({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      title={title}
      onClick={onClick}
      className="flex size-9 items-center justify-center rounded-lg text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink-1"
    >
      {children}
    </button>
  );
}

/**
 * هدر مینیمال: لوگو (کلیک = خانه) + در صفحهٔ خانه فقط آیکن تنظیمات و یک
 * ورودی جمع‌وجور به بخش پیشرفته؛ در صفحات پیشرفته، نوار ناوبری فشرده.
 */
export function Header({
  page,
  onNavigate,
}: {
  page: PageId;
  onNavigate: (page: PageId) => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const onHome = page === "home";

  return (
    <header className="relative z-30 flex h-14 shrink-0 items-center justify-between border-b border-line bg-surface-1 px-4">
      <button
        onClick={() => onNavigate("home")}
        className="flex items-center gap-2.5 rounded-lg px-1.5 py-1 transition-colors hover:bg-surface-2"
        title="خانه"
      >
        <div className="flex size-8 items-center justify-center rounded-lg bg-accent text-zinc-950">
          <Clapperboard className="size-4.5" />
        </div>
        <span className="text-sm font-bold text-ink-1">SubSync Studio</span>
      </button>

      {onHome ? (
        <div className="flex items-center gap-1.5">
          <IconButton title="تنظیمات" onClick={() => onNavigate("settings")}>
            <Settings className="size-4.5" />
          </IconButton>
          <div className="relative">
            <button
              onClick={() => setMenuOpen((v) => !v)}
              className={`h-9 rounded-lg px-3 text-xs font-medium transition-colors ${
                menuOpen
                  ? "bg-accent-dim text-accent-strong"
                  : "text-ink-2 hover:bg-surface-2 hover:text-ink-1"
              }`}
            >
              پیشرفته
            </button>
            {menuOpen && (
              <>
                <div
                  className="fixed inset-0 z-30"
                  onClick={() => setMenuOpen(false)}
                />
                <div className="absolute end-0 top-full z-40 mt-1.5 w-44 rounded-xl border border-line bg-surface-1 p-1.5 shadow-xl">
                  {ADVANCED_ITEMS.map(({ id, label, icon: Icon }) => (
                    <button
                      key={id}
                      onClick={() => {
                        setMenuOpen(false);
                        onNavigate(id);
                      }}
                      className="flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-sm text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink-1"
                    >
                      <Icon className="size-4" />
                      {label}
                    </button>
                  ))}
                </div>
              </>
            )}
          </div>
        </div>
      ) : (
        <nav className="flex items-center gap-1">
          {ADVANCED_ITEMS.map(({ id, label, icon: Icon }) => {
            const active = page === id;
            return (
              <button
                key={id}
                onClick={() => onNavigate(id)}
                className={`flex h-9 items-center gap-2 rounded-lg px-3 text-xs font-medium transition-colors ${
                  active
                    ? "bg-accent-dim text-accent-strong"
                    : "text-ink-2 hover:bg-surface-2 hover:text-ink-1"
                }`}
              >
                <Icon className="size-4" />
                {label}
              </button>
            );
          })}
          <IconButton title="خانه" onClick={() => onNavigate("home")}>
            <House className="size-4.5" />
          </IconButton>
        </nav>
      )}
    </header>
  );
}
