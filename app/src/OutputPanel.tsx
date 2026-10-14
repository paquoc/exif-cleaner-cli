import { open } from "@tauri-apps/plugin-dialog";
import type { Settings } from "./types";

interface Props {
  settings: Settings;
  onSettings(s: Settings): void;
  locked: boolean;
}

export function OutputPanel({ settings: s, onSettings: set, locked }: Props) {
  const pick = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Chọn thư mục output" });
    if (typeof dir === "string") set({ ...s, outputFolder: dir, sameFolder: false });
  };
  const overwrite = s.sameFolder && s.suffix.trim() === "";
  return (
    <aside className="side">
      <div className="panel-title">Output</div>
      <fieldset className="panel-body" disabled={locked}>
        <div className="form">
          <label className="lbl">Thư mục</label>
          <input className="input" value={s.outputFolder} readOnly disabled={s.sameFolder} placeholder={s.sameFolder ? "" : "Chưa chọn thư mục"} title={s.outputFolder} />
          <div className="row-end">
            <button className="btn small" onClick={pick} disabled={s.sameFolder}>
              Chọn...
            </button>
          </div>
          <label className="check">
            <input type="checkbox" checked={s.sameFolder} onChange={(e) => set({ ...s, sameFolder: e.target.checked })} />
            Cùng thư mục với file gốc
          </label>
          <label className="check">
            <input type="checkbox" checked={s.keepStructure} disabled={s.sameFolder} onChange={(e) => set({ ...s, keepStructure: e.target.checked })} />
            Giữ cấu trúc thư mục
          </label>
          <div className="check-row">
            <label className="check">
              <input type="checkbox" checked={s.moveOriginal} onChange={(e) => set({ ...s, moveOriginal: e.target.checked })} />
              Chuyển file gốc
            </label>
            <select
              className="input select"
              value={s.originalTarget}
              disabled={!s.moveOriginal}
              onChange={(e) => set({ ...s, originalTarget: e.target.value as Settings["originalTarget"] })}
            >
              <option value="trash">Thùng rác</option>
              <option value="backup_folder">_exif_backup</option>
            </select>
          </div>
          <hr />
          <label className="lbl" htmlFor="suffix">
            Hậu tố tên file
          </label>
          <input id="suffix" className="input" value={s.suffix} placeholder="vd: _clean" onChange={(e) => set({ ...s, suffix: e.target.value })} />
          {overwrite && (
            <p className={`hint ${s.moveOriginal ? "" : "warn-hint"}`}>
              {s.moveOriginal ? "File gốc sẽ được chuyển đi, file sạch nằm đúng chỗ cũ." : "Cùng thư mục và không có hậu tố: file gốc sẽ bị ghi đè."}
            </p>
          )}
        </div>
      </fieldset>
      <p className="hint about">Xóa XMP photoshop:DocumentAncestors trong file và mọi Smart Object PSD/PSB lồng bên trong, không cần Photoshop.</p>
    </aside>
  );
}
