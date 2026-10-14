import { useCallback, useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ask, message, open } from "@tauri-apps/plugin-dialog";
import { checkFiles, initialPaths, listenRun, scanPaths, startRun, stopRun, type ScannedFile } from "./api";
import { FileTable } from "./FileTable";
import { OutputPanel } from "./OutputPanel";
import { bytes, count, splitPath } from "./format";
import { CircleMinus, FilePlus2, FolderPlus, Play, Square, Trash2 } from "lucide-react";
import { ThemeSwitch } from "./theme";
import { DEFAULT_SETTINGS, type FileItem, type FileReport, type Settings } from "./types";

const SETTINGS_KEY = "exif-cleaner.settings";
const MAX_DEPTH = 32;

function loadSettings(): Settings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (!raw) return DEFAULT_SETTINGS;
    const saved = JSON.parse(raw);
    const s = { ...DEFAULT_SETTINGS };
    for (const k of Object.keys(DEFAULT_SETTINGS) as (keyof Settings)[]) {
      if (typeof saved[k] === typeof DEFAULT_SETTINGS[k]) (s as Record<string, unknown>)[k] = saved[k];
    }
    return s;
  } catch {
    return DEFAULT_SETTINGS;
  }
}

const idOf = (path: string) => path.toLowerCase();

interface RunState {
  running: boolean;
  stopping: boolean;
  total: number;
  done: number;
  current: string;
}

const IDLE: RunState = { running: false, stopping: false, total: 0, done: 0, current: "" };
const APP = { title: "Exif Cleaner" };

export default function App() {
  const [files, setFiles] = useState<FileItem[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [settings, setSettings] = useState<Settings>(loadSettings);
  const [run, setRun] = useState<RunState>(IDLE);
  const [summary, setSummary] = useState<string>("");
  const [dragOver, setDragOver] = useState(false);
  const runIds = useRef<string[]>([]);
  const results = useRef<FileReport[]>([]);
  const runningRef = useRef(false);
  runningRef.current = run.running;

  useEffect(() => {
    try {
      localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
    } catch {
      /* settings just won't persist */
    }
  }, [settings]);

  const patch = useCallback((id: string, f: (x: FileItem) => FileItem) => setFiles((fs) => fs.map((x) => (x.id === id ? f(x) : x))), []);

  const addScanned = useCallback((found: ScannedFile[]) => {
    setFiles((fs) => {
      const have = new Set(fs.map((f) => f.id));
      const fresh = found
        .filter((s) => !have.has(idOf(s.path)))
        .map<FileItem>((s) => {
          const { folder, name } = splitPath(s.path);
          return { id: idOf(s.path), path: s.path, rel: s.rel, name, folder, size: s.size, status: s.size == null ? "missing" : "idle" };
        });
      return [...fs, ...fresh];
    });
  }, []);

  const addPaths = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      const found = await scanPaths(paths);
      if (found.length === 0) {
        await message("Không tìm thấy file .psd/.psb nào.", { title: APP.title, kind: "info" });
        return;
      }
      addScanned(found);
    },
    [addScanned],
  );

  // Files passed on the command line (dropped on the exe, "Open with").
  useEffect(() => {
    initialPaths()
      .then(async (p) => {
        if (p.length > 0) addScanned(await scanPaths(p));
      })
      .catch(() => {});
  }, [addScanned]);

  // Drag and drop from Explorer.
  useEffect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "over" || p.type === "enter") setDragOver(true);
        else if (p.type === "leave") setDragOver(false);
        else if (p.type === "drop") {
          setDragOver(false);
          if (!runningRef.current) addPaths(p.paths);
        }
      })
      .then((u) => (dead ? u() : (off = u)));
    return () => {
      dead = true;
      off?.();
    };
  }, [addPaths]);

  // Run events.
  useEffect(() => {
    let off: (() => void) | undefined;
    let dead = false;
    listenRun({
      onFileStart(index, output) {
        const id = runIds.current[index];
        patch(id, (f) => ({ ...f, status: "running", output, current: undefined, progress: 0 }));
        setRun((r) => ({ ...r, current: id }));
      },
      onProgress(index, ev) {
        const id = runIds.current[index];
        if (ev.kind === "progress") {
          // Keep the last 10% for verifying and writing the file.
          patch(id, (f) => ({ ...f, progress: Math.max(f.progress ?? 0, ev.fraction * 0.9) }));
        } else if (ev.kind !== "leave") {
          patch(id, (f) => ({ ...f, current: ev.name }));
        }
      },
      onFileDone(index, report) {
        const id = runIds.current[index];
        results.current.push(report);
        patch(id, (f) => ({ ...f, status: report.status, report, current: undefined, progress: undefined, size: report.size_before || f.size }));
        setRun((r) => ({ ...r, done: r.done + 1 }));
      },
      onFinished(cancelled) {
        setRun(IDLE);
        setFiles((fs) => fs.map((f) => (f.status === "queued" ? { ...f, status: cancelled ? "cancelled" : "idle" } : f)));
        const rs = results.current;
        const ok = rs.filter((r) => r.status === "cleaned" || r.status === "already_clean");
        const before = ok.reduce((a, r) => a + r.size_before, 0);
        const after = ok.reduce((a, r) => a + (r.size_after ?? r.size_before), 0);
        const failed = rs.filter((r) => r.status === "failed" || r.status === "missing").length;
        setSummary(
          `${cancelled ? "Đã dừng" : "Xong"}: ${ok.length} file, giảm ${bytes(before - after)} (${bytes(before)} → ${bytes(after)})` + (failed ? ` • ${failed} file lỗi` : ""),
        );
      },
    }).then((u) => (dead ? u() : (off = u)));
    return () => {
      dead = true;
      off?.();
    };
  }, [patch]);

  const addFiles = async () => {
    const picked = await open({ multiple: true, title: "Chọn file PSD/PSB", filters: [{ name: "Photoshop", extensions: ["psd", "psb"] }] });
    if (picked) addPaths(Array.isArray(picked) ? picked : [picked]);
  };

  const addFolder = async () => {
    const picked = await open({ directory: true, multiple: true, title: "Chọn thư mục" });
    if (picked) addPaths(Array.isArray(picked) ? picked : [picked]);
  };

  const removeSelected = () => {
    setFiles((fs) => fs.filter((f) => !selected.has(f.id)));
    setSelected(new Set());
  };

  const clearAll = () => {
    setFiles([]);
    setSelected(new Set());
    setSummary("");
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (runningRef.current || (e.target as HTMLElement)?.closest("input,select,textarea")) return;
      if (e.key === "Delete" && selected.size > 0) removeSelected();
      if (e.key === "a" && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        setSelected(new Set(files.map((f) => f.id)));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const resetQueued = () => setFiles((fs) => fs.map((f) => (f.status === "queued" ? { ...f, status: "idle" } : f)));

  const start = async () => {
    if (run.running || files.length === 0) return;
    if (!settings.sameFolder && !settings.outputFolder) {
      await message('Chọn thư mục output, hoặc tick "Cùng thư mục với file gốc".', { title: APP.title, kind: "warning" });
      return;
    }
    // Check that every file still exists before running.
    const checks = await checkFiles(files.map((f) => f.path));
    const byId = new Map(checks.map((c) => [idOf(c.path), c]));
    const missing = files.filter((f) => !byId.get(f.id)?.exists);
    const runnable = files.filter((f) => byId.get(f.id)?.exists);
    setFiles((fs) =>
      fs.map((f) => {
        const c = byId.get(f.id);
        return c?.exists ? { ...f, size: c.size, status: "queued", report: undefined, output: undefined } : { ...f, status: "missing", report: undefined };
      }),
    );
    if (runnable.length === 0) {
      await message("Không còn file nào tồn tại trên đĩa.", { title: APP.title, kind: "error" });
      return;
    }
    if (missing.length > 0) {
      const list = missing
        .slice(0, 8)
        .map((f) => f.name)
        .join("\n");
      const go = await ask(`${missing.length} file không còn tồn tại và sẽ được bỏ qua:\n\n${list}${missing.length > 8 ? "\n..." : ""}\n\nChạy tiếp với ${runnable.length} file còn lại?`, {
        title: APP.title,
        kind: "warning",
      });
      if (!go) return resetQueued();
    }
    const overwrite = settings.sameFolder && settings.suffix.trim() === "" && !settings.moveOriginal;
    if (overwrite) {
      const go = await ask(`${runnable.length} file sẽ bị ghi đè trực tiếp, không giữ bản gốc. Tiếp tục?`, { title: APP.title, kind: "warning" });
      if (!go) return resetQueued();
    }
    runIds.current = runnable.map((f) => f.id);
    results.current = [];
    setSummary("");
    setRun({ ...IDLE, running: true, total: runnable.length });
    try {
      await startRun(
        runnable.map((f) => ({ path: f.path, rel: f.rel })),
        {
          dryRun: false,
          sameFolder: settings.sameFolder,
          outputFolder: settings.outputFolder,
          keepStructure: settings.keepStructure,
          suffix: settings.suffix.trim(),
          original: settings.moveOriginal ? settings.originalTarget : "keep",
          maxDepth: MAX_DEPTH,
        },
      );
    } catch (e) {
      setRun(IDLE);
      resetQueued();
      await message(String(e), { title: APP.title, kind: "error" });
    }
  };

  const stop = async () => {
    setRun((r) => ({ ...r, stopping: true }));
    await stopRun();
  };

  const totalSize = files.reduce((a, f) => a + (f.size ?? 0), 0);
  const currentFile = files.find((f) => f.id === run.current);

  return (
    <div className="app">
      <header className="toolbar">
        <button className="tb" onClick={addFiles} disabled={run.running}>
          <FilePlus2 size={17} /> Thêm file
        </button>
        <button className="tb" onClick={addFolder} disabled={run.running}>
          <FolderPlus size={17} /> Thêm thư mục
        </button>
        <span className="sep" />
        <button className="tb" onClick={removeSelected} disabled={run.running || selected.size === 0}>
          <CircleMinus size={17} /> Xóa khỏi danh sách
        </button>
        <button className="tb" onClick={clearAll} disabled={run.running || files.length === 0}>
          <Trash2 size={17} /> Xóa tất cả
        </button>
        <span className="grow" />
        <span className="muted">
          {count(files.length)} file • {bytes(totalSize)}
        </span>
        <span className="sep" />
        <ThemeSwitch />
      </header>

      <main className="main">
        <FileTable files={files} selected={selected} dragOver={dragOver} onSelect={setSelected} />
        <OutputPanel settings={settings} onSettings={setSettings} locked={run.running} />
      </main>

      <footer className="statusbar">
        <div className="status-text">
          {run.running ? (
            <>
              <b className="counter">
                {Math.min(run.done + 1, run.total)}/{run.total}
              </b>
              {run.stopping ? "Đang dừng sau bước hiện tại..." : currentFile ? currentFile.name : "Đang bắt đầu..."}
            </>
          ) : (
            summary || (files.length ? "Sẵn sàng" : "Thêm file để bắt đầu")
          )}
        </div>
        {run.running ? (
          <button className="btn danger" onClick={stop} disabled={run.stopping}>
            <Square size={14} fill="currentColor" /> Dừng
          </button>
        ) : (
          <button className="btn primary" onClick={start} disabled={files.length === 0}>
            <Play size={15} fill="currentColor" /> Chạy
          </button>
        )}
      </footer>
    </div>
  );
}
