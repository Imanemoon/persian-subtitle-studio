import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { LogEntry, ScanSummary, Settings, ToolInfo, VideoEntry } from "../types";

export const api = {
  getLibrary: () => invoke<VideoEntry[]>("get_library"),
  getLastScan: () => invoke<ScanSummary | null>("get_last_scan"),
  isScanning: () => invoke<boolean>("is_scanning"),
  startScan: (root: string) => invoke<void>("start_scan", { root }),
  stopScan: () => invoke<boolean>("stop_scan"),

  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) =>
    invoke<Settings>("save_settings", { settings }),

  detectTools: () => invoke<ToolInfo[]>("detect_tools"),
  getLogs: (limit = 300) => invoke<LogEntry[]>("get_logs", { limit }),

  pickFolder: async (): Promise<string | null> => {
    const picked = await open({ directory: true, multiple: false });
    return typeof picked === "string" ? picked : null;
  },

  pickExecutable: async (): Promise<string | null> => {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Executable", extensions: ["exe"] }],
    });
    return typeof picked === "string" ? picked : null;
  },
};
