import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import type { HardsubSnapshot, HardsubSummary } from "../types/hardsub";

interface HardsubState {
  /** آخرین snapshot شناخته‌شده — پس از پایان run هم برای نمایش می‌ماند */
  snapshot: HardsubSnapshot | null;
  /** خلاصهٔ نهایی پس از رویداد hardsub:done — منبع حقیقت پایان */
  summary: HardsubSummary | null;
  /** run واقعاً در جریان است */
  runActive: boolean;
  /** کنش start/cancel در جریان است */
  busy: boolean;
  /** خطای کنش‌ها (شروع، لغو) — جدا از رندر run */
  actionError: string | null;
  startHardsub: (paths: string[]) => Promise<boolean>;
  cancelHardsub: () => Promise<void>;
  /** پاک‌سازی نمایش پس از پایان */
  dismiss: () => void;
  clearActionError: () => void;
}

const HardsubContext = createContext<HardsubState | null>(null);

export function HardsubProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<HardsubSnapshot | null>(null);
  const [summary, setSummary] = useState<HardsubSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const runIdRef = useRef<string | null>(null);
  const seqRef = useRef<number>(0);
  /** runی که رویداد پایانی‌اش رسیده — رویدادهای کهنهٔ همان run دور ریخته می‌شوند */
  const finishedRunRef = useRef<string | null>(null);

  const applySnapshot = useCallback((next: HardsubSnapshot) => {
    if (runIdRef.current !== null && runIdRef.current !== next.runId) {
      // run جدید → وضعیت run قبلی پاک می‌شود
      setSummary(null);
      seqRef.current = 0;
      finishedRunRef.current = null;
    }
    if (finishedRunRef.current === next.runId) return; // رویداد کهنه پس از پایان
    runIdRef.current = next.runId;
    // رویداد قدیمی‌تر از snapshot فعلی هرگز وضعیت جدیدتر را overwrite نمی‌کند
    if (next.seq > 0 && next.seq <= seqRef.current) return;
    if (next.seq > 0) seqRef.current = next.seq;
    setSnapshot(next);
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    const register = async () => {
      unlisteners.push(
        await listen<HardsubSnapshot>("hardsub:state", (event) => {
          applySnapshot(event.payload);
        }),
      );
      unlisteners.push(
        await listen<HardsubSummary>("hardsub:done", (event) => {
          const summaryEvent = event.payload;
          if (runIdRef.current !== null && runIdRef.current !== summaryEvent.runId) {
            return; // خلاصهٔ run دیگری — نادیده
          }
          finishedRunRef.current = summaryEvent.runId;
          runIdRef.current = summaryEvent.runId;
          setSummary(summaryEvent);
        }),
      );
    };

    // ابتدا شنونده‌ها، بعد بازیابی وضعیت — رویداد بین این دو گم نمی‌شود
    (async () => {
      try {
        await register();
      } catch {
        // حتی اگر ثبت شنونده شکست بخورد، بازیابی وضعیت را امتحان می‌کنیم
      }
      if (disposed) return;
      try {
        const restored = await api.getHardsubState();
        if (!disposed && restored) {
          applySnapshot(restored);
        }
      } catch {
        // بک‌اند در دسترس نیست — رویدادهای بعدی وضعیت را می‌سازند
      }
    })();

    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, [applySnapshot]);

  const startHardsub = useCallback(
    async (paths: string[]): Promise<boolean> => {
      setActionError(null);
      setBusy(true);
      try {
        const initial = await api.startHardsub(paths);
        applySnapshot(initial);
        return true;
      } catch (e) {
        setActionError(String(e));
        return false;
      } finally {
        setBusy(false);
      }
    },
    [applySnapshot],
  );

  const cancelHardsub = useCallback(async () => {
    setActionError(null);
    setBusy(true);
    try {
      await api.cancelHardsub();
      const fresh = await api.getHardsubState().catch(() => null);
      if (fresh) applySnapshot(fresh);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
  }, [applySnapshot]);

  const dismiss = useCallback(() => {
    setSummary(null);
    setSnapshot(null);
    setActionError(null);
    runIdRef.current = null;
    // پاک‌سازی کامل — وگرنه seq قدیمی اولین snapshot run بعدی را دور می‌ریزد
    seqRef.current = 0;
    finishedRunRef.current = null;
  }, []);

  const value: HardsubState = {
    snapshot,
    summary,
    runActive: snapshot !== null && snapshot.running,
    busy,
    actionError,
    startHardsub,
    cancelHardsub,
    dismiss,
    clearActionError: () => setActionError(null),
  };

  return <HardsubContext.Provider value={value}>{children}</HardsubContext.Provider>;
}

export function useHardsub(): HardsubState {
  const ctx = useContext(HardsubContext);
  if (!ctx) throw new Error("useHardsub must be used within HardsubProvider");
  return ctx;
}
