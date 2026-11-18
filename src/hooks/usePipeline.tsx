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
import type {
  PipelineItemSnapshot,
  PipelineSnapshot,
  PipelineSummary,
  PendingDecision,
} from "../types/pipeline";

interface PipelineState {
  /** آخرین snapshot شناخته‌شده — پس از پایان run هم برای نمایش می‌ماند */
  snapshot: PipelineSnapshot | null;
  /** آیتم‌ها به‌ترتیب ورود — upsert بر اساس videoPath */
  items: PipelineItemSnapshot[];
  decision: PendingDecision | null;
  summary: PipelineSummary | null;
  /** run واقعاً در جریان است (running یا awaitingDecision) */
  runActive: boolean;
  /** کنش start/decide/cancel در جریان است */
  busy: boolean;
  /** خطای کنش‌ها (شروع، resolve، لغو) — جدا از رندر run */
  actionError: string | null;
  startPipeline: (paths: string[]) => Promise<boolean>;
  resolveDecision: (optionId: string) => Promise<void>;
  cancelPipeline: () => Promise<void>;
  /** پاک‌سازی نمایش خلاصهٔ پایان — بک‌اند control را خودش حذف کرده است */
  dismissSummary: () => void;
  /** خطای کنش را پاک می‌کند */
  clearActionError: () => void;
}

const PipelineContext = createContext<PipelineState | null>(null);

const TERMINAL_STATUSES = new Set(["completed", "cancelled", "failed"]);

function isRunActive(snapshot: PipelineSnapshot | null): boolean {
  return snapshot !== null && !TERMINAL_STATUSES.has(snapshot.status);
}

export function PipelineProvider({ children }: { children: ReactNode }) {
  const [snapshot, setSnapshot] = useState<PipelineSnapshot | null>(null);
  const [items, setItems] = useState<PipelineItemSnapshot[]>([]);
  const [decision, setDecision] = useState<PendingDecision | null>(null);
  const [summary, setSummary] = useState<PipelineSummary | null>(null);
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const runIdRef = useRef<string | null>(null);
  const seqRef = useRef<number>(0);
  /** runی که رویداد پایانی‌اش رسیده — رویدادهای کهنهٔ همان run دور ریخته می‌شوند */
  const finishedRunRef = useRef<string | null>(null);

  const applySnapshot = useCallback((next: PipelineSnapshot) => {
    if (runIdRef.current !== null && runIdRef.current !== next.runId) {
      // run جدید → وضعیت run قبلی پاک می‌شود (رویدادهای قدیمی معتبر نیستند)
      setItems([]);
      setSummary(null);
      setDecision(null);
      seqRef.current = 0;
      finishedRunRef.current = null;
    }
    if (finishedRunRef.current === next.runId) return; // رویداد کهنه پس از پایان
    runIdRef.current = next.runId;
    // رویداد قدیمی‌تر از snapshot فعلی هرگز وضعیت جدیدتر را overwrite نمی‌کند
    if (next.seq > 0 && next.seq <= seqRef.current) return;
    if (next.seq > 0) seqRef.current = next.seq;
    setSnapshot(next);
    // snapshot مرجع حقیقت تصمیم باز است — رویداد decision فقط اعلام اولیه است
    setDecision(next.pendingDecision ?? null);
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];

    const register = async () => {
      unlisteners.push(
        await listen<PipelineSnapshot>("pipeline:state", (event) => {
          applySnapshot(event.payload);
        }),
      );
      unlisteners.push(
        await listen<PipelineItemSnapshot>("pipeline:item", (event) => {
          const item = event.payload;
          setItems((prev) => {
            const index = prev.findIndex((it) => it.videoPath === item.videoPath);
            if (index === -1) return [...prev, item];
            const next = [...prev];
            next[index] = item;
            return next;
          });
        }),
      );
      unlisteners.push(
        await listen<PendingDecision>("pipeline:decision", (event) => {
          setDecision(event.payload);
        }),
      );
      unlisteners.push(
        await listen<PipelineSummary>("pipeline:done", (event) => {
          const summaryEvent = event.payload;
          // خلاصه مرجع حقیقت پایان است: snapshot وضعیت کهنه (مثلاً running)
          // دارد چون مسیر لغو/شکست رویداد state نهایی نمی‌فرستد؛ آیتم‌های
          // نهایی‌شده هم فقط در خلاصه‌اند (mark_all_cancelled رویداد Item ندارد)
          if (runIdRef.current !== null && runIdRef.current !== summaryEvent.runId) {
            return; // خلاصهٔ run دیگری — نادیده
          }
          finishedRunRef.current = summaryEvent.runId;
          runIdRef.current = summaryEvent.runId;
          setSnapshot(null);
          setItems(summaryEvent.items);
          setDecision(null);
          setSummary(summaryEvent);
        }),
      );
    };

    // ابتدا شنونده‌ها، بعد بازیابی وضعیت — رویداد بین این دو گم نمی‌شود.
    (async () => {
      try {
        await register();
      } catch {
        // حتی اگر ثبت شنونده شکست بخورد، بازیابی وضعیت را امتحان می‌کنیم
      }
      if (disposed) return;
      try {
        const [restored, restoredItems] = await Promise.all([
          api.getPipelineState(),
          api.getPipelineItems(),
        ]);
        if (!disposed && restored) {
          applySnapshot(restored);
          // ادغام: رویدادهای تازه‌ترِ رسیده، بر snapshot بازیابی‌شده مقدم‌اند
          setItems((prev) => {
            const map = new Map(restoredItems.map((it) => [it.videoPath, it]));
            for (const item of prev) map.set(item.videoPath, item);
            return [...map.values()];
          });
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

  const startPipeline = useCallback(async (paths: string[]): Promise<boolean> => {
    setActionError(null);
    setBusy(true);
    try {
      const initial = await api.startPipeline(paths);
      applySnapshot(initial);
      return true;
    } catch (e) {
      setActionError(String(e));
      return false;
    } finally {
      setBusy(false);
    }
  }, [applySnapshot]);

  const resolveDecision = useCallback(
    async (optionId: string) => {
      const current = decision;
      if (!current) return;
      setActionError(null);
      setBusy(true);
      try {
        await api.resolvePipelineDecision(current.decisionId, optionId);
        setDecision(null);
      } catch (e) {
        setActionError(String(e));
        // تصمیم شاید در بک‌اند بسته/عوض شده باشد — وضعیت واقعی را دوباره می‌گیریم
        try {
          const fresh = await api.getPipelineState();
          if (fresh) {
            applySnapshot(fresh);
          } else {
            setDecision(null);
          }
        } catch {
          setDecision(null);
        }
      } finally {
        setBusy(false);
      }
    },
    [decision, applySnapshot],
  );

  const cancelPipeline = useCallback(async () => {
    setActionError(null);
    setBusy(true);
    try {
      // false = pipeline دیگر فعال نیست؛ وضعیت واقعی از رویدادها/snapshot می‌آید
      await api.cancelPipeline();
      const fresh = await api.getPipelineState().catch(() => null);
      if (fresh) applySnapshot(fresh);
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(false);
    }
  }, [applySnapshot]);

  const dismissSummary = useCallback(() => {
    setSummary(null);
    setSnapshot(null);
    setItems([]);
    setDecision(null);
    setActionError(null);
    runIdRef.current = null;
    // پاک‌سازی کامل — وگرنه seq قدیمی اولین snapshot run بعدی را دور می‌ریزد
    seqRef.current = 0;
    finishedRunRef.current = null;
  }, []);

  const value: PipelineState = {
    snapshot,
    items,
    decision,
    summary,
    runActive: isRunActive(snapshot),
    busy,
    actionError,
    startPipeline,
    resolveDecision,
    cancelPipeline,
    dismissSummary,
    clearActionError: () => setActionError(null),
  };

  return <PipelineContext.Provider value={value}>{children}</PipelineContext.Provider>;
}

export function usePipeline(): PipelineState {
  const ctx = useContext(PipelineContext);
  if (!ctx) throw new Error("usePipeline must be used within PipelineProvider");
  return ctx;
}
