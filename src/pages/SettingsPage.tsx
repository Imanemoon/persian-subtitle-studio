import { useEffect, useMemo, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { Check, FolderOpen, KeyRound, Plus, Save, Settings as SettingsIcon, Trash2, Wrench, X } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { Badge, Button, Card, SectionTitle, Toggle } from "../components/ui";
import { api } from "../lib/api";
import type { Settings, ToolPathOverrides } from "../types";

const TOOL_FIELDS: { key: keyof ToolPathOverrides; label: string; hint: string }[] = [
  { key: "ffmpeg", label: "FFmpeg", hint: "استخراج و بررسی رسانه" },
  { key: "ffprobe", label: "FFprobe", hint: "خواندن اطلاعات Streamها" },
  { key: "subsource", label: "SubSource CLI (sub-rs.exe)", hint: "دانلود زیرنویس (فاز ۳)" },
  { key: "sevenZip", label: "7-Zip", hint: "استخراج ZIP/RAR (فاز ۳)" },
  { key: "ffsubsync", label: "FFsubsync (ffs.exe)", hint: "همگام‌سازی (فاز ۴)" },
];

export function SettingsPage() {
  const { settings, reloadSettings, tools, refreshTools, toolsLoading } = useAppData();
  const [draft, setDraft] = useState<Settings | null>(null);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<{ tone: "success" | "danger"; text: string } | null>(null);
  const [newExcluded, setNewExcluded] = useState("");
  // ---- SubSource API key — کاملاً مستقل از draft و دکمهٔ ذخیرهٔ تنظیمات؛
  // مقدار کلید فقط در این input محلی می‌ماند و هرگز به Settings/draft نمی‌رود.
  // خطای خواندن Credential Manager = «نامشخص»، نه «ذخیره نشده» ----
  const [keyStatus, setKeyStatus] = useState<"checking" | "stored" | "missing" | "unknown">(
    "checking",
  );
  const [keyInput, setKeyInput] = useState("");
  const [keyBusy, setKeyBusy] = useState<"save" | "delete" | null>(null);
  const [keyMessage, setKeyMessage] = useState<{ tone: "success" | "danger"; text: string } | null>(
    null,
  );

  useEffect(() => {
    let cancelled = false;
    api
      .hasSubsourceApiKey()
      .then((stored) => {
        if (!cancelled) setKeyStatus(stored ? "stored" : "missing");
      })
      .catch((e) => {
        if (!cancelled) {
          // خطای زیرساخت هرگز به «ذخیره نشده» تبدیل نمی‌شود — وضعیت نامشخص می‌ماند
          setKeyStatus("unknown");
          setKeyMessage({
            tone: "danger",
            text: `خواندن وضعیت کلید از Credential Manager ناموفق بود — وضعیت نامشخص است: ${String(e)}`,
          });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (settings && draft === null) {
      setDraft(structuredClone(settings));
    }
  }, [settings, draft]);

  const dirty = useMemo(
    () => (draft && settings ? JSON.stringify(draft) !== JSON.stringify(settings) : false),
    [draft, settings],
  );

  if (!draft) {
    return (
      <div className="mx-auto max-w-4xl p-6">
        <h1 className="text-xl font-bold">تنظیمات</h1>
        <p className="mt-2 text-sm text-ink-3">در حال بارگذاری…</p>
      </div>
    );
  }

  const patch = (partial: Partial<Settings>) => {
    setDraft((d) => (d ? { ...d, ...partial } : d));
    setMessage(null);
  };

  const pickRoot = async () => {
    const picked = await api.pickFolder();
    if (picked) patch({ rootFolder: picked });
  };

  const addExcluded = () => {
    const value = newExcluded.trim();
    if (!value) return;
    if (draft.excludedFolders.some((f) => f.toLowerCase() === value.toLowerCase())) {
      setNewExcluded("");
      return;
    }
    patch({ excludedFolders: [...draft.excludedFolders, value] });
    setNewExcluded("");
  };

  const save = async () => {
    setSaving(true);
    setMessage(null);
    try {
      const saved = await api.saveSettings(draft);
      await reloadSettings();
      setDraft(structuredClone(saved));
      setMessage({ tone: "success", text: "تنظیمات با موفقیت ذخیره شد." });
    } catch (e) {
      setMessage({ tone: "danger", text: `ذخیره ناموفق بود: ${String(e)}` });
    } finally {
      setSaving(false);
    }
  };

  const toolStatus = (key: keyof ToolPathOverrides) => {
    const id = key === "sevenZip" ? "sevenZip" : key;
    return tools.find((t) => t.id === id)?.status ?? null;
  };

  // ---- کنش‌های API key — مستقل از save عمومی تنظیمات ----

  /** بازخوانی وضعیت کلید؛ verified=false یعنی خواندن ناموفق بود و وضعیت «نامشخص» می‌ماند.
   * پیام را فراخواننده می‌سازد تا نتیجهٔ خودش (ذخیره/حذف موفق) با خطای refresh ترکیب شود. */
  const refreshKeyStatus = async (): Promise<{ verified: boolean; error: string | null }> => {
    try {
      const stored = await api.hasSubsourceApiKey();
      setKeyStatus(stored ? "stored" : "missing");
      return { verified: true, error: null };
    } catch (e) {
      setKeyStatus("unknown");
      return { verified: false, error: String(e) };
    }
  };

  const saveKey = async () => {
    const value = keyInput.trim();
    if (!value) {
      setKeyMessage({ tone: "danger", text: "مقدار API key خالی است." });
      return;
    }
    setKeyBusy("save");
    setKeyMessage(null);
    try {
      await api.setSubsourceApiKey(value);
      setKeyInput("");
      // نتیجهٔ خود ذخیره از بازخوانی وضعیت جدا گزارش می‌شود تا خطای بررسی
      // هرگز با پیام موفقیت جایگزین نشود.
      const { verified, error } = await refreshKeyStatus();
      setKeyMessage(
        verified
          ? { tone: "success", text: "API key در Credential Manager ذخیره شد." }
          : {
              tone: "danger",
              text: `ذخیرهٔ API key انجام شد، اما خواندن مجدد وضعیت ناموفق بود — وضعیت کلید فعلاً نامشخص است: ${error ?? ""}`,
            },
      );
    } catch (e) {
      setKeyMessage({ tone: "danger", text: `ذخیرهٔ API key ناموفق بود: ${String(e)}` });
    } finally {
      setKeyBusy(null);
    }
  };

  const deleteKey = async () => {
    const ok = await ask(
      "کلید ذخیره‌شدهٔ SubSource حذف شود؟\n\nبرای دانلود بعدی باید دوباره کلید وارد کنید. خود کلید هرگز نمایش داده نمی‌شود.",
      { title: "حذف SubSource API key", kind: "warning" },
    );
    if (!ok) return;
    setKeyBusy("delete");
    setKeyMessage(null);
    try {
      await api.deleteSubsourceApiKey();
      setKeyInput("");
      const { verified, error } = await refreshKeyStatus();
      setKeyMessage(
        verified
          ? { tone: "success", text: "API key از Credential Manager حذف شد." }
          : {
              tone: "danger",
              text: `حذف API key انجام شد، اما خواندن مجدد وضعیت ناموفق بود — وضعیت کلید فعلاً نامشخص است: ${error ?? ""}`,
            },
      );
    } catch (e) {
      setKeyMessage({ tone: "danger", text: `حذف API key ناموفق بود: ${String(e)}` });
    } finally {
      setKeyBusy(null);
    }
  };

  return (
    <div className="mx-auto max-w-4xl space-y-6 p-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-bold">تنظیمات</h1>
          <p className="mt-1 text-sm text-ink-3">پس از بستن و بازکردن برنامه حفظ می‌شوند.</p>
        </div>
        <div className="flex items-center gap-3">
          {dirty && <Badge tone="warn">تغییرات ذخیره‌نشده</Badge>}
          <Button variant="primary" onClick={save} loading={saving}>
            <Save className="size-4" />
            ذخیره
          </Button>
        </div>
      </div>

      {message && (
        <div
          className={`flex items-center justify-between gap-3 rounded-lg px-4 py-3 text-sm ${
            message.tone === "success"
              ? "border border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
              : "border border-red-500/30 bg-red-500/10 text-red-300"
          }`}
        >
          <span>{message.text}</span>
          <button onClick={() => setMessage(null)}>✕</button>
        </div>
      )}

      <Card className="p-5">
        <SectionTitle icon={SettingsIcon} title="عمومی" />
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-40 shrink-0 text-sm text-ink-2">پوشه ریشه پیش‌فرض</span>
            <span dir="ltr" className="min-w-0 flex-1 truncate rounded-lg border border-line bg-surface-2 px-3 py-2 text-start font-mono text-xs text-ink-2">
              {draft.rootFolder ?? "تعیین نشده"}
            </span>
            <Button onClick={pickRoot}>
              <FolderOpen className="size-4" />
              انتخاب
            </Button>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-40 shrink-0 text-sm text-ink-2">پوشه لاگ‌ها</span>
            <input
              dir="ltr"
              value={draft.logsDir ?? ""}
              onChange={(e) => patch({ logsDir: e.target.value || null })}
              placeholder="(پیش‌فرض سیستم — خالی بگذارید)"
              className="h-10 min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 text-start font-mono text-xs outline-none focus:border-accent"
            />
          </div>
          <p className="text-xs text-ink-3">
            تغییر پوشه لاگ پس از ذخیره و ری‌استارت برنامه اعمال می‌شود.
          </p>
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle icon={X} title="پوشه‌های مستثنا" />
        <div className="space-y-3">
          <div className="flex flex-wrap gap-2">
            {draft.excludedFolders.map((folder) => (
              <span
                key={folder}
                className="inline-flex items-center gap-2 rounded-lg border border-line bg-surface-2 px-3 py-1.5 text-sm text-ink-1"
              >
                {folder}
                <button
                  onClick={() =>
                    patch({ excludedFolders: draft.excludedFolders.filter((f) => f !== folder) })
                  }
                  className="text-ink-3 hover:text-red-400"
                  title="حذف"
                >
                  <X className="size-3.5" />
                </button>
              </span>
            ))}
            {draft.excludedFolders.length === 0 && (
              <span className="text-xs text-ink-3">هیچ پوشه‌ای مستثنا نشده است.</span>
            )}
          </div>
          <div className="flex gap-2">
            <input
              value={newExcluded}
              onChange={(e) => setNewExcluded(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && addExcluded()}
              placeholder="نام پوشه (مثلاً Extras)…"
              className="h-10 flex-1 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            />
            <Button onClick={addExcluded}>
              <Plus className="size-4" />
              افزودن
            </Button>
          </div>
          <p className="text-xs text-ink-3">
            این پوشه‌ها (به‌علاوه پوشه‌های مخفی و پوشه‌های کاری _SubSource_*) هنگام اسکن نادیده گرفته می‌شوند.
          </p>
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle
          icon={Wrench}
          title="ابزارهای خارجی"
          extra={
            <Button
              onClick={refreshTools}
              loading={toolsLoading}
              className="h-8 px-3 text-xs"
            >
              بررسی دوباره
            </Button>
          }
        />
        {/* وضعیت زندهٔ ابزارها — از داشبورد قبلی به تنظیمات منتقل شده */}
        <div className="mb-4 space-y-2">
          {tools.map((t) => {
            const tone =
              t.status === "found" ? "success" : t.status === "missing" ? "warn" : "danger";
            const label = t.status === "found" ? "آماده" : t.status === "missing" ? "پیدا نشد" : "خراب";
            return (
              <div
                key={t.id}
                className="flex items-center gap-3 rounded-lg border border-line bg-surface-2 px-3 py-2"
              >
                <span
                  className={`size-2 shrink-0 rounded-full ${
                    t.status === "found"
                      ? "bg-emerald-500"
                      : t.status === "missing"
                        ? "bg-amber-500"
                        : "bg-red-500"
                  }`}
                />
                <span className="w-44 shrink-0 text-sm text-ink-1">{t.name}</span>
                <Badge tone={tone}>{label}</Badge>
                <span
                  dir="ltr"
                  className="min-w-0 flex-1 truncate font-mono text-xs text-ink-3"
                  title={t.path ?? t.detail ?? undefined}
                >
                  {t.path ?? t.detail ?? "—"}
                </span>
                {t.version && (
                  <span
                    dir="ltr"
                    className="hidden shrink-0 font-mono text-[11px] text-ink-3 xl:block"
                  >
                    {t.version}
                  </span>
                )}
              </div>
            );
          })}
        </div>
        <div className="space-y-3">
          {TOOL_FIELDS.map(({ key, label, hint }) => {
            const status = toolStatus(key);
            return (
              <div key={key} className="flex flex-wrap items-center gap-2">
                <div className="w-56 shrink-0">
                  <div className="flex items-center gap-2 text-sm text-ink-1">
                    {label}
                    {status && (
                      <Badge tone={status === "found" ? "success" : status === "missing" ? "warn" : "danger"}>
                        {status === "found" ? "آماده" : status === "missing" ? "پیدا نشد" : "خراب"}
                      </Badge>
                    )}
                  </div>
                  <div className="text-[11px] text-ink-3">{hint}</div>
                </div>
                <input
                  dir="ltr"
                  value={draft.toolPaths[key] ?? ""}
                  onChange={(e) =>
                    patch({
                      toolPaths: { ...draft.toolPaths, [key]: e.target.value || null },
                    })
                  }
                  placeholder="(تشخیص خودکار)"
                  className="h-10 min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 text-start font-mono text-xs outline-none focus:border-accent"
                />
                <Button
                  onClick={async () => {
                    const picked = await api.pickExecutable();
                    if (picked) patch({ toolPaths: { ...draft.toolPaths, [key]: picked } });
                  }}
                  title="انتخاب فایل اجرایی"
                >
                  <FolderOpen className="size-4" />
                </Button>
                {draft.toolPaths[key] && (
                  <Button
                    onClick={() => patch({ toolPaths: { ...draft.toolPaths, [key]: null } })}
                    title="بازگشت به تشخیص خودکار"
                  >
                    <Trash2 className="size-4" />
                  </Button>
                )}
              </div>
            );
          })}
          <p className="text-xs text-ink-3">
            در شروع اجرا و با دکمهٔ «بررسی دوباره»، سلامت ابزارها آزموده می‌شود (اجرای
            <span dir="ltr" className="mx-1 font-mono">--version/--help</span>).
          </p>
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle icon={KeyRound} title="SubSource API Key" />
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-40 shrink-0 text-sm text-ink-2">وضعیت کلید</span>
            {keyStatus === "checking" ? (
              <Badge tone="warn">در حال بررسی…</Badge>
            ) : keyStatus === "stored" ? (
              <Badge tone="success">ذخیره‌شده در Credential Manager</Badge>
            ) : keyStatus === "missing" ? (
              <Badge tone="warn">ذخیره نشده</Badge>
            ) : (
              <Badge tone="danger">نامشخص — خطا در خواندن وضعیت</Badge>
            )}
          </div>
          <p className="text-xs text-ink-3">
            این وضعیت فقط یعنی کلیدی روی همین سیستم ذخیره شده است؛ معتبر بودن آن هنگام دانلود توسط
            SubSource بررسی می‌شود. خود کلید هرگز نمایش داده نمی‌شود و در settings.json نوشته نمی‌شود.
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-40 shrink-0 text-sm text-ink-2">کلید جدید</span>
            <input
              type="password"
              dir="ltr"
              autoComplete="new-password"
              value={keyInput}
              disabled={keyBusy !== null}
              onChange={(e) => {
                setKeyInput(e.target.value);
                setKeyMessage(null);
              }}
              placeholder="API key را اینجا وارد کنید…"
              className="h-10 min-w-0 flex-1 rounded-lg border border-line bg-surface-2 px-3 text-start font-mono text-xs outline-none focus:border-accent disabled:cursor-not-allowed disabled:opacity-50"
            />
          </div>
          <div className="flex flex-wrap gap-2">
            <Button
              variant="primary"
              onClick={saveKey}
              loading={keyBusy === "save"}
              disabled={keyBusy !== null || !keyInput.trim()}
            >
              <Save className="size-4" />
              ذخیره کلید
            </Button>
            <Button
              variant="danger"
              onClick={deleteKey}
              loading={keyBusy === "delete"}
              disabled={keyBusy !== null || keyStatus !== "stored"}
              title="حذف کلید ذخیره‌شده"
            >
              <Trash2 className="size-4" />
              حذف کلید
            </Button>
          </div>
          {keyMessage && (
            <div
              className={`flex items-center justify-between gap-3 rounded-lg px-4 py-3 text-sm ${
                keyMessage.tone === "success"
                  ? "border border-emerald-500/30 bg-emerald-500/10 text-emerald-300"
                  : "border border-red-500/30 bg-red-500/10 text-red-300"
              }`}
            >
              <span>{keyMessage.text}</span>
              <button onClick={() => setKeyMessage(null)}>✕</button>
            </div>
          )}
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle icon={Check} title="پردازش و سینک (فاز ۳ و ۴)" />
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-56 shrink-0 text-sm text-ink-2">تعداد Workerهای هم‌زمان سینک</span>
            <input
              type="number"
              min={1}
              max={16}
              value={draft.processing.syncWorkers}
              onChange={(e) =>
                patch({
                  processing: {
                    ...draft.processing,
                    syncWorkers: Number(e.target.value) || 1,
                  },
                })
              }
              className="h-10 w-24 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            />
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-56 shrink-0 text-sm text-ink-2">تعداد Segmentهای Multi-Segment Sync</span>
            <input
              type="number"
              min={1}
              max={12}
              value={draft.processing.segmentCount}
              onChange={(e) =>
                patch({
                  processing: {
                    ...draft.processing,
                    segmentCount: Number(e.target.value) || 1,
                  },
                })
              }
              className="h-10 w-24 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            />
          </div>
          <Toggle
            checked={draft.processing.multiSegment}
            onChange={(v) => patch({ processing: { ...draft.processing, multiSegment: v } })}
            label="Multi-Segment Sync"
            hint="سینک دقیق‌تر با نمونه‌برداری چندبخشی (پیش‌فرض: فعال)"
          />
          <Toggle
            checked={draft.processing.skipIntroOutro}
            onChange={(v) => patch({ processing: { ...draft.processing, skipIntroOutro: v } })}
            label="رد کردن تیتراژ اول/آخر"
            hint="مطابق تجربه اسکریپت قبلی (پیش‌فرض: فعال)"
          />
          <Toggle
            checked={draft.processing.skipSyncOnLowQuality}
            onChange={(v) =>
              patch({ processing: { ...draft.processing, skipSyncOnLowQuality: v } })
            }
            label="رد کردن سینک در کیفیت پایین تطبیق"
            hint="جلوگیری از خروجی نامطمئن (پیش‌فرض: فعال)"
          />
        </div>
      </Card>
    </div>
  );
}
