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
  analysis: VideoAnalysis | null;
}

export type SubtitleKind = "text" | "image" | "unknown";

export interface SubtitleStream {
  streamIndex: number;
  codecName: string;
  kind: SubtitleKind;
  language: string | null;
  languageIetf: string | null;
  title: string | null;
  defaultDisposition: boolean;
  forcedDisposition: boolean;
  persianCandidate: boolean;
  /** محتوا با استخراج آزمایشی به‌عنوان فارسی تأیید شده؟ */
  persianConfirmed: boolean;
}

export type AnalysisStatus =
  | "notAnalyzed"
  | "analyzing"
  | "persianTextTrackFound"
  | "imageSubtitleFound"
  | "noEmbeddedSubtitle"
  | "noPersianTrack"
  | "extracting"
  | "extracted"
  | "invalidSubtitle"
  | "needsTrackSelection"
  | "failed";

export interface SubtitleValidation {
  valid: boolean;
  sizeBytes: number;
  encoding: string;
  lineCount: number;
  cueCount: number;
  hasPersian: boolean;
  persianCharCount: number;
  error: string | null;
}

export interface ExtractionOutcome {
  streamIndex: number;
  targetPath: string;
  validation: SubtitleValidation;
  persianConfirmed: boolean;
}

export interface VideoAnalysis {
  status: AnalysisStatus;
  analyzedAtMs: number;
  streams: SubtitleStream[];
  message: string | null;
  lastExtraction: ExtractionOutcome | null;
  reviews: TrackReview[];
}

/** نتیجه «استخراج آزمایشی برای بررسی» یک Track متنی */
export interface TrackReview {
  streamIndex: number;
  reviewPath: string;
  validation: SubtitleValidation;
  persianConfirmed: boolean;
  promoted: boolean;
}

export interface ExtractionPlanItem {
  videoPath: string;
  videoFileName: string;
  streamIndex: number;
  trackLabel: string;
  targetPath: string;
}

export interface SkippedExtraction {
  videoPath: string;
  videoFileName: string;
  reason: string;
}

export interface ExtractionPlan {
  items: ExtractionPlanItem[];
  skipped: SkippedExtraction[];
}

export interface OpsProgress {
  kind: string;
  current: number;
  total: number;
  videoPath: string;
  message: string;
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
  subtitleOutputMode: "softSub" | "sidecar";
}

/** تنظیمات اعلان‌های سیستمی پایان تسک‌ها — قرینه NotificationSettings بک‌اند */
export interface NotificationSettings {
  enabled: boolean;
  pipelineDone: boolean;
  hardsubDone: boolean;
  softsubDone: boolean;
}

/** تنظیمات هاردساب — قرینه HardsubSettings بک‌اند */
export type HardsubEncoder = "auto" | "nvidia" | "intel" | "amd" | "cpu";
export type HardsubQuality = "fast" | "balanced" | "high";

export interface HardsubSettings {
  encoder: HardsubEncoder;
  quality: HardsubQuality;
  workers: number;
}

export type ExistingFileBehavior = "neverOverwrite";

export interface Settings {
  rootFolder: string | null;
  excludedFolders: string[];
  toolPaths: ToolPathOverrides;
  processing: ProcessingSettings;
  hardsub: HardsubSettings;
  notifications: NotificationSettings;
  existingFileBehavior: ExistingFileBehavior;
  logsDir: string | null;
  /** معاونت راه‌اندازی یک‌بار طی شده یا پشت زده شده */
  setupCompleted: boolean;
}

export type PageId = "home" | "setup" | "settings";

/** انتخاب کاربر در صفحهٔ خانه — پوشه یا چند فایل، هر دو با هم نه */
export type HomeSelection =
  | { kind: "folder"; path: string }
  | { kind: "files"; paths: string[] };
