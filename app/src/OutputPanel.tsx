import { open } from "@tauri-apps/plugin-dialog";
import type { Settings } from "./types";

interface Props {
  settings: Settings;
  onSettings(s: Settings): void;
  locked: boolean;
}

export function OutputPanel({ settings: s, onSettings: set, locked }: Props) {
  const pick = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose output folder" });
    if (typeof dir === "string") set({ ...s, outputFolder: dir, sameFolder: false });
  };
  const overwrite = s.sameFolder && s.suffix.trim() === "";
  return (
    <aside className="side">
      <div className="panel-title">Output</div>
      <fieldset className="panel-body" disabled={locked}>
        <div className="form">
          <label className="check">
            <input type="checkbox" checked={s.sameFolder} onChange={(e) => set({ ...s, sameFolder: e.target.checked })} />
            Same folder as input
          </label>
          <label className="lbl">Folder</label>
          <input className="input" value={s.sameFolder ? "" : s.outputFolder} readOnly disabled={s.sameFolder} placeholder={s.sameFolder ? "" : "No folder selected"} title={s.outputFolder} />
          <div className="row-end">
            <button className="btn small" onClick={pick} disabled={s.sameFolder}>
              Select...
            </button>
          </div>
          <label className="check">
            <input type="checkbox" checked={s.keepStructure} disabled={s.sameFolder} onChange={(e) => set({ ...s, keepStructure: e.target.checked })} />
            Keep folder structure
          </label>
          <hr />
          <label className="lbl" htmlFor="suffix">
            Suffix
          </label>
          <input id="suffix" className="input" value={s.suffix} placeholder="e.g. _clean" onChange={(e) => set({ ...s, suffix: e.target.value })} />
          {overwrite && <p className="hint warn-hint">Same folder and no suffix: original files will be overwritten (Date Created is kept). Back them up first if you need them.</p>}
        </div>
      </fieldset>
      <p className="hint about">Removes the XMP property photoshop:DocumentAncestors from PSD/PSB files and every nested PSD/PSB Smart Object, without Photoshop.</p>
    </aside>
  );
}
