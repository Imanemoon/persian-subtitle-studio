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
  TrackReview,
  VideoEntry,
} from "../types";

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
};
