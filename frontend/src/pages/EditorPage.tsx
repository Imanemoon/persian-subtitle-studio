import { useEffect, useMemo, useState } from "react";
import {
  AlertTriangle,
  Check,
  FileText,
  Loader2,
  Plus,
  Save,
  Search,
  Trash2,
} from "lucide-react";
import { api } from "../lib/api";
import { Button } from "../components/ui";
import type { SubtitleCue, SubtitleEncodingReport } from "../types";

/**
 * ویرایشگر زیرنویس — فقط SRT؛ بک‌اند مالک parse/validate/save است.
 *
 * قراردادها:
 * - فایل اصلی هرگز ذخیره نمی‌شود؛ ذخیره به `<stem>.edited.srt` جدید می‌رود.
 * - فایل غیر UTF-8/16 ابتدا باید با «تبدیل encoding» تبدیل شود — همان
 *   دکمه در نوار بالا؛ فایل اصلی حفظ می‌شود و خروجی جدید می‌آید.
 * - خطاهای اعتبارسنجی (مدت نامعتبر/ترتیب واژگون) ذخیره را رد می‌کنند و
 *   همین‌جا نمایش داده می‌شوند؛ هشدارها (هم‌پوشانی/متن خالی) فقط گزارش‌اند.
 */

/** ms → HH:MM:SS,mmm — نمایش LTR با ارقام لاتین (فرمت SRT) */
function formatMs(totalMs: number): string {
  const clamped = Math.max(0, Math.floor(totalMs));
  const h = Math.floor(clamped / 3_600_000);
  const m = Math.floor((clamped % 3_600_000) / 60_000);
  const s = Math.floor((clamped % 60_000) / 1000);
  const ms = clamped % 1000;
  const pad = (v: number, n = 2) => String(v).padStart(n, "0");
  return `${pad(h)}:${pad(m)}:${pad(s)},${pad(ms, 3)}`;
}

/** HH:MM:SS,mmm → ms؛ ورودی نامعتبر → null */
function parseMs(input: string): number | null {
  const match = input.trim().match(/^(\d{1,2}):(\d{2}):(\d{2})[.,](\d{1,3})$/);
  if (!match) return null;
  const [, h, m, s, ms] = match;
  const fraction = Number(ms) * 10 ** (3 - ms.length);
  return ((Number(h) * 60 + Number(m)) * 60 + Number(s)) * 1000 + fraction;
}

/** input زمان برای یک cue — تغییر موفق → ms، لغو → مقدار قبلی */
function TimeInput({
  value,
  onChange,
  label,
}: {
  value: number;
  onChange: (ms: number) => void;
  label: string;
}) {
  const [text, setText] = useState(formatMs(value));
  useEffect(() => setText(formatMs(value)), [value]);
  const invalid = parseMs(text) === null;
  return (
    <label className="flex items-center gap-1 text-[11px] text-ink-3">
      {label}
      <input
        dir="ltr"
        value={text}
        onChange={(e) => {
          setText(e.target.value);
          const parsed = parseMs(e.target.value);
          if (parsed !== null) onChange(parsed);
        }}
        className={`w-28 rounded-md border bg-surface-2 px-2 py-1 font-mono text-xs text-ink-1 outline-none ${
          invalid ? "border-red-500/60" : "border-line focus:border-accent"
        }`}
      />
    </label>
  );
}

export function EditorPage() {
  const [path, setPath] = useState<string | null>(null);
  const [cues, setCues] = useState<SubtitleCue[] | null>(null);
  const [encoding, setEncoding] = useState<SubtitleEncodingReport | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [converting, setConverting] = useState(false);

  const pickAndLoad = async () => {
    setError(null);
    setNotice(null);
    const picked = await api.pickSubtitle();
    if (!picked) return;
    setLoading(true);
    setEncoding(null);
    try {
      const report = await api.analyzeSubtitleEncoding(picked);
      setEncoding(report);
      const result = await api.loadSubtitleForEdit(picked);
      setPath(result.path || picked);
      setCues(result.cues);
      setWarnings(result.warnings);
      setDirty(false);
    } catch (e) {
      setPath(picked);
      setCues(null);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const convertToUtf8 = async () => {
    if (!path) return;
    setConverting(true);
    setError(null);
    setNotice(null);
    try {
      const outcome = await api.convertSubtitleToUtf8(path);
      setNotice(
        `تبدیل شد (${outcome.fromLabel}) — فایل جدید: ${outcome.outputPath}؛ فایل اصلی دست‌نخورده ماند.`,
      );
      // فایل جدید را مستقیم باز می‌کنیم
      const report = await api.analyzeSubtitleEncoding(outcome.outputPath);
      setEncoding(report);
      const result = await api.loadSubtitleForEdit(outcome.outputPath);
      setPath(result.path || outcome.outputPath);
      setCues(result.cues);
      setWarnings(result.warnings);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setConverting(false);
    }
  };

  const updateCue = (index: number, patch: Partial<SubtitleCue>) => {
    setCues((prev) => {
      if (!prev) return prev;
      const next = [...prev];
      next[index] = { ...next[index], ...patch };
      return next;
    });
    setDirty(true);
  };

  const addCue = (afterIndex: number) => {
    setCues((prev) => {
      if (!prev) return prev;
      const anchor = prev[afterIndex];
      const startMs = anchor ? anchor.endMs + 100 : 1000;
      const next = [...prev];
      next.splice(afterIndex + 1, 0, {
        startMs,
        endMs: startMs + 2000,
        text: "",
      });
      return next;
    });
    setDirty(true);
  };

  const deleteCue = (index: number) => {
    setCues((prev) => {
      if (!prev) return prev;
      const next = prev.filter((_, i) => i !== index);
      return next;
    });
    setDirty(true);
  };

  // اعتبارسنجی زنده — رندر قطعی بک‌اند؛ خطاها در Err می‌آیند
  const [validationErrors, setValidationErrors] = useState<string[]>([]);
  const [validationWarnings, setValidationWarnings] = useState<string[]>([]);
  useEffect(() => {
    if (!cues) return;
    let cancelled = false;
    api
      .validateSubtitleCues(cues)
      .then((result) => {
        if (!cancelled) {
          setValidationErrors([]);
          setValidationWarnings(result.warnings);
        }
      })
      .catch((e) => {
        if (!cancelled) {
          setValidationErrors(String(e).split("\n").filter(Boolean));
          setValidationWarnings([]);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [cues]);

  const save = async () => {
    if (!path || !cues) return;
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const outcome = await api.saveSubtitleEdits(path, cues);
      setNotice(
        `ذخیره شد: ${outcome.outputPath} (${outcome.cueCount} cue) — فایل اصلی دست‌نخورده ماند.`,
      );
      setWarnings(outcome.warnings);
      setDirty(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const filteredIndices = useMemo(() => {
    if (!cues) return [];
    if (!query.trim()) return cues.map((_, i) => i);
    const q = query.trim();
    return cues
      .map((cue, i) => ({ cue, i }))
      .filter(({ cue }) => cue.text.includes(q) || formatMs(cue.startMs).includes(q))
      .map(({ i }) => i);
  }, [cues, query]);

  return (
    <div className="mx-auto max-w-4xl space-y-5 px-6 py-8">
      <div className="flex items-center justify-between">
        <h1 className="flex items-center gap-2 text-xl font-bold text-ink-1">
          <FileText className="size-5 text-accent" />
          ویرایشگر زیرنویس
        </h1>
        <Button variant="primary" onClick={() => void pickAndLoad()} disabled={loading}>
          {loading ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <FileText className="size-4" />
          )}
          بازکردن فایل SRT
        </Button>
      </div>

      {error && (
        <p className="flex items-start gap-2 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-400">
          <AlertTriangle className="mt-0.5 size-4 shrink-0" />
          <span className="whitespace-pre-line">{error}</span>
        </p>
      )}
      {notice && (
        <p className="flex items-start gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2 text-xs leading-5 text-emerald-400">
          <Check className="mt-0.5 size-4 shrink-0" />
          <span className="break-all">{notice}</span>
        </p>
      )}

      {path && (
        <p dir="ltr" className="truncate font-mono text-[11px] text-ink-3" title={path}>
          {path}
          {dirty && <span className="text-amber-400"> — تغییرات ذخیره‌نشده</span>}
        </p>
      )}

      {/* نوار encoding — تحلیل واقعی بک‌اند + تبدیل امن */}
      {encoding && (
        <div className="rounded-lg border border-line bg-surface-1 px-3 py-2 text-xs">
          <span className="text-ink-3">Encoding تشخیص‌داده‌شده:</span>{" "}
          <span className="font-semibold text-ink-1">{encoding.label}</span>
          {!encoding.confident && (
            <span className="text-amber-400"> — تشخیص نامطمئن؛ تبدیل انجام نمی‌شود</span>
          )}
          {encoding.kind === "utf8" && (
            <span className="text-ink-3"> — از قبل UTF-8 است؛ تبدیل لازم نیست</span>
          )}
          {encoding.convertible && (
            <Button
              variant="primary"
              className="ms-3 h-8 px-3 text-xs"
              loading={converting}
              onClick={() => void convertToUtf8()}
            >
              تبدیل به UTF-8 (فایل جدید)
            </Button>
          )}
          {encoding.detail && (
            <p className="mt-1 leading-5 text-ink-3">{encoding.detail}</p>
          )}
        </div>
      )}

      {cues && (
        <>
          {/* نوار جستجو + ذخیره */}
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative flex-1 min-w-56">
              <Search className="pointer-events-none absolute end-2 top-2 size-4 text-ink-3" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="جستجو در متن cueها…"
                className="h-10 w-full rounded-lg border border-line bg-surface-2 ps-3 pe-9 text-sm outline-none focus:border-accent"
              />
            </div>
            <Button
              variant="primary"
              loading={saving}
              onClick={() => void save()}
              disabled={validationErrors.length > 0}
              title={
                validationErrors.length > 0
                  ? "ابتدا خطاهای اعتبارسنجی را رفع کنید"
                  : "ذخیره در فایل جدید <name>.edited.srt"
              }
            >
              <Save className="size-4" />
              ذخیره (فایل جدید)
            </Button>
          </div>

          {validationErrors.length > 0 && (
            <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-400">
              <div className="font-semibold">خطاهای اعتبارسنجی — ذخیره رد می‌شود:</div>
              {validationErrors.map((e, i) => (
                <div key={i}>{e}</div>
              ))}
            </div>
          )}
          {(validationWarnings.length > 0 || warnings.length > 0) &&
            validationErrors.length === 0 && (
              <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs leading-5 text-amber-400">
                {[...warnings, ...validationWarnings].map((w, i) => (
                  <div key={i}>{w}</div>
                ))}
              </div>
            )}

          {/* لیست cueها */}
          <div className="space-y-3">
            {filteredIndices.length === 0 && (
              <p className="text-center text-sm text-ink-3">
                {query ? "چیزی مطابق جستجو پیدا نشد." : "فایل بدون cue است."}
              </p>
            )}
            {filteredIndices.map((index) => {
              const cue = cues![index];
              return (
                <div
                  key={index}
                  className="rounded-xl border border-line bg-surface-1 p-3"
                >
                  <div className="flex flex-wrap items-center gap-3">
                    <span className="rounded-md bg-accent-dim px-2 py-0.5 text-xs font-bold text-accent-strong">
                      {index + 1}
                    </span>
                    <TimeInput
                      label="شروع"
                      value={cue.startMs}
                      onChange={(ms) => updateCue(index, { startMs: ms })}
                    />
                    <TimeInput
                      label="پایان"
                      value={cue.endMs}
                      onChange={(ms) => updateCue(index, { endMs: ms })}
                    />
                    <span className="text-[11px] text-ink-3">
                      مدت: {Math.max(0, cue.endMs - cue.startMs)}ms
                    </span>
                    <div className="ms-auto flex items-center gap-1">
                      <Button
                        variant="ghost"
                        className="h-8 px-2"
                        title="افزودن cue بعد از این"
                        onClick={() => addCue(index)}
                      >
                        <Plus className="size-4" />
                      </Button>
                      <Button
                        variant="danger"
                        className="h-8 px-2"
                        title="حذف این cue"
                        onClick={() => deleteCue(index)}
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </div>
                  </div>
                  <textarea
                    dir="rtl"
                    value={cue.text}
                    onChange={(e) => updateCue(index, { text: e.target.value })}
                    rows={Math.max(2, cue.text.split("\n").length)}
                    className="mt-2 w-full rounded-lg border border-line bg-surface-2 px-3 py-2 text-sm leading-6 text-ink-1 outline-none focus:border-accent"
                  />
                </div>
              );
            })}
          </div>
        </>
      )}

      {!path && !loading && (
        <div className="rounded-xl border border-dashed border-line bg-surface-1/60 px-6 py-10 text-center">
          <p className="text-sm text-ink-2">
            یک فایل زیرنویس SRT باز کنید — متن، زمان‌ها و ترتیب cueها را ویرایش
            کنید و در یک فایل جدید ذخیره کنید.
          </p>
          <p className="mt-2 text-xs leading-5 text-ink-3">
            فایل cp1256 (ANSI فارسی) ابتدا از همین صفحه به UTF-8 تبدیل می‌شود؛
            فایل اصلی هرگز تغییر نمی‌کند.
          </p>
        </div>
      )}
    </div>
  );
}
