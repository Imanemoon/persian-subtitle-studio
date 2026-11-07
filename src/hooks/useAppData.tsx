import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type {
  LogEntry,
  ScanProgress,
  ScanSummary,
  Settings,
  ToolInfo,
  VideoEntry,
} from "../types";

interface AppDataState {
  library: VideoEntry[];
  lastScan: ScanSummary | null;
  progress: ScanProgress | null;
  scanning: boolean;
  tools: ToolInfo[];
  toolsLoading: boolean;
  settings: Settings | null;
  logs: LogEntry[];
  error: string | null;
  setError: (message: string | null) => void;
  refreshLibrary: () => Promise<void>;
  refreshTools: () => Promise<void>;
  refreshLogs: () => Promise<void>;
  reloadSettings: () => Promise<void>;
  startScan: (root: string) => Promise<void>;
  stopScan: () => Promise<void>;
}

const AppDataContext = createContext<AppDataState | null>(null);

const LOG_CAP = 1000;

export function AppDataProvider({ children }: { children: ReactNode }) {
  const [library, setLibrary] = useState<VideoEntry[]>([]);
  const [lastScan, setLastScan] = useState<ScanSummary | null>(null);
  const [progress, setProgress] = useState<ScanProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [tools, setTools] = useState<ToolInfo[]>([]);
  const [toolsLoading, setToolsLoading] = useState(true);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refreshLibrary = useCallback(async () => {
    setLibrary(await api.getLibrary());
  }, []);

  const refreshTools = useCallback(async () => {
    setToolsLoading(true);
    try {
      setTools(await api.detectTools());
    } finally {
      setToolsLoading(false);
    }
  }, []);

  const refreshLogs = useCallback(async () => {
    setLogs(await api.getLogs(500));
  }, []);

  const reloadSettings = useCallback(async () => {
    setSettings(await api.getSettings());
  }, []);

  const startScan = useCallback(async (root: string) => {
    setError(null);
    setScanning(true);
    setProgress(null);
    try {
      await api.startScan(root);
    } catch (e) {
      setScanning(false);
      setError(String(e));
      throw e;
    }
  }, []);

  const stopScan = useCallback(async () => {
    try {
      await api.stopScan();
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    (async () => {
      try {
        const [settingsResult, libraryResult, lastScanResult, scanningResult, toolsResult, logsResult] =
          await Promise.all([
            api.getSettings(),
            api.getLibrary(),
            api.getLastScan(),
            api.isScanning(),
            api.detectTools(),
            api.getLogs(500),
          ]);
        if (disposed) return;
        setSettings(settingsResult);
        setLibrary(libraryResult);
        setLastScan(lastScanResult);
        setScanning(scanningResult);
        setTools(toolsResult);
        setToolsLoading(false);
        setLogs(logsResult);
      } catch (e) {
        if (!disposed) {
          setError(`خطا در بارگذاری اولیه: ${String(e)}`);
          setToolsLoading(false);
        }
      }
    })();

    const register = async () => {
      unlisteners.push(
        await listen<ScanProgress>("scan:progress", (event) => {
          setProgress(event.payload);
        }),
      );
      unlisteners.push(
        await listen<ScanSummary>("scan:done", (event) => {
          setScanning(false);
          setProgress(null);
          setLastScan(event.payload);
          void api.getLibrary().then(setLibrary);
        }),
      );
      unlisteners.push(
        await listen<LogEntry>("log:new", (event) => {
          setLogs((prev) => {
            const next = [...prev, event.payload];
            return next.length > LOG_CAP ? next.slice(next.length - LOG_CAP) : next;
          });
        }),
      );
    };
    void register();

    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, []);

  const value: AppDataState = {
    library,
    lastScan,
    progress,
    scanning,
    tools,
    toolsLoading,
    settings,
    logs,
    error,
    setError,
    refreshLibrary,
    refreshTools,
    refreshLogs,
    reloadSettings,
    startScan,
    stopScan,
  };

  return <AppDataContext.Provider value={value}>{children}</AppDataContext.Provider>;
}

export function useAppData(): AppDataState {
  const ctx = useContext(AppDataContext);
  if (!ctx) throw new Error("useAppData must be used within AppDataProvider");
  return ctx;
}
