import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CleanEvent, FileReport } from "./types";

export interface ScannedFile {
  path: string;
  rel: string;
  size: number | null;
}

export interface FileCheck {
  path: string;
  exists: boolean;
  size: number | null;
}

export interface RunOptions {
  dryRun: boolean;
  sameFolder: boolean;
  outputFolder: string;
  keepStructure: boolean;
  suffix: string;
  maxDepth: number;
}

export const initialPaths = () => invoke<string[]>("initial_paths");
export const scanPaths = (paths: string[]) => invoke<ScannedFile[]>("scan_paths", { paths });
export const checkFiles = (paths: string[]) => invoke<FileCheck[]>("check_files", { paths });
export const startRun = (tasks: { path: string; rel: string }[], options: RunOptions) => invoke<void>("start_run", { tasks, options });
export const stopRun = () => invoke<void>("stop_run");
export const isInstalled = () => invoke<boolean>("is_installed");

export interface RunHandlers {
  onFileStart(index: number, output: string | null): void;
  onProgress(index: number, event: CleanEvent): void;
  onFileDone(index: number, report: FileReport): void;
  onFinished(cancelled: boolean): void;
}

export async function listenRun(h: RunHandlers): Promise<UnlistenFn> {
  const offs = await Promise.all([
    listen<{ index: number; output: string | null }>("run:file-start", (e) => h.onFileStart(e.payload.index, e.payload.output)),
    listen<{ index: number; event: CleanEvent }>("run:progress", (e) => h.onProgress(e.payload.index, e.payload.event)),
    listen<{ index: number; report: FileReport }>("run:file-done", (e) => h.onFileDone(e.payload.index, e.payload.report)),
    listen<{ cancelled: boolean }>("run:finished", (e) => h.onFinished(e.payload.cancelled)),
  ]);
  return () => offs.forEach((off) => off());
}
