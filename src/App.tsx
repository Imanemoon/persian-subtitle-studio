import { useState } from "react";
import { AppDataProvider } from "./hooks/useAppData";
import { Header } from "./components/Header";
import { Home } from "./pages/Home";
import { Library } from "./pages/Library";
import { Processing } from "./pages/Processing";
import { Logs } from "./pages/Logs";
import { SettingsPage } from "./pages/SettingsPage";
import type { PageId } from "./types";

export default function App() {
  const [page, setPage] = useState<PageId>("home");

  return (
    <AppDataProvider>
      <div dir="rtl" className="flex h-screen flex-col bg-surface-0 text-ink-1">
        <Header page={page} onNavigate={setPage} />
        <main className="min-h-0 flex-1 overflow-y-auto">
          {page === "home" && <Home onNavigate={setPage} />}
          {page === "library" && <Library />}
          {page === "processing" && <Processing />}
          {page === "logs" && <Logs />}
          {page === "settings" && <SettingsPage />}
        </main>
      </div>
    </AppDataProvider>
  );
}
