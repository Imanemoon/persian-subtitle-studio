import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  ExtractionPlan,
  ExtractionPlanItem,
  LogEntry,
  ScanSummary,
  Settings,
  SubtitleValidation,
  ToolInfo,
  ToolStatus,
  TrackReview,
  VideoEntry,
} from "../types";
import type { PipelineItemSnapshot, PipelineSnapshot } from "../types/pipeline";
import type { HardsubPlan, HardsubSnapshot } from "../types/hardsub";

/** وضعیت یک ابزار در معاونت راه‌اندازی — قرینه SetupToolStatus بک‌اند. */
export interface SetupToolStatus {
  id: string;
  label: string;
  requiredForEntry: boolean;
  sizeHint: string;
  status: ToolStatus;
}

/** پیام پیشرفت نصب — رویداد setup:progress؛ اعداد از واقعیت بک‌اند. */
export interface SetupProgressPayload {
  toolId: string;
  phase: string;
  receivedBytes: number;
  totalBytes: number;
  message: string;
}

export const api = {
  getLibrary: () => invoke<VideoEntry[]>("get_library"),
  getLastScan: () => invoke<ScanSummary | null>("get_last_scan"),
  isScanning: () => invoke<boolean>("is_scanning"),
  startScan: (root: string) => invoke<void>("start_scan", { root }),
  stopScan: () => invoke<boolean>("stop_scan"),

  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invoke<Settings>("save_settings", { settings }),

  // ---- مدیریت امن SubSource API key — فقط Credential Manager؛
  // مقدار واقعی کلید هرگز به فرانت برگردانده نمی‌شود، فقط وضعیت boolean ----
  setSubsourceApiKey: (apiKey: string) =>
    invoke<void>("set_subsource_api_key", { apiKey }),
  hasSubsourceApiKey: () => invoke<boolean>("has_subsource_api_key"),
  deleteSubsourceApiKey: () => invoke<void>("delete_subsource_api_key"),

  detectTools: () => invoke<ToolInfo[]>("detect_tools"),
  getLogs: (limit = 300) => invoke<LogEntry[]>("get_logs", { limit }),

  // ---- معاونت راه‌اندازی: نصب خودکار پیش‌نیازها + شتاب Defender + میان‌بر ----
  setupStatus: () => invoke<SetupToolStatus[]>("setup_status"),
  installTool: (id: string) => invoke<void>("install_tool", { id }),
  cancelSetup: () => invoke<void>("cancel_setup"),
  isElevated: () => invoke<boolean>("is_elevated"),
  addDefenderExclusion: (paths: string[]) =>
    invoke<string>("add_defender_exclusion", { paths }),
  createAdminShortcut: () => invoke<string>("create_admin_shortcut"),
  getAppDirHints: () =>
    invoke<{ installDir: string; toolsDir: string }>("get_app_dir_hints"),

  // ---- فاز ۲: تحلیل و استخراج زیرنویس داخلی ----
  analyzeVideo: (path: string) => invoke<VideoEntry>("analyze_video", { path }),
  analyzeAll: () => invoke<number>("analyze_all"),
  extractTrack: (path: string, streamIndex: number) =>
    invoke<VideoEntry>("extract_track", { path, streamIndex }),
  extractTrackForReview: (path: string, streamIndex: number) =>
    invoke<TrackReview>("extract_track_for_review", { path, streamIndex }),
  promoteReview: (path: string, streamIndex: number) =>
    invoke<VideoEntry>("promote_review", { path, streamIndex }),
  planExtractionAll: () => invoke<ExtractionPlan>("plan_extraction_all"),
  executeExtractionPlan: (items: ExtractionPlanItem[]) =>
    invoke<number>("execute_extraction_plan", { items }),
  cancelSubtitleOps: () => invoke<boolean>("cancel_subtitle_ops"),
  validateSubtitleFile: (path: string) =>
    invoke<SubtitleValidation>("validate_subtitle_file", { path }),

  pickFolder: async (): Promise<string | null> => {
    const picked = await open({ directory: true, multiple: false });
    return typeof picked === "string" ? picked : null;
  },

  /** انتخاب چند فایل ویدیویی با دیالوگ بومی ویندوز — فیلترها مطابق Scanner */
  pickVideos: async (): Promise<string[] | null> => {
    const picked = await open({
      multiple: true,
      filters: [
        {
          name: "Video",
          extensions: [
            "mkv", "mp4", "m4v", "mov", "avi", "ts", "m2ts", "mts",
            "webm", "mpg", "mpeg", "vob", "wmv", "flv",
          ],
        },
      ],
    });
    if (!picked) return null; // لغو دیالوگ — انتخاب فعلی دست‌نخورده می‌ماند
    return Array.isArray(picked) ? picked : [picked];
  },

  pickExecutable: async (): Promise<string | null> => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Executable", extensions: ["exe"] }],
    });
    return typeof picked === "string" ? picked : null;
  },

  // ---- Pipeline انتها-به-انتها — فرانت فقط شناسه‌های مات بک‌اند را می‌فرستد ----
  startPipeline: (paths: string[]) =>
    invoke<PipelineSnapshot>("start_pipeline", {
      input: { kind: "files", paths },
    }),
  resolvePipelineDecision: (decisionId: string, optionId: string) =>
    invoke<void>("resolve_pipeline_decision", { decisionId, optionId }),
  cancelPipeline: () => invoke<boolean>("cancel_pipeline"),
  getPipelineState: () =>
    invoke<PipelineSnapshot | null>("get_pipeline_state"),
  getPipelineItems: () =>
    invoke<PipelineItemSnapshot[]>("get_pipeline_items"),
  isPipelineActive: () => invoke<boolean>("is_pipeline_active"),

  // ---- هاردساب — عملیات اختیاری پس از سینک؛ خروجی فایل جدید کنار ویدیو ----
  planHardsub: (paths: string[]) =>
    invoke<HardsubPlan>("plan_hardsub", { paths }),
  startHardsub: (paths: string[]) =>
    invoke<HardsubSnapshot>("start_hardsub", { paths }),
  cancelHardsub: () => invoke<boolean>("cancel_hardsub"),
  getHardsubState: () => invoke<HardsubSnapshot | null>("get_hardsub_state"),
  isHardsubActive: () => invoke<boolean>("is_hardsub_active"),

  // ---- افزودن زیرنویس چسبیده برای ویدیوهای هاردساب‌دار ----
  // هر دو مسیر خروجی <stem>.fa.srt هم‌نام ویدیو می‌سازند و هرگز بازنویسی نمی‌کنند
  placeLocalSubtitle: (videoPath: string, subtitlePath: string) =>
    invoke<string>("place_local_subtitle", { videoPath, subtitlePath }),
  downloadSoftsubBatch: (paths: string[]) =>
    invoke<number>("download_softsub_batch", { paths }),
  /** تنظیم دوباره: حذف زیرنویس فعلی + دانلود تازه + سینک خودکار از اول */
  resetSubtitle: (videoPath: string) =>
    invoke<void>("reset_subtitle", { videoPath }),
  /** پخش ویدیو با پخش‌کنندهٔ پیش‌فرض ویندوز — برای قضاوت سریع سینک */
  playVideo: (videoPath: string) => invoke<void>("play_video", { path: videoPath }),
  /** باز کردن پوشهٔ محل ذخیره در Explorer با انتخاب فایل */
  revealInStorage: (videoPath: string) =>
    invoke<void>("reveal_in_explorer", { path: videoPath }),
  /** استخراج زیرنویس سینک‌شدهٔ نهایی به <base>.fa.srt — جایگزین زیرنویس لوکال */
  extractSyncedSubtitle: (videoPath: string) =>
    invoke<string>("extract_synced_subtitle", { path: videoPath }),

  /** انتخاب یک فایل زیرنویس (srt/ass/ssa/vtt) با دیالوگ بومی */
  pickSubtitle: async (): Promise<string | null> => {
    const picked = await open({
      multiple: false,
      filters: [
        { name: "Subtitle", extensions: ["srt", "ass", "ssa", "vtt"] },
      ],
    });
    return typeof picked === "string" ? picked : null;
  },
};
