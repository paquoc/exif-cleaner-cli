import { useRef } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { bytes, count, duration, percentSaved } from "./format";
import { FolderOpen, Layers } from "lucide-react";
import type { FileItem, FileStatus } from "./types";

const STATUS_LABEL: Record<FileStatus, string> = {
  idle: "Pending",
  queued: "Queued",
  running: "Running",
  cleaned: "Cleaned",
  already_clean: "Already clean",
  missing: "Not found",
  failed: "Failed",
  cancelled: "Stopped",
};

/** The second line under the file name: error, warning, smart object in progress, or folder. */
function subLine(f: FileItem): { text: string; cls: string } {
  const r = f.report;
  if (f.status === "running") return { text: f.current ? `› ${f.current}` : f.folder, cls: "sub-run" };
  if (r?.error) return { text: r.error, cls: "sub-err" };
  if (f.status === "missing") return { text: "The file no longer exists", cls: "sub-err" };
  if (r && r.stats.warnings.length > 0) return { text: `${r.stats.warnings.length} warning(s): ${r.stats.warnings[0]}`, cls: "sub-warn" };
  if (r?.original_moved_to) return { text: r.original_moved_to === "deleted" ? "Original deleted" : `Original → ${r.original_moved_to}`, cls: "" };
  return { text: f.folder, cls: "" };
}

function tooltip(f: FileItem): string {
  const r = f.report;
  if (!r || !(r.status === "cleaned" || r.status === "already_clean")) return f.path;
  const lines = [f.path, `Smart objects: ${count(r.stats.smart_objects)} (${count(r.stats.shared_smart_objects)} with identical content)`, `Ancestors removed: ${count(r.stats.ancestors_removed)}`];
  const skipped = Object.entries(r.stats.skipped_types);
  if (skipped.length) lines.push(`Skipped (not PSD/PSB): ${skipped.map(([k, v]) => `${k} ×${v}`).join(", ")}`);
  if (r.output && r.output !== f.path) lines.push(`Output: ${r.output}`);
  if (r.original_moved_to) lines.push(r.original_moved_to === "deleted" ? "Original deleted" : `Original → ${r.original_moved_to}`);
  lines.push(...r.stats.warnings.map((w) => `⚠ ${w}`));
  return lines.join("\n");
}

function Status({ f }: { f: FileItem }) {
  if (f.status === "running") {
    const pct = Math.round((f.progress ?? 0) * 100);
    return (
      <div className="row-progress" title={`${pct}%`}>
        <div className="row-bar">
          <div style={{ width: `${pct}%` }} />
        </div>
        <span className="row-pct">{pct}%</span>
      </div>
    );
  }
  return <span className={`badge b-${f.status}`}>{STATUS_LABEL[f.status]}</span>;
}

interface Props {
  files: FileItem[];
  selected: Set<string>;
  onSelect(next: Set<string>): void;
  dragOver: boolean;
}

export function FileTable({ files, selected, onSelect, dragOver }: Props) {
  const anchor = useRef<number | null>(null);

  const click = (e: React.MouseEvent, i: number) => {
    const id = files[i].id;
    if (e.shiftKey && anchor.current != null) {
      const [a, b] = [Math.min(anchor.current, i), Math.max(anchor.current, i)];
      onSelect(new Set(files.slice(a, b + 1).map((f) => f.id)));
      return;
    }
    anchor.current = i;
    if (e.ctrlKey || e.metaKey) {
      const next = new Set(selected);
      next.has(id) ? next.delete(id) : next.add(id);
      onSelect(next);
    } else {
      onSelect(new Set([id]));
    }
  };

  const reveal = (e: React.MouseEvent, f: FileItem) => {
    e.stopPropagation();
    revealItemInDir(f.report?.output ?? f.path).catch(() => {});
  };

  if (files.length === 0) {
    return (
      <div className={`empty ${dragOver ? "drag" : ""}`}>
        <Layers size={44} strokeWidth={1.3} />
        <p className="empty-title">Drop PSD/PSB files or folders here</p>
        <p className="muted">or use Add files / Add folder above</p>
      </div>
    );
  }

  return (
    <div className={`table-wrap ${dragOver ? "drag" : ""}`}>
      <table className="files">
        <thead>
          <tr>
            <th className="c-name">Name</th>
            <th className="c-size">Size</th>
            <th className="c-size">Cleaned</th>
            <th className="c-saved">Saved</th>
            <th className="c-time">Time</th>
            <th className="c-status">Status</th>
            <th className="c-act" aria-label="Open folder" />
          </tr>
        </thead>
        <tbody>
          {files.map((f, i) => {
            const r = f.report;
            const done = r && (r.status === "cleaned" || r.status === "already_clean");
            const sub = subLine(f);
            return (
              <tr key={f.id} className={`${selected.has(f.id) ? "sel" : ""} st-${f.status}`} onClick={(e) => click(e, i)} onDoubleClick={(e) => reveal(e, f)} title={tooltip(f)}>
                <td className="c-name">
                  <div className="name">{f.name}</div>
                  <div className={`sub ${sub.cls}`}>{sub.text}</div>
                </td>
                <td className="c-size">{bytes(f.size)}</td>
                <td className="c-size">{done ? bytes(r!.size_after) : "—"}</td>
                <td className={`c-saved ${done && r!.size_after! < r!.size_before ? "good" : ""}`}>{done ? percentSaved(r!.size_before, r!.size_after) : "—"}</td>
                <td className="c-time">{r && r.status !== "missing" ? duration(r.millis) : "—"}</td>
                <td className="c-status">
                  <Status f={f} />
                </td>
                <td className="c-act">
                  <button className="icon-btn" title={r?.output && r.output !== f.path ? "Show output file" : "Show in folder"} onClick={(e) => reveal(e, f)}>
                    <FolderOpen size={16} />
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
