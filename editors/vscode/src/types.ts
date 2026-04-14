// TypeScript mirrors of scrutin-web wire.rs types.
// Keep in sync with crates/scrutin-web/src/wire.rs.

export type FileId = string; // hex-encoded xxhash64
export type RunId = string; // UUID v4

export type WireOutcome = "pass" | "fail" | "error" | "skip" | "xfail" | "warn";

export type WireStatus =
  | "unknown"
  | "pending"
  | "running"
  | "passed"
  | "failed"
  | "errored"
  | "skipped"
  | "cancelled";

export interface WireCounts {
  pass: number;
  fail: number;
  error: number;
  skip: number;
  xfail: number;
  warn: number;
}

export interface WireLocation {
  file: string;
  line?: number;
}

export interface WireMetrics {
  total?: number;
  failed?: number;
  fraction?: number;
}

export interface WireMessage {
  outcome: WireOutcome;
  test_name?: string;
  subject_kind?: string;
  subject_parent?: string;
  location?: WireLocation;
  message?: string;
  duration_ms: number;
  metrics?: WireMetrics;
}

export interface WireFile {
  id: FileId;
  path: string;
  name: string;
  suite: string;
  status: WireStatus;
  last_duration_ms?: number;
  last_run_id?: RunId;
  counts: WireCounts;
  messages: WireMessage[];
  bad: boolean;
}

export interface WireSuiteAction {
  name: string;
  key: string;
  label: string;
}

export interface WireSuite {
  name: string;
  language: string;
  test_dir: string;
  source_dir?: string;
  file_count: number;
  actions: WireSuiteAction[];
}

export interface WirePackage {
  name: string;
  root: string;
  tool: string;
  suites: WireSuite[];
}

export interface WireRunSummary {
  run_id?: RunId;
  started_at?: string;
  finished_at?: string;
  in_progress: boolean;
  totals: WireCounts;
  bad_files: FileId[];
  busy: number;
}

export interface WireSnapshot {
  pkg: WirePackage;
  files: WireFile[];
  current_run: WireRunSummary;
  watching: boolean;
  n_workers: number;
}

// SSE event kinds
export type WireEventKind =
  | "run_started"
  | "file_started"
  | "file_finished"
  | "run_complete"
  | "run_cancelled"
  | "watcher_triggered"
  | "log"
  | "heartbeat";
