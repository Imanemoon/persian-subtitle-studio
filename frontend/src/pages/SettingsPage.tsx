import { useEffect, useMemo, useState } from "react";
import { ask } from "@tauri-apps/plugin-dialog";
import { Bell, Check, FileText, Film, FolderOpen, KeyRound, Layers, Plus, Save, Settings as SettingsIcon, Trash2, Wrench, X, Zap } from "lucide-react";
import { useAppData } from "../hooks/useAppData";
import { Badge, Button, Card, SectionTitle, Toggle } from "../components/ui";
import { api } from "../lib/api";
import type {
  AutomationMode,
  HardsubSettings,
  NamingPreview,
  ProcessingProfile,
  Settings,
  ToolPathOverrides,
} from "../types";

const TOOL_FIELDS: { key: keyof ToolPathOverrides; label: string; hint: string }[] = [
  { key: "ffmpeg", label: "FFmpeg", hint: "استخراج و بررسی رسانه" },
  { key: "ffprobe", label: "FFprobe", hint: "خواندن اطلاعات Streamها" },
  { key: "subsource", label: "SubSource CLI (sub-rs.exe)", hint: "دانلود زیرنویس (فاز ۳)" },
  { key: "sevenZip", label: "7-Zip", hint: "استخراج ZIP/RAR (فاز ۳)" },
  { key: "ffsubsync", label: "FFsubsync (ffs.exe)", hint: "همگام‌سازی (فاز ۴)" },
];

/** راهنمای پلاسهولدرها — به‌صورت رشتهٔ ثابت تا براکت‌ها در JSX متن تعبیر نشوند */
const PLACEHOLDER_HELP =
  "پلاسهولدرهای مجاز: {name} نام ویدیو — {show} نام نمایشی سریال — {season} فصل " +
  "(دورقمی) — {episode} قسمت (دورقمی) — {parent} نام پوشه. پسوند «.fa.synced.srt» " +
  "همیشه حفظ می‌شود؛ اگر متادیتای یک پلاسهولدر برای ویدیویی نباشد (مثلاً قسمت برای فیلم)، " +
  "خروجی به‌صورت قطعی به نام خود ویدیو برمی‌گردد.";

/** نتیجهٔ بازخوانی وضعیت کلید — همان چیزی که واقعاً دیده شد، نه صرفاً «درخواست موفق بود».
 * خواندنِ موفقِ متناقض هم stored/missing برمی‌گرداند، نه unknown. */
type KeyStatusRefresh = { state: "stored" | "missing" | "unknown"; error: string | null };

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

  // ---- پروفایل‌ها — همهٔ کنش‌ها از مرز backend می‌گذرند (اعتبارسنجی و id آنجا است) ----

  const [profileBusy, setProfileBusy] = useState<string | null>(null);
  const [profileMessage, setProfileMessage] = useState<{ tone: "success" | "danger"; text: string } | null>(null);
  /** نام برای «ایجاد از مقادیر فعلی» و «تغییر نام» — یک input مشترک ساده */
  const [profileNameInput, setProfileNameInput] = useState("");

  const applyProfileMutation = async (profiles: ProcessingProfile[]) => {
    setProfileBusy("save");
    setProfileMessage(null);
    try {
      const saved = await api.saveProfiles(profiles);
      await reloadSettings();
      setDraft(structuredClone(saved));
      setProfileNameInput("");
      return saved;
    } catch (e) {
      setProfileMessage({ tone: "danger", text: `ذخیرهٔ پروفایل‌ها ناموفق بود: ${String(e)}` });
      return null;
    } finally {
      setProfileBusy(null);
    }
  };

  const activeProfile = draft.profiles.find((p) => p.id === draft.activeProfileId) ?? null;

  /** پروفایل انتخاب‌شده برای مدیریت — پیش‌فرض همان پروفایل فعال */
  const [selectedProfileId, setSelectedProfileId] = useState<string | null>(null);
  const selectedProfile =
    draft.profiles.find((p) => p.id === selectedProfileId) ??
    draft.profiles.find((p) => p.id === draft.activeProfileId) ??
    null;

  /** ایجاد پروفایل از مقادیر مؤثر فعلی — id خالی؛ backend توکن می‌سازد */
  const createProfileFromCurrent = async () => {
    const name = profileNameInput.trim();
    if (!name) {
      setProfileMessage({ tone: "danger", text: "برای پروفایل جدید یک نام بنویسید." });
      return;
    }
    await applyProfileMutation([
      ...draft.profiles,
      {
        id: "",
        name,
        builtIn: false,
        processing: structuredClone(draft.processing),
        automation: structuredClone(draft.automation),
        naming: structuredClone(draft.naming),
      },
    ]);
    setProfileMessage({ tone: "success", text: `پروفایل «${name}» از مقادیر فعلی ساخته شد.` });
  };

  const duplicateActiveProfile = async () => {
    if (!activeProfile) return;
    const name = profileNameInput.trim() || `${activeProfile.name} (کپی)`;
    await applyProfileMutation([
      ...draft.profiles,
      {
        ...structuredClone(activeProfile),
        id: "",
        name,
        builtIn: false,
      },
    ]);
    setProfileMessage({ tone: "success", text: `پروفایل «${name}» ساخته شد.` });
  };

  const renameProfile = async () => {
    if (!selectedProfile) return;
    const name = profileNameInput.trim();
    if (!name) {
      setProfileMessage({ tone: "danger", text: "برای تغییر نام، نام جدید را بنویسید." });
      return;
    }
    if (selectedProfile.builtIn) {
      setProfileMessage({ tone: "danger", text: "پروفایل پیش‌فرض قابل تغییر نام نیست." });
      return;
    }
    await applyProfileMutation(
      draft.profiles.map((p) => (p.id === selectedProfile.id ? { ...p, name } : p)),
    );
    setProfileMessage({ tone: "success", text: `نام پروفایل به «${name}» تغییر کرد.` });
  };

  const deleteSelectedProfile = async () => {
    if (!selectedProfile) return;
    if (selectedProfile.builtIn) {
      setProfileMessage({ tone: "danger", text: "پروفایل پیش‌فرض قابل حذف نیست — همیشه یک پیکربندی قابل استفاده باید بماند." });
      return;
    }
    const ok = await ask(
      `پروفایل «${selectedProfile.name}» حذف شود؟\n\nمقادیر مؤثر فعلی تغییر نمی‌کنند؛ فقط ورودی پروفایل حذف می‌شود.`,
      { kind: "warning" },
    );
    if (!ok) return;
    await applyProfileMutation(draft.profiles.filter((p) => p.id !== selectedProfile.id));
    setProfileMessage({ tone: "success", text: "پروفایل حذف شد." });
  };

  /** اعمال پروفایل انتخاب‌شده در dropdown — مقادیر مؤثر را همان‌جا کپی می‌کند */
  const applySelectedProfile = async () => {
    if (!selectedProfile) return;
    setProfileBusy("apply");
    setProfileMessage(null);
    try {
      const saved = await api.applyProfile(selectedProfile.id);
      await reloadSettings();
      setDraft(structuredClone(saved));
      setProfileMessage({
        tone: "success",
        text: `پروفایل «${selectedProfile.name}» روی مقادیر فعلی اعمال شد.`,
      });
    } catch (e) {
      setProfileMessage({ tone: "danger", text: `اعمال پروفایل ناموفق بود: ${String(e)}` });
    } finally {
      setProfileBusy(null);
    }
  };

  // ---- پیش‌نمایش قالب نام — رندر قطعی از backend، هرگز از فرانت ----

  const [namingPreview, setNamingPreview] = useState<NamingPreview | null>(null);
  const [namingPreviewError, setNamingPreviewError] = useState<string | null>(null);
  const PREVIEW_SAMPLE = {
    videoName: "Show.S01E05.1080p.mkv",
    show: "Show",
    season: 1,
    episode: 5,
  };

  useEffect(() => {
    let cancelled = false;
    const template = draft.naming.syncedSidecarBase;
    api
      .previewSidecarName({ template, ...PREVIEW_SAMPLE })
      .then((preview) => {
        if (!cancelled) {
          setNamingPreview(preview);
          setNamingPreviewError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setNamingPreview(null);
          setNamingPreviewError(String(e));
        }
      });
    return () => {
      cancelled = true;
    };
    // پیش‌نمایش فقط به قالب وابسته است — نمونه ثابت و نماینده است
  }, [draft.naming.syncedSidecarBase]);

  const toolStatus = (key: keyof ToolPathOverrides) => {
    const id = key === "sevenZip" ? "sevenZip" : key;
    return tools.find((t) => t.id === id)?.status ?? null;
  };

  // ---- کنش‌های API key — مستقل از save عمومی تنظیمات ----

  /** بازخوانی وضعیت کلید — خروجی همان وضعیت مشاهده‌شده است:
   * «stored»/«missing» پس از خواندن موفق (هرچه بود، حتی اگر با کنش قبلی متناقض باشد)،
   * «unknown» فقط پس از خطای خواندن. بج از همین نتیجه پیروی می‌کند. */
  const refreshKeyStatus = async (): Promise<KeyStatusRefresh> => {
    try {
      const stored = await api.hasSubsourceApiKey();
      const state = stored ? "stored" : "missing";
      setKeyStatus(state);
      return { state, error: null };
    } catch (e) {
      setKeyStatus("unknown");
      return { state: "unknown", error: String(e) };
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
      // نتیجهٔ خود ذخیره از بازخوانی وضعیت جدا گزارش می‌شود — حتی خواندنِ
      // موفقِ متناقض، پیام موفقیت بی‌قید نمی‌سازد.
      const refresh = await refreshKeyStatus();
      if (refresh.state === "stored") {
        setKeyMessage({ tone: "success", text: "API key در Credential Manager ذخیره شد." });
      } else if (refresh.state === "missing") {
        setKeyMessage({
          tone: "danger",
          text: "ذخیرهٔ API key انجام شد، اما بررسی بلافاصلهٔ وضعیت نشان می‌دهد کلید ذخیره نشده است — لطفاً دوباره تلاش کنید.",
        });
      } else {
        setKeyMessage({
          tone: "danger",
          text: `ذخیرهٔ API key انجام شد، اما خواندن مجدد وضعیت ناموفق بود — وضعیت کلید فعلاً نامشخص است: ${refresh.error ?? ""}`,
        });
      }
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
      const refresh = await refreshKeyStatus();
      if (refresh.state === "missing") {
        setKeyMessage({ tone: "success", text: "API key از Credential Manager حذف شد." });
      } else if (refresh.state === "stored") {
        setKeyMessage({
          tone: "danger",
          text: "حذف API key انجام شد، اما بررسی بلافاصلهٔ وضعیت همچنان کلید ذخیره‌شده‌ای می‌بیند — ظاهراً حذف کامل نشده است.",
        });
      } else {
        setKeyMessage({
          tone: "danger",
          text: `حذف API key انجام شد، اما خواندن مجدد وضعیت ناموفق بود — وضعیت کلید فعلاً نامشخص است: ${refresh.error ?? ""}`,
        });
      }
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
        <SectionTitle icon={KeyRound} title="SubSource API Key (اختیاری)" />
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-40 shrink-0 text-sm text-ink-2">وضعیت کلید</span>
            {keyStatus === "checking" ? (
              <Badge tone="warn">در حال بررسی…</Badge>
            ) : keyStatus === "stored" ? (
              <Badge tone="success">ذخیره‌شده در Credential Manager</Badge>
            ) : keyStatus === "missing" ? (
              <Badge tone="success">تنظیم نشده — دانلود ناشناس فعال است</Badge>
            ) : (
              <Badge tone="danger">نامشخص — خطا در خواندن وضعیت</Badge>
            )}
          </div>
          <p className="text-xs text-ink-3">
            کلید کاملاً اختیاری است — بدون آن هم دانلود زیرنویس به‌صورت ناشناس کار
            می‌کند و سیستم خودکار راه می‌افتد. کلید فقط برای حساب شخصی شما در
            SubSource است؛ اگر وارد کنید با همان حساب دانلود می‌شود. خود کلید هرگز
            نمایش داده نمی‌شود و در settings.json نوشته نمی‌شود.
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
          <div className="flex flex-wrap items-center gap-3">
            <label htmlFor="subtitle-output-mode" className="w-56 shrink-0 text-sm text-ink-2">
              نحوهٔ ذخیرهٔ زیرنویس
            </label>
            <select
              id="subtitle-output-mode"
              value={draft.processing.subtitleOutputMode}
              onChange={(e) =>
                patch({
                  processing: {
                    ...draft.processing,
                    subtitleOutputMode: e.target.value as "softSub" | "sidecar",
                  },
                })
              }
              className="h-10 min-w-52 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            >
              <option value="softSub">SoftSub — داخل فایل ویدیو (پیش‌فرض)</option>
              <option value="sidecar">فایل جدا کنار ویدیو (سازگار با تلویزیون‌ها)</option>
            </select>
          </div>
          <p className="text-xs leading-6 text-ink-3">
            در حالت SoftSub، زیرنویس فارسیِ سینک‌شده داخل ویدیو قرار می‌گیرد. حالت فایل جدا، رفتار قبلی برنامه را حفظ می‌کند.
          </p>
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

      <Card className="p-5">
        <SectionTitle icon={Layers} title="پروفایل‌های پردازش" />
        <div className="space-y-3">
          <p className="text-xs leading-6 text-ink-3">
            هر پروفایل مجموعهٔ «پردازش و سینک + اتوماسیون + نام‌گذاری خروجی» را نگه
            می‌دارد و با «اعمال» روی مقادیر فعلی کپی می‌شود. تنظیمات سراسری
            (ابزارها، پوشه‌ها، اعلان‌ها، کلید API و…) خارج از پروفایل‌اند.
          </p>
          <div className="flex flex-wrap items-center gap-2">
            <label htmlFor="profile-select" className="w-56 shrink-0 text-sm text-ink-2">
              پروفایل — انتخاب برای مدیریت
            </label>
            <select
              id="profile-select"
              value={selectedProfile?.id ?? ""}
              onChange={(e) => setSelectedProfileId(e.target.value)}
              className="h-10 min-w-52 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            >
              {draft.profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                  {p.id === draft.activeProfileId ? " — فعال" : ""}
                  {p.builtIn ? " (ساختی)" : ""}
                </option>
              ))}
            </select>
            <Button
              variant="primary"
              loading={profileBusy === "apply"}
              onClick={applySelectedProfile}
              disabled={profileBusy !== null || !selectedProfile}
            >
              اعمال روی مقادیر فعلی
            </Button>
          </div>
          {activeProfile && (
            <p className="text-xs leading-6 text-ink-3">
              پروفایل فعال:{" "}
              <span className="font-semibold text-ink-1">{activeProfile.name}</span>
              {" "}— ویرایش کارت‌های «پردازش و سینک»، «اتوماسیون» و «نام‌گذاری» مقادیر
              مؤثر را عوض می‌کند و پروفایل را بازنویسی نمی‌کند؛ برای به‌روزرسانی
              پروفایل از مقادیر فعلی، آن را با «ایجاد» یا «تکثیر» دوباره ذخیره کنید.
            </p>
          )}
          <div className="flex flex-wrap items-center gap-2">
            <input
              value={profileNameInput}
              onChange={(e) => setProfileNameInput(e.target.value)}
              placeholder="نام برای پروفایل جدید / نام جدید برای پروفایل انتخاب‌شده"
              className="h-10 min-w-56 flex-1 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            />
            <Button
              variant="primary"
              loading={profileBusy === "save"}
              onClick={createProfileFromCurrent}
              disabled={profileBusy !== null}
            >
              ایجاد از مقادیر فعلی
            </Button>
            <Button
              variant="ghost"
              onClick={duplicateActiveProfile}
              disabled={profileBusy !== null || !activeProfile}
            >
              تکثیر فعال
            </Button>
            <Button
              variant="ghost"
              onClick={renameProfile}
              disabled={profileBusy !== null || !selectedProfile || selectedProfile.builtIn}
            >
              تغییر نام
            </Button>
            <Button
              variant="danger"
              onClick={deleteSelectedProfile}
              disabled={
                profileBusy !== null || !selectedProfile || selectedProfile.builtIn
              }
            >
              حذف
            </Button>
          </div>
          {profileMessage && (
            <p
              className={`rounded-lg border px-3 py-2 text-xs leading-5 ${
                profileMessage.tone === "success"
                  ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
                  : "border-red-500/30 bg-red-500/10 text-red-400"
              }`}
            >
              {profileMessage.text}
            </p>
          )}
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle icon={Zap} title="اتوماسیون" />
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-3">
            <label htmlFor="automation-mode" className="w-56 shrink-0 text-sm text-ink-2">
              حالت پردازش
            </label>
            <select
              id="automation-mode"
              value={draft.automation.mode}
              onChange={(e) =>
                patch({
                  automation: {
                    ...draft.automation,
                    mode: e.target.value as AutomationMode,
                  },
                })
              }
              className="h-10 min-w-52 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            >
              <option value="interactive">تعاملی — هر ابهام پرسیده می‌شود (پیش‌فرض)</option>
              <option value="auto">خودکار — بدون توقف؛ نامطمئن‌ها «نیازمند بررسی»</option>
            </select>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-56 shrink-0 text-sm text-ink-2">
              حداکثر تلاش سینک خودکار (هر ویدیو)
            </span>
            <input
              type="number"
              min={1}
              max={4}
              value={draft.automation.maxSyncAttempts}
              onChange={(e) =>
                patch({
                  automation: {
                    ...draft.automation,
                    maxSyncAttempts: Number(e.target.value) || 1,
                  },
                })
              }
              className="h-10 w-24 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            />
          </div>
          <p className="text-xs leading-6 text-ink-3">
            در حالت خودکار، انتخاب Track/آرشیو/زیرنویس فقط وقتی با امتیازدهی محتوا و با اطمینان
            کافی انجام می‌شود؛ بقیه به «نیازمند بررسی» می‌روند. گیت کیفیت ۳۰ ثانیه‌ای سینک در
            هر دو حالت پابرجاست و گزارش JSON هر اجرا در پوشهٔ دادهٔ برنامه (data/runs) ذخیره
            می‌شود.
          </p>
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle icon={FileText} title="نام‌گذاری خروجی" />
        <div className="space-y-3">
          <div className="flex flex-wrap items-center gap-3">
            <label htmlFor="naming-template" className="w-56 shrink-0 text-sm text-ink-2">
              قالب پایهٔ نام خروجی سینک‌شده
            </label>
            <input
              id="naming-template"
              dir="ltr"
              value={draft.naming.syncedSidecarBase}
              onChange={(e) =>
                patch({
                  naming: { ...draft.naming, syncedSidecarBase: e.target.value },
                })
              }
              className="h-10 min-w-64 flex-1 rounded-lg border border-line bg-surface-2 px-3 font-mono text-sm outline-none focus:border-accent"
            />
            <Button
              variant="ghost"
              onClick={() =>
                patch({
                  naming: { ...draft.naming, syncedSidecarBase: "{name}" },
                })
              }
            >
              بازگشت به پیش‌فرض
            </Button>
          </div>
          <p className="text-xs leading-6 text-ink-3">{PLACEHOLDER_HELP}</p>
          {namingPreviewError ? (
            <p className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-400">
              {namingPreviewError}
            </p>
          ) : namingPreview ? (
            <div className="rounded-lg border border-line bg-surface-2 px-3 py-2 text-xs">
              <span className="text-ink-3">
                پیش‌نمایش برای نمونهٔ نماینده (Show.S01E05.1080p.mkv):
              </span>{" "}
              <span dir="ltr" className="font-mono text-ink-1">
                {namingPreview.fileName}
              </span>
              {namingPreview.fellBack && (
                <span className="text-amber-400">
                  {" "}— متادیتای نمونه برای این قالب کافی نبود؛ fallback پیش‌فرض
                </span>
              )}
            </div>
          ) : null}
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle icon={Bell} title="اعلان‌ها" />
        <div className="space-y-3">
          <p className="text-xs leading-6 text-ink-3">
            وقتی یک تسک طولانی تمام می‌شود (سینک، هاردساب، افزودن زیرنویس چسبیده)
            یک اعلان سیستمی ویندوز می‌آید — حتی وقتی پنجره مینیمایز است.
          </p>
          <Toggle
            checked={draft.notifications.enabled}
            onChange={(v) => patch({ notifications: { ...draft.notifications, enabled: v } })}
            label="اعلان‌های سیستمی"
            hint="کلید اصلی همهٔ اعلان‌ها (پیش‌فرض: فعال)"
          />
          <Toggle
            checked={draft.notifications.pipelineDone}
            onChange={(v) => patch({ notifications: { ...draft.notifications, pipelineDone: v } })}
            label="پایان سینک"
            hint="اعلان پس از پایان یا توقف pipeline"
          />
          <Toggle
            checked={draft.notifications.hardsubDone}
            onChange={(v) => patch({ notifications: { ...draft.notifications, hardsubDone: v } })}
            label="پایان هاردساب"
            hint="اعلان پس از پایان حک کردن زیرنویس داخل تصویر"
          />
          <Toggle
            checked={draft.notifications.softsubDone}
            onChange={(v) => patch({ notifications: { ...draft.notifications, softsubDone: v } })}
            label="پایان افزودن زیرنویس چسبیده"
            hint="اعلان پس از دانلود و جانمایی زیرنویس چسبیده"
          />
        </div>
      </Card>

      <Card className="p-5">
        <SectionTitle icon={Film} title="هاردساب" />
        <div className="space-y-3">
          <p className="text-xs leading-6 text-ink-3">
            هاردساب یعنی حک کردن زیرنویس داخل تصویر ویدیو — به‌صورت اختیاری پس از
            پایان سینک، از پنجرهٔ نتیجه اجرا می‌شود. خروجی همیشه فایل جدید کنار
            ویدیو است (<span dir="ltr" className="font-mono">&lt;نام&gt;.fa.hardsubbed</span>) و
            فایل اصلی دست‌نخورده می‌ماند.
          </p>
          <div className="flex flex-wrap items-center gap-3">
            <label htmlFor="hardsub-encoder" className="w-56 shrink-0 text-sm text-ink-2">
              انکودر ویدیو
            </label>
            <select
              id="hardsub-encoder"
              value={draft.hardsub.encoder}
              onChange={(e) =>
                patch({
                  hardsub: {
                    ...draft.hardsub,
                    encoder: e.target.value as HardsubSettings["encoder"],
                  },
                })
              }
              className="h-10 min-w-52 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            >
              <option value="auto">خودکار — بهترین GPU موجود، در نبود آن CPU</option>
              <option value="nvidia">فقط NVIDIA NVENC</option>
              <option value="intel">فقط Intel QSV</option>
              <option value="amd">فقط AMD AMF</option>
              <option value="cpu">همیشه CPU</option>
            </select>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <label htmlFor="hardsub-quality" className="w-56 shrink-0 text-sm text-ink-2">
              کیفیت انکود
            </label>
            <select
              id="hardsub-quality"
              value={draft.hardsub.quality}
              onChange={(e) =>
                patch({
                  hardsub: {
                    ...draft.hardsub,
                    quality: e.target.value as HardsubSettings["quality"],
                  },
                })
              }
              className="h-10 min-w-52 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            >
              <option value="fast">سریع — سرعت اول (پیش‌فرض)</option>
              <option value="balanced">متعادل</option>
              <option value="high">باکیفیت — کندتر</option>
            </select>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <span className="w-56 shrink-0 text-sm text-ink-2">تعداد Workerهای هم‌زمان هاردساب</span>
            <input
              type="number"
              min={1}
              max={4}
              value={draft.hardsub.workers}
              onChange={(e) =>
                patch({
                  hardsub: {
                    ...draft.hardsub,
                    workers: Number(e.target.value) || 1,
                  },
                })
              }
              className="h-10 w-24 rounded-lg border border-line bg-surface-2 px-3 text-sm outline-none focus:border-accent"
            />
          </div>
          <p className="text-xs leading-6 text-ink-3">
            انکودر GPU چند برابر CPU سرعت دارد؛ در حالت خودکار با یک انکود آزمایشی
            کوتاه بهترین گزینه انتخاب و در دیالوگ هاردساب نمایش داده می‌شود.
          </p>
        </div>
      </Card>
    </div>
  );
}
