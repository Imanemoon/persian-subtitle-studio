import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  Loader2,
  Shield,
  Sparkles,
  XCircle,
} from "lucide-react";
import { Badge, Button, Card } from "../components/ui";
import {
  api,
  type SetupProgressPayload,
  type SetupToolStatus,
} from "../lib/api";
import type { ToolStatus } from "../types";

/**
 * معاونت راه‌اندازی — قبل از ورود به برنامه.
 * یک دکمهٔ اصلی، همه‌چیز را ترتیبی نصب می‌کند؛ هر جزئیات دیگری حذف شده تا
 * کاربر فقط یک تصمیم بگیرد. همهٔ داده‌ها از بک‌اند می‌آید (detect واقعی +
 * progress واقعی)؛ هیچ پیشرفت جعلی‌ای وجود ندارد.
 */

interface PerTool {
  progress: string | null;
  phase: string;
  received: number;
  total: number;
}

export function SetupPage({
  onEnter,
}: {
  onEnter: () => void;
}) {
  const [tools, setTools] = useState<SetupToolStatus[] | null>(null);
  const [perTool, setPerTool] = useState<Record<string, PerTool>>({});
  const [installing, setInstalling] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [defenderRunning, setDefenderRunning] = useState(false);
  const [defenderMessage, setDefenderMessage] = useState<string | null>(null);
  const [defenderOk, setDefenderOk] = useState(false);
  const [shortcutMessage, setShortcutMessage] = useState<string | null>(null);
  const [shortcutOk, setShortcutOk] = useState(false);
  const [mediaDir, setMediaDir] = useState<string | null>(null);
  const [appDirHint, setAppDirHint] = useState<string | null>(null);
  const doneResolversRef = useRef<Array<(ok: boolean) => void>>([]);

  const refresh = async () => {
    try {
      setTools(await api.setupStatus());
    } catch (e) {
      setError(`خواندن وضعیت ابزارها ناموفق: ${errorText(e)}`);
    }
  };

  useEffect(() => {
    let unlisteners: Array<() => void> = [];
    void refresh();
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const u1 = await listen<SetupProgressPayload>("setup:progress", (e) => {
        const p = e.payload;
        setPerTool((prev) => ({
          ...prev,
          [p.toolId]: {
            progress: p.message,
            phase: p.phase,
            received: p.receivedBytes,
            total: p.totalBytes,
          },
        }));
      });
      const u2 = await listen<{ toolId: string; ok: boolean; error: string | null }>(
        "setup:done",
        (e) => {
          const { toolId, ok, error: err } = e.payload;
          setInstalling(null);
          if (ok) {
            setNotice(`نصب ${toolId} کامل شد.`);
            setPerTool((prev) => ({
              ...prev,
              [toolId]: { progress: "نصب کامل شد", phase: "done", received: 1, total: 1 },
            }));
          } else {
            setPerTool((prev) => ({
              ...prev,
              [toolId]: { progress: null, phase: "idle", received: 0, total: 0 },
            }));
            setError(`نصب ${toolId} ناموفق: ${err ?? "خطای نامشخص"}`);
          }
          void refresh();
          const resolvers = doneResolversRef.current;
          doneResolversRef.current = [];
          for (const r of resolvers) r(ok);
        },
      );
      unlisteners = [u1, u2];
    })();
    return () => {
      for (const u of unlisteners) u();
    };
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        const hints = await api.getAppDirHints();
        setAppDirHint(hints.installDir || hints.toolsDir || null);
      } catch {
        setAppDirHint(null);
      }
    })();
  }, []);

  /** نصب یک ابزار — Promise تا رویداد setup:done همین ابزار برسد. */
  const installAndWait = (id: string): Promise<boolean> => {
    setInstalling(id);
    setPerTool((prev) => ({
      ...prev,
      [id]: { progress: "شروع…", phase: "start", received: 0, total: 0 },
    }));
    return new Promise((resolve) => {
      doneResolversRef.current.push((ok) => resolve(ok));
      void api.installTool(id).catch((e) => {
        // شکست هم‌زمان (مثلاً نصب دیگر در جریان است) — resolverها را تخلیه کن
        const resolvers = doneResolversRef.current;
        doneResolversRef.current = [];
        for (const r of resolvers) r(false);
        setInstalling(null);
        setError(`شروع نصب ناموفق: ${errorText(e)}`);
        void refresh();
      });
    });
  };

  const install = async (id: string) => {
    setError(null);
    setNotice(null);
    await installAndWait(id);
  };

  const installAll = async () => {
    if (!tools) return;
    setError(null);
    setNotice(null);
    // ترتیبی و ادامه‌دار — شکست یک ابزار بقیه را متوقف نمی‌کند
    const failed: string[] = [];
    for (const t of tools) {
      if (t.status !== "found") {
        const ok = await installAndWait(t.id);
        if (!ok) failed.push(t.label);
      }
    }
    if (failed.length > 0) {
      setError(`این مورد نصب نشد: ${failed.join("، ")} — دوباره تلاش کنید.`);
    } else {
      setNotice("همهٔ پیش‌نیازها آماده شد.");
    }
  };

  const addExclusion = async (paths: string[]) => {
    setDefenderRunning(true);
    setDefenderMessage(null);
    setDefenderOk(false);
    try {
      const message = await api.addDefenderExclusion(paths);
      setDefenderMessage(message);
      setDefenderOk(true);
    } catch (e) {
      setDefenderMessage(`افزودن استثنا ناموفق: ${errorText(e)}`);
    } finally {
      setDefenderRunning(false);
    }
  };

  const addShortcut = async () => {
    setShortcutMessage(null);
    setShortcutOk(false);
    try {
      setShortcutMessage(await api.createAdminShortcut());
      setShortcutOk(true);
    } catch (e) {
      setShortcutMessage(`ساخت میان‌بر ناموفق: ${errorText(e)}`);
    }
  };

  const statusBadge = (status: ToolStatus) => {
    switch (status) {
      case "found":
        return <Badge tone="success">آماده است</Badge>;
      case "broken":
        return <Badge tone="warn">خراب — نصب دوباره</Badge>;
      default:
        return <Badge tone="danger">نصب نشده</Badge>;
    }
  };

  const coreReady =
    tools !== null &&
    tools.filter((t) => t.requiredForEntry).every((t) => t.status === "found");

  const pending = tools === null ? 0 : tools.filter((t) => t.status !== "found").length;
  const busy = installing !== null;

  return (
    <div className="flex h-full justify-center overflow-y-auto px-6 py-10">
      <div className="w-full max-w-3xl space-y-6">
        <div className="text-center">
          <h1 className="flex items-center justify-center gap-2 text-2xl font-bold text-ink-1">
            <Sparkles className="size-6 text-accent" />
            راه‌اندازی SubSync Studio
          </h1>
          <p className="mt-2 text-sm leading-6 text-ink-2">
            فقط یک بار — همه‌چیز خودکار نصب می‌شود، داخل پوشهٔ خود برنامه و بدون
            admin.
          </p>
        </div>

        {/* ابزارها */}
        <Card className="space-y-1 p-6">
          {tools !== null && pending > 0 && (
            <div className="pb-2">
              <Button
                variant="primary"
                className="h-11 w-full text-base"
                disabled={busy}
                loading={busy}
                onClick={() => void installAll()}
              >
                <Download className="size-4" />
                نصب همهٔ پیش‌نیازها
              </Button>
              <p className="pt-1.5 text-center text-[11px] text-ink-3">
                {pending} مورد باقی مانده — دانلود و نصب خودکار، ترتیبی
              </p>
            </div>
          )}
          {tools === null ? (
            <div className="flex items-center justify-center gap-2 py-8 text-sm text-ink-3">
              <Loader2 className="size-4 animate-spin" />
              بررسی وضعیت ابزارها…
            </div>
          ) : (
            tools.map((t) => {
              const state = perTool[t.id];
              const busyThis = installing === t.id;
              return (
                <div
                  key={t.id}
                  className="flex items-start gap-3 border-b border-line/60 py-4 last:border-0"
                >
                  <span className="mt-0.5 shrink-0">
                    {t.status === "found" ? (
                      <CheckCircle2 className="size-5 text-emerald-400" />
                    ) : (
                      <XCircle className="size-5 text-red-400" />
                    )}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-sm font-semibold text-ink-1">
                        {t.label}
                      </span>
                      {statusBadge(t.status)}
                      <span className="text-[11px] text-ink-3">{t.sizeHint}</span>
                    </div>
                    {busyThis && state?.progress && (
                      <div className="mt-2 space-y-1.5">
                        <div className="h-2 w-full overflow-hidden rounded-full bg-zinc-800">
                          <div
                            className="h-full rounded-full bg-accent transition-all"
                            style={{
                              width:
                                state.total > 0
                                  ? `${Math.min(100, Math.round((state.received / state.total) * 100))}%`
                                  : state.phase === "download"
                                    ? "8%"
                                    : "4%",
                            }}
                          />
                        </div>
                        <p
                          dir="ltr"
                          className="truncate text-start font-mono text-[11px] text-ink-3"
                        >
                          {state.progress}
                        </p>
                      </div>
                    )}
                  </div>
                  {t.status !== "found" && (
                    <Button
                      variant={busyThis ? undefined : "primary"}
                      disabled={busy}
                      loading={busyThis}
                      onClick={() => void install(t.id)}
                    >
                      {busyThis ? "در حال نصب…" : "نصب"}
                    </Button>
                  )}
                </div>
              );
            })
          )}
        </Card>

        {error && (
          <p className="flex items-start gap-2 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-400">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            <span>{error}</span>
          </p>
        )}
        {notice && <p className="text-center text-xs text-emerald-400">{notice}</p>}

        {/* سرعت بیشتر — اختیاری، کامپکت */}
        <Card className="space-y-3 p-6">
          <div className="flex items-center gap-2">
            <Shield className="size-5 text-accent" />
            <h2 className="text-sm font-bold text-ink-1">سرعت بیشتر (اختیاری)</h2>
          </div>
          <p className="text-xs leading-5 text-ink-2">
            هر دکمه یک پنجرهٔ تأیید ویندوز (UAC) باز می‌کند — با تأیید شما اعمال
            می‌شود.
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="primary"
              disabled={defenderRunning || appDirHint === null}
              loading={defenderRunning}
              onClick={() => {
                const paths = [appDirHint].filter((p): p is string => Boolean(p));
                if (mediaDir) paths.push(mediaDir);
                void addExclusion(paths);
              }}
            >
              افزودن به استثنای Defender
            </Button>
            <Button
              disabled={defenderRunning}
              onClick={async () => {
                const picked = await api.pickFolder();
                if (picked) {
                  setMediaDir(picked);
                  void addExclusion([picked]);
                }
              }}
            >
              استثنای پوشهٔ رسانه‌ها…
            </Button>
            <Button disabled={defenderRunning} onClick={() => void addShortcut()}>
              میان‌بر Administrator روی دسکتاپ
            </Button>
          </div>
          {mediaDir && (
            <span
              dir="ltr"
              className="block max-w-md truncate font-mono text-[11px] text-ink-3"
            >
              {mediaDir}
            </span>
          )}
          {defenderMessage && (
            <p
              className={`text-xs leading-5 ${defenderOk ? "text-emerald-400" : "text-ink-2"}`}
            >
              {defenderMessage}
            </p>
          )}
          {shortcutMessage && (
            <p
              className={`text-xs leading-5 ${shortcutOk ? "text-emerald-400" : "text-ink-2"}`}
            >
              {shortcutMessage}
            </p>
          )}
        </Card>

        {/* ورود به برنامه */}
        <div className="flex flex-col items-center gap-2 pb-6">
          <Button
            variant="primary"
            className="h-12 min-w-64 text-base"
            onClick={onEnter}
          >
            {coreReady ? "ورود به برنامه" : "رد کردن و ورود به برنامه"}
          </Button>
          {tools !== null && !coreReady && (
            <p className="text-xs text-amber-400/90">
              بدون FFmpeg استخراج و سینک کار نمی‌کند — می‌توانید بعداً از
              تنظیمات نصب کنید.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}
