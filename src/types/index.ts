// انواع داده مشترک بین بک‌اند Rust و فرانت‌اند — قرینه serde structs

export interface SubtitleFileInfo {
  path: string;
  fileName: string;
  ext: string;
  sizeBytes: number;
  nameSuggestsFa: boolean;
}

export interface VideoEntry {
  id: string;
  path: string;
  fileName: string;
  sizeBytes: number;
  modifiedMs: number | null;
  parentDir: string;
  showName: string | null;
  season: number | null;
  episode: number | null;
  siblingSubtitles: SubtitleFileInfo[];
}

export interface ScanProgress {
  dirsScanned: number;
  filesSeen: number;
  videosFound: number;
  currentPath: string;
}

export interface ScanSummary {
  root: string;
  dirsScanned: number;
  filesSeen: number;
  videoCount: number;
  excludedDirHits: number;
  parseFailed: number;
  withFaSidecar: number;
  withAnySidecar: number;
  durationMs: number;
  cancelled: boolean;
}

export type ToolStatus = "found" | "missing" | "broken";

export interface ToolInfo {
  id: string;
  name: string;
  status: ToolStatus;
  path: string | null;
  version: string | null;
  detail: string | null;
}

export type LogLevel = "debug" | "info" | "warn" | "error";

export interface LogEntry {
  id: number;
  timestamp: string;
  level: LogLevel;
  source: string;
  message: string;
}

export interface ToolPathOverrides {
  ffmpeg: string | null;
  ffprobe: string | null;
  subsource: string | null;
  sevenZip: string | null;
  ffsubsync: string | null;
}

export interface ProcessingSettings {
  syncWorkers: number;
  multiSegment: boolean;
  segmentCount: number;
  skipIntroOutro: boolean;
  skipSyncOnLowQuality: boolean;
}

export type ExistingFileBehavior = "neverOverwrite";

export interface Settings {
  rootFolder: string | null;
  excludedFolders: string[];
  toolPaths: ToolPathOverrides;
  processing: ProcessingSettings;
  existingFileBehavior: ExistingFileBehavior;
  logsDir: string | null;
}

export type PageId = "dashboard" | "library" | "processing" | "logs" | "settings";
