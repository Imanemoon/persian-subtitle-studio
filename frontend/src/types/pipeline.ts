// انواع Pipeline — قرینه دقیق serde structs بک‌اند (src-tauri/src/pipeline/types.rs).
// این قرارداد فقط از بک‌اند می‌آید؛ فرانت هیچ منطق pipeline ندارد و فقط
// snapshot/رویدادها را رندر و شناسه‌های مات را برمی‌گرداند.

/** مراحل pipeline — ترتیب barrier بک‌اند؛ هیچ درصدی وجود ندارد. */
export type PipelineStage =
  | "validating"
  | "scanning"
  | "evaluatingExisting"
  | "analyzing"
  | "extracting"
  | "downloading"
  | "planningArchives"
  | "extractingArchives"
  | "selectingSyncInput"
  | "syncing"
  | "finalizing"
  | "done";

/** وضعیت کلی run */
export type PipelineStatus =
  | "running"
  | "awaitingDecision"
  | "completed"
  | "cancelled"
  | "failed";

/** وضعیت مرحلهٔ جاری یک ویدیو — اطلاعاتی؛ نتیجهٔ نهایی outcome است نه این. */
export type StageStatus =
  | { kind: "pending" }
  | { kind: "inProgress" }
  | { kind: "skipped"; reason: string }
  | { kind: "failed"; reason: string }
  | { kind: "needsDecision"; decisionId: string }
  | { kind: "done" };

/** وضعیت پایانی یک ویدیو — فقط یک‌بار ست می‌شود. */
export type ItemOutcome =
  | { kind: "alreadySynced" }
  | { kind: "outputConflict"; preservedPath: string }
  | { kind: "synced"; outputPath: string }
  | { kind: "softSubbed"; videoPath: string }
  /** زیرنویس فارسی داخل تصویر حک شده (قرارداد نام‌گذاری گروه‌های رلیز) */
  | { kind: "hardsubbed" }
  | { kind: "unchangedLowQuality"; reason?: string }
  /** سیاست خودکار اطمینان کافی نداشت — ورودی دست‌نخورده ماند؛ دلیل صادقانه */
  | { kind: "needsReview"; reason: string }
  | { kind: "needsApiKey" }
  | { kind: "keyStoreUnavailable"; reason: string }
  | { kind: "skipped"; reason: string }
  | { kind: "failed"; reason: string }
  | { kind: "cancelled" };

/** تعارض sidecar حفظ‌شده — قرینه subsource::plan::SidecarConflict */
export interface SidecarConflict {
  path: string;
  reason: string;
}

/** رکورد یک تلاش سینک — قرینه sync::SyncAttemptRecord */
export interface SyncAttemptRecord {
  strategy: string;
  status: string;
  offsetSeconds?: number;
  accepted: boolean;
}

/** تصمیم خودکار سیاست در حالت Auto — قرینه pipeline::AutoDecisionRecord */
export interface AutoDecisionRecord {
  kind: DecisionKind;
  chosen: string;
  reasons: string[];
}

/** داده‌های قابل گزارش هر ویدیو — در JSON به‌صورت flatten داخل آیتم می‌آید. */
export interface ItemArtifacts {
  syncInputPath?: string;
  placedPath?: string;
  conflictPath?: string;
  syncCandidatePath?: string;
  stagedPaths?: string[];
  validSidecarPaths?: string[];
  sidecarConflicts?: SidecarConflict[];
  offsetSeconds?: number;
  /** روش سینک برنده (مرجع صوتی/مرجع کامل/نمونه‌برداری) — شفافیت تنظیم دوباره */
  syncStrategy?: string;
  preSelection?: { archiveFileName: string; memberPath: string };
  /** تصمیم خودکار سیاست در حالت Auto — چه انتخابی و چرا */
  autoDecision?: AutoDecisionRecord;
  /** رکورد تلاش‌های سینک — هر استراتژی با وضعیت و آفستش */
  syncAttempts?: SyncAttemptRecord[];
}

/** snapshot یک ویدیو در pipeline */
export interface PipelineItemSnapshot extends ItemArtifacts {
  videoPath: string;
  videoFileName: string;
  stage: PipelineStage;
  stageStatus: StageStatus;
  outcome: ItemOutcome | null;
}

/** شمارنده‌های واقعی بک‌اند — هیچ درصد ساختگی وجود ندارد. */
export interface PipelineCounts {
  analyzed: number;
  extracted: number;
  downloaded: number;
  archivesPlaced: number;
  synced: number;
  softsubbed: number;
  hardsubbed: number;
  unchangedLowQuality: number;
  /** مواردی که سیاست خودکار تصمیم‌نگرفت — نیازمند بررسی کاربر */
  needsReview: number;
  alreadySynced: number;
  conflicts: number;
  needsApiKey: number;
  keyStoreUnavailable: number;
  skipped: number;
  failed: number;
  decisionsPending: number;
}

/** مسیر انتخابی ردشده — قرینه scanner::SelectionIssue */
export interface SelectionIssue {
  path: string;
  kind: string;
  detail: string;
}

/** تصمیم باز — شناسه‌ها مات و backend-owned هستند؛ فرانت فقط همین idها را
 * برمی‌گرداند و هرگز مقدار (مسیر/اندیس) از خودش نمی‌سازد. */
export type DecisionKind =
  | "embeddedTrack"
  | "reviewTrack"
  | "archiveCandidate"
  | "syncSidecar";

export type DecisionOptionPayload =
  | { kind: "embeddedTrack"; streamIndex: number; label: string }
  | { kind: "reviewTrack"; streamIndex: number; label: string }
  | { kind: "archiveCandidate"; archiveFileName: string; memberPath: string }
  | { kind: "sidecarPath"; path: string };

/** گزینهٔ ثبت‌شده — payload با flatten داخل همان شیء می‌آید. */
export type RegisteredOption = { optionId: string } & DecisionOptionPayload;

export interface PendingDecision {
  decisionId: string;
  kind: DecisionKind;
  videoPath: string;
  videoFileName: string;
  options: RegisteredOption[];
}

/** snapshot کامل run — برای رویداد pipeline:state و بازیابی get_pipeline_state */
export interface PipelineSnapshot {
  /** شمارهٔ یکنوای snapshot — رویداد قدیمی‌تر نباید وضعیت جدیدتر را overwrite کند */
  seq: number;
  runId: string;
  stage: PipelineStage;
  status: PipelineStatus;
  total: number;
  completed: number;
  /** تعداد آیتم‌هایی که همین حالا در حال پردازش‌اند */
  active: number;
  /** درخواست توقف ثبت شده و worker هنوز به وضعیت پایانی نرسیده */
  cancelling: boolean;
  currentFile: string | null;
  counts: PipelineCounts;
  message: string;
  /** پیشرفت واقعی مرحلهٔ جاری — فقط وقتی مرحله شمارش آیتمی معتبر دارد */
  stageProgress?: { completed: number; total: number } | null;
  /** فقط حین انتظار تصمیم حاضر است (skip_serializing_if) */
  pendingDecision?: PendingDecision;
}

/** خلاصهٔ نهایی run — رویداد pipeline:done */
export interface PipelineSummary {
  runId: string;
  status: PipelineStatus;
  cancelled: boolean;
  counts: PipelineCounts;
  items: PipelineItemSnapshot[];
  invalidSelections: SelectionIssue[];
  stagedPaths: string[];
  warnings: string[];
  keyStoreError?: string;
  durationMs: number;
  ephemeral: boolean;
}
