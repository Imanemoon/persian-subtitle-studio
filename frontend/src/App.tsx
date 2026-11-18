import { useEffect, useRef, useState } from "react";
import { AppDataProvider, useAppData } from "./hooks/useAppData";
import { PipelineProvider } from "./hooks/usePipeline";
import { Header } from "./components/Header";
import { Home } from "./pages/Home";
import { Library } from "./pages/Library";
import { Processing } from "./pages/Processing";
import { Logs } from "./pages/Logs";
import { SettingsPage } from "./pages/SettingsPage";
import { SetupPage } from "./pages/SetupPage";
import { api } from "./lib/api";
import type { PageId } from "./types";

export default function App() {
  return (
    <AppDataProvider>
      <PipelineProvider>
        <Shell />
      </PipelineProvider>
    </AppDataProvider>
  );
}

/**
 * پوستهٔ برنامه — مسیریابی.
 * معاونت راه‌اندازی فقط یک‌بار: اگر setupCompleted نباشد و ابزارهای حیاتی
 * (ffmpeg/ffprobe) نباشند، صفحهٔ setup پیش از هر چیز می‌آید. تصمیم فقط با
 * دادهٔ واقعی بک‌اند گرفته می‌شود و یک‌بار است — تغییر ابزارها در میانهٔ
 * کارِ کاربر، مسیر را عوض نمی‌کند.
 */
function Shell() {
  const [page, setPage] = useState<PageId>("home");
  const { settings, tools, toolsLoading, reloadSettings } = useAppData();
  const decidedRef = useRef(false);

  useEffect(() => {
    if (decidedRef.current || toolsLoading) return;
    decidedRef.current = true;
    const setupDone = settings?.setupCompleted ?? false;
    const coreMissing = tools
      .filter((t) => t.id === "ffmpeg" || t.id === "ffprobe")
      .some((t) => t.status !== "found");
    // preview فقط-dev برای دیدن معاونت روی ماشین‌های کامل (در بیلد نهایی حذف می‌شود)
    const forceSetup =
      import.meta.env.DEV && new URLSearchParams(window.location.search).has("setup");
    if (forceSetup || (!setupDone && coreMissing)) setPage("setup");
  }, [tools, toolsLoading, settings]);

  const enterApp = async () => {
    try {
      const current = await api.getSettings();
      await api.saveSettings({ ...current, setupCompleted: true });
      await reloadSettings();
    } catch {
      // حتی اگر ذخیرهٔ پرچم ناموفق بود، کاربر آزاد است وارد شود
    }
    setPage("home");
  };

  return (
    <div dir="rtl" className="flex h-screen flex-col bg-surface-0 text-ink-1">
      <Header page={page} onNavigate={setPage} />
      <main className="min-h-0 flex-1 overflow-y-auto">
        {page === "setup" && <SetupPage onEnter={() => void enterApp()} />}
        {page === "home" && <Home onNavigate={setPage} />}
        {page === "library" && <Library />}
        {page === "processing" && <Processing onNavigate={setPage} />}
        {page === "logs" && <Logs />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
