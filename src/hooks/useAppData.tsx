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
  ExtractionPlan,
  ExtractionPlanItem,
  LogEntry,
  OpsProgress,
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
  opsRunning: boolean;
  opsProgress: OpsProgress | null;
  opsSummary: string | null;
  setError: (message: string | null) => void;
  applyEntry: (entry: VideoEntry) => void;
  refreshLibrary: () => Promise<void>;
  refreshTools: () => Promise<void>;
  refreshLogs: () => Promise<void>;
  reloadSettings: () => Promise<void>;
  startScan: (root: string) => Promise<void>;
  stopScan: () => Promise<void>;
  analyzeVideo: (path: string) => Promise<void>;
  analyzeAll: () => Promise<void>;
  extractTrack: (path: string, streamIndex: number) => Promise<void>;
  planExtractionAll: () => Promise<ExtractionPlan | null>;
  executeExtractionPlan: (items: ExtractionPlanItem[]) => Promise<void>;
  cancelOps: () => Promise<void>;
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
  const [opsRunning, setOpsRunning] = useState(false);
  const [opsProgress, setOpsProgress] = useState<OpsProgress | null>(null);
  const [opsSummary, setOpsSummary] = useState<string | null>(null);

  const applyEntry = useCallback((entry: VideoEntry) => {
    setLibrary((prev) => {
      const index = prev.findIndex((e) => e.id === entry.id);
      if (index === -1) return prev;
      const next = [...prev];
      next[index] = entry;
      return next;
    });
  }, []);

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

  const analyzeVideo = useCallback(
    async (path: string) => {
      setError(null);
      try {
        const entry = await api.analyzeVideo(path);
        applyEntry(entry);
      } catch (e) {
        setError(String(e));
      }
    },
    [applyEntry],
  );

  const analyzeAll = useCallback(async () => {
    setError(null);
    setOpsSummary(null);
    setOpsRunning(true);
    setOpsProgress(null);
    try {
      await api.analyzeAll();
    } catch (e) {
      setOpsRunning(false);
      setError(String(e));
    }
  }, []);

  const extractTrack = useCallback(async (path: string, streamIndex: number) => {
    setError(null);
    try {
      await api.extractTrack(path, streamIndex);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const planExtractionAll = useCallback(async (): Promise<ExtractionPlan | null> => {
    setError(null);
    try {
      return await api.planExtractionAll();
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, []);

  const executeExtractionPlan = useCallback(async (items: ExtractionPlanItem[]) => {
    setError(null);
    setOpsSummary(null);
    setOpsRunning(true);
    setOpsProgress(null);
    try {
      await api.executeExtractionPlan(items);
    } catch (e) {
      setOpsRunning(false);
      setError(String(e));
    }
  }, []);

  const cancelOps = useCallback(async () => {
    try {
      await api.cancelSubtitleOps();
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
        await listen<VideoEntry>("library:entry", (event) => {
          applyEntry(event.payload);
        }),
      );
      unlisteners.push(
        await listen<OpsProgress>("ops:progress", (event) => {
          setOpsProgress(event.payload);
        }),
      );
      unlisteners.push(
        await listen<OpsProgress>("ops:done", (event) => {
          setOpsRunning(false);
          setOpsProgress(null);
          setOpsSummary(event.payload.message || null);
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
  }, [applyEntry]);

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
    opsRunning,
    opsProgress,
    opsSummary,
    setError,
    applyEntry,
    refreshLibrary,
    refreshTools,
    refreshLogs,
    reloadSettings,
    startScan,
    stopScan,
    analyzeVideo,
    analyzeAll,
    extractTrack,
    planExtractionAll,
    executeExtractionPlan,
    cancelOps,
  };

  return <AppDataContext.Provider value={value}>{children}</AppDataContext.Provider>;
}

export function useAppData(): AppDataState {
  const ctx = useContext(AppDataContext);
  if (!ctx) throw new Error("useAppData must be used within AppDataProvider");
  return ctx;
}
