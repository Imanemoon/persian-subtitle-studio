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
import type { OpsProgress } from "../types";

/** نوع تسک قابل‌فوکوس — نمای آن در صفحهٔ Home رندر می‌شود. */
export type TaskFocusKind = "pipeline" | "hardsub" | null;

/** وضعیت زندهٔ تسک افزودن زیرنویس چسبیده — از رویدادهای ops با kind اختصاصی. */
export interface SoftsubTaskState {
  active: boolean;
  current: number;
  total: number;
  message: string;
  /** آخرین فهرست ارسالی — برای بازکردن دوبارهٔ مودال از جعبهٔ تسک‌ها */
  lastPaths: string[];
}

interface TaskFocusState {
  focus: TaskFocusKind;
  setFocus: (focus: TaskFocusKind) => void;
  softsubPaths: string[] | null;
  openSoftsub: (paths: string[]) => void;
  closeSoftsub: () => void;
  softsubTask: SoftsubTaskState;
}

const EMPTY_SOFTSUB: SoftsubTaskState = {
  active: false,
  current: 0,
  total: 0,
  message: "",
  lastPaths: [],
};

const TaskFocusContext = createContext<TaskFocusState | null>(null);

/**
 * فوکوس نمای تسک + وضعیت زندهٔ افزودن زیرنویس چسبیده.
 *
 * کاربر می‌تواند نمای یک تسک را با «خانه» ترک کند و کار در پس‌زمینه ادامه
 * یابد؛ جعبهٔ شناور تسک‌ها (TaskDock) وضعیت همهٔ تسک‌های فعال را نشان می‌دهد
 * و با کلیک، نمای همان تسک دوباره باز می‌شود.
 */
export function TaskFocusProvider({ children }: { children: ReactNode }) {
  const [focus, setFocus] = useState<TaskFocusKind>(null);
  const [softsubPaths, setSoftsubPaths] = useState<string[] | null>(null);
  const [softsubTask, setSoftsubTask] = useState<SoftsubTaskState>(EMPTY_SOFTSUB);
  const doneRef = useRef(false);

  const openSoftsub = useCallback((paths: string[]) => {
    if (paths.length === 0) return;
    setSoftsubPaths(paths);
    setSoftsubTask((prev) => ({ ...prev, lastPaths: paths }));
  }, []);

  const closeSoftsub = useCallback(() => setSoftsubPaths(null), []);

  // رویدادهای پیشرفت softsub-add — برای جعبهٔ تسک‌ها (مستقل از مودال)
  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    (async () => {
      unlisteners.push(
        await listen<OpsProgress>("ops:progress", (event) => {
          const p = event.payload;
          if (p.kind !== "softsub-add") return;
          doneRef.current = false;
          setSoftsubTask((prev) => ({
            active: true,
            current: p.current,
            total: p.total,
            message: p.message,
            lastPaths: prev.lastPaths.length > 0 ? prev.lastPaths : [],
          }));
        }),
      );
      unlisteners.push(
        await listen<OpsProgress>("ops:done", (event) => {
          const p = event.payload;
          if (p.kind !== "softsub-add") return;
          doneRef.current = true;
          setSoftsubTask((prev) => ({
            ...prev,
            active: false,
            message: p.message,
          }));
        }),
      );
      if (disposed) unlisteners.forEach((u) => u());
    })();
    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, []);

  const value: TaskFocusState = {
    focus,
    setFocus,
    softsubPaths,
    openSoftsub,
    closeSoftsub,
    softsubTask,
  };

  return <TaskFocusContext.Provider value={value}>{children}</TaskFocusContext.Provider>;
}

export function useTaskFocus(): TaskFocusState {
  const ctx = useContext(TaskFocusContext);
  if (!ctx) throw new Error("useTaskFocus must be used within TaskFocusProvider");
  return ctx;
}
