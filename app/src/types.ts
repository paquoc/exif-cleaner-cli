export type Original = "keep" | "trash" | "delete";

export type FileStatus = "queued" | "idle" | "running" | "cleaned" | "already_clean" | "missing" | "failed" | "cancelled";

export interface Stats {
  documents: number;
  documents_changed: number;
  smart_objects: number;
  shared_smart_objects: number;
  ancestors_removed: number;
  skipped_types: Record<string, number>;
  warnings: string[];
}

export interface FileReport {
  path: string;
  status: "cleaned" | "already_clean" | "missing" | "failed" | "cancelled";
  dry_run: boolean;
  size_before: number;
  size_after: number | null;
  output: string | null;
  original_moved_to: string | null;
  stats: Stats;
  error: string | null;
  millis: number;
}

export interface FileItem {
  id: string;
  path: string;
  rel: string;
  name: string;
  folder: string;
  size: number | null;
  status: FileStatus;
  report?: FileReport;
  output?: string | null;
  /** Smart object being cleaned right now. */
  current?: string;
  /** 0..1 while running. */
  progress?: number;
}

export interface Settings {
  sameFolder: boolean;
  outputFolder: string;
  keepStructure: boolean;
  suffix: string;
  moveOriginal: boolean;
  originalTarget: "trash" | "delete";
}

export const DEFAULT_SETTINGS: Settings = {
  sameFolder: true,
  outputFolder: "",
  keepStructure: true,
  suffix: "",
  moveOriginal: false,
  originalTarget: "trash",
};

export type CleanEvent =
  | { kind: "enter"; depth: number; name: string }
  | { kind: "shared"; depth: number; name: string }
  | { kind: "leave"; depth: number; name: string; ancestors: number; changed: boolean }
  | { kind: "progress"; fraction: number };
