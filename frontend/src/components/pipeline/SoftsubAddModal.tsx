import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  AlertTriangle,
  CheckCircle2,
  Download,
  FilePlus2,
  Loader2,
} from "lucide-react";
import { Button, Modal } from "../ui";
import { api } from "../../lib/api";
import { fileBaseName } from "../../lib/videoFiles";
import { faNum } from "../../lib/format";
import type { OpsProgress } from "../../types";

/** kind رویدادهای بک‌اند — قرینهٔ SOFTSUB_ADD_EVENT_KIND */
const KIND = "softsub-add";

/**
 * افزودن زیرنویس نرم برای ویدیوهای هاردساب‌دار — دانلود از SubSource یا
 * فایل محلی؛ هر دو خروجی را هم‌نام ویدیو (`<stem>.fa.srt`) کنارش می‌گذارند
 * و هرگز بازنویسی نمی‌کنند. پس از جانمایی، اجرای مجدد سینک آن را هم‌تراز
 * و (در حالت پیش‌فرض) SoftSub می‌کند.
 */
export function SoftsubAddModal({
  open,
  paths,
  onClose,
}: {
  open: boolean;
  paths: string[];
  onClose: () => void;
}) {
  const batch = paths.length > 1;
  const [running, setRunning] = useState(false);
  const [busyLocal, setBusyLocal] = useState(false);
  const [statuses, setStatuses] = useState<Record<string, string>>({});
  const [summary, setSummary] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // رویدادهای پیشرفت بک‌اند — فقط وقتی مودال باز است
  useEffect(() => {
    if (!open) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    (async () => {
      const un1 = await listen<OpsProgress>("ops:progress", (event) => {
        const p = event.payload;
        if (p.kind !== KIND || !p.videoPath) return;
        setStatuses((prev) => ({ ...prev, [p.videoPath]: p.message }));
      });
      const un2 = await listen<OpsProgress>("ops:done", (event) => {
        const p = event.payload;
        if (p.kind !== KIND) return;
        setRunning(false);
        setSummary(p.message);
      });
      if (disposed) {
        un1();
        un2();
      } else {
        unlisten = () => {
          un1();
          un2();
        };
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [open]);

  // باز شدن مودال تازه = وضعیت تازه
  useEffect(() => {
    if (open) {
      setRunning(false);
      setBusyLocal(false);
      setStatuses({});
      setSummary(null);
      setError(null);
    }
  }, [open, paths]);

  const startDownload = async () => {
    setError(null);
    setSummary(null);
    setRunning(true);
    try {
      await api.downloadSoftsubBatch(paths);
      // پایان از رویداد ops:done می‌آید
    } catch (e) {
      setRunning(false);
      setError(String(e));
    }
  };

  const pickLocal = async (videoPath: string) => {
    setError(null);
    setBusyLocal(true);
    try {
      const sub = await api.pickSubtitle();
      if (!sub) {
        setBusyLocal(false);
        return;
      }
      const placed = await api.placeLocalSubtitle(videoPath, sub);
      setStatuses((prev) => ({
        ...prev,
        [videoPath]: `انجام شد: ${fileBaseName(placed)}`,
      }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusyLocal(false);
    }
  };

  const doneCount = paths.filter((p) =>
    (statuses[p] ?? "").startsWith("انجام شد"),
  ).length;
  const [listExpanded, setListExpanded] = useState(false);

  return (
    <Modal
      open={open}
      title="افزودن زیرنویس نرم به ویدیوی هاردساب‌دار"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose} disabled={busyLocal}>
            {running ? "ادامه در پس‌زمینه" : "بستن"}
          </Button>
          {!running && (
            <Button variant="primary" onClick={() => void startDownload()}>
              <Download className="size-3.5" />
              دانلود از SubSource ({faNum(paths.length)} مورد)
            </Button>
          )}
        </>
      }
    >
      <div className="space-y-3 text-sm">
        <p className="text-xs leading-6 text-ink-2">
          زیرنویس این ویدیوها داخل تصویر حک شده است. با افزودن زیرنویس نرم، فایل
          <span dir="ltr" className="mx-1 font-mono">
            &lt;نام ویدیو&gt;.fa.srt
          </span>
          دقیقاً هم‌نام کنار ویدیو قرار می‌گیرد؛ فایل اصلی تغییر نمی‌کند و هیچ
          چیز بازنویسی نمی‌شود. برای هم‌ترازسازی و SoftSub شدن، بعد از آن سینک
          را دوباره اجرا کنید.
        </p>

        {!running && (
          <div className="flex flex-wrap gap-2">
            <Button
              variant="primary"
              disabled={busyLocal}
              loading={busyLocal}
              onClick={() => void pickLocal(paths[0])}
            >
              <FilePlus2 className="size-3.5" />
              از فایل محلی…
            </Button>
            <Button
              variant="primary"
              disabled={busyLocal}
              loading={running}
              onClick={() => void startDownload()}
            >
              <Download className="size-3.5" />
              دانلود از SubSource
            </Button>
          </div>
        )}

        {running && (
          <div className="flex items-center gap-2 rounded-lg border border-line bg-surface-2/60 px-3 py-2.5 text-xs text-ink-2">
            <Loader2 className="size-4 shrink-0 animate-spin text-sky-400" />
            دانلود در جریان است — هر ویدیو جدا دانلود و جانمایی می‌شود. می‌توانید
            پنجره را با «ادامه در پس‌زمینه» ببندید؛ کار متوقف نمی‌شود و پایان با
            اعلان گزارش می‌شود.
          </div>
        )}

        {error && (
          <p className="flex items-start gap-1.5 text-xs leading-5 text-red-300">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            {error}
          </p>
        )}
        {summary && (
          <p className="flex items-start gap-1.5 text-xs leading-5 text-ink-2">
            <CheckCircle2 className="mt-0.5 size-3.5 shrink-0 text-emerald-400" />
            {summary}
          </p>
        )}
        {doneCount > 0 && (
          <p className="text-xs text-emerald-300/90">
            {faNum(doneCount)} مورد زیرنویس نرم گرفت.
          </p>
        )}

        <div className="max-h-56 space-y-1 overflow-y-auto">
          {(listExpanded ? paths : paths.slice(0, 5)).map((path) => {
            const status = statuses[path];
            const done = (status ?? "").startsWith("انجام شد");
            const failed = (status ?? "").startsWith("ناموفق");
            return (
              <div
                key={path}
                className="rounded-lg border border-line bg-surface-2/60 px-3 py-2"
              >
                <div dir="ltr" className="truncate font-mono text-xs text-ink-1">
                  {fileBaseName(path)}
                </div>
                {status && (
                  <div
                    className={`mt-0.5 text-[11px] leading-5 ${
                      done
                        ? "text-emerald-300/90"
                        : failed
                          ? "text-red-300"
                          : "text-ink-3"
                    }`}
                  >
                    {status}
                  </div>
                )}
              </div>
            );
          })}
        </div>
        {paths.length > 5 && (
          <button
            onClick={() => setListExpanded((prev) => !prev)}
            className="w-full text-center text-xs text-ink-3 transition-colors hover:text-ink-1"
          >
            {listExpanded
              ? "نمایش کمتر"
              : `نمایش همهٔ ${faNum(paths.length)} مورد`}
          </button>
        )}
        {batch && (
          <p className="text-[11px] leading-5 text-ink-3">
            گزینهٔ فایل محلی برای حالت تک‌ویدیو است؛ برای حالت گروهی از دانلود
            استفاده کنید.
          </p>
        )}
      </div>
    </Modal>
  );
}
