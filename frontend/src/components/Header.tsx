import { Clapperboard, Settings } from "lucide-react";
import type { PageId } from "../types";

/**
 * هدر مینیمال — هدف: کمترین شلوغی. لوگو (کلیک = خانه) + آیکن تنظیمات.
 * ناوبری تسک‌ها از جعبهٔ شناور تسک‌ها انجام می‌شود نه از هدر.
 */
export function Header({
  onNavigate,
}: {
  onNavigate: (page: PageId) => void;
}) {
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

      <button
        title="تنظیمات"
        onClick={() => onNavigate("settings")}
        className="flex size-9 items-center justify-center rounded-lg text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink-1"
      >
        <Settings className="size-4.5" />
      </button>
    </header>
  );
}
