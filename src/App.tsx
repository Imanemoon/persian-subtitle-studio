import { useState } from "react";
import { AppDataProvider } from "./hooks/useAppData";
import { Sidebar } from "./components/Sidebar";
import { Dashboard } from "./pages/Dashboard";
import { Library } from "./pages/Library";
import { Processing } from "./pages/Processing";
import { Logs } from "./pages/Logs";
import { SettingsPage } from "./pages/SettingsPage";
import type { PageId } from "./types";

export default function App() {
  const [page, setPage] = useState<PageId>("dashboard");

  return (
    <AppDataProvider>
      <div dir="rtl" className="flex h-screen bg-surface-0 text-ink-1">
        <Sidebar page={page} onNavigate={setPage} />
        <main className="h-full min-w-0 flex-1 overflow-y-auto">
          {page === "dashboard" && <Dashboard onNavigate={setPage} />}
          {page === "library" && <Library />}
          {page === "processing" && <Processing />}
          {page === "logs" && <Logs />}
          {page === "settings" && <SettingsPage />}
        </main>
      </div>
    </AppDataProvider>
  );
}
