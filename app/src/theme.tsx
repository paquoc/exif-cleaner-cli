import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Monitor, Moon, Sun } from "lucide-react";

export type ThemeMode = "system" | "light" | "dark";
const KEY = "exif-cleaner.theme";
const MODES: { mode: ThemeMode; label: string; Icon: typeof Sun }[] = [
  { mode: "system", label: "Theo hệ thống", Icon: Monitor },
  { mode: "light", label: "Sáng", Icon: Sun },
  { mode: "dark", label: "Tối", Icon: Moon },
];

function load(): ThemeMode {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

/** Applies the theme to the page (`data-theme` on <html>) and to the native title bar. */
function apply(mode: ThemeMode) {
  const root = document.documentElement;
  if (mode === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", mode);
  getCurrentWindow()
    .setTheme(mode === "system" ? null : mode)
    .catch(() => {});
}

// Apply before the first paint so the window does not flash the other theme.
apply(load());

export function ThemeSwitch() {
  const [mode, setMode] = useState<ThemeMode>(load);
  useEffect(() => {
    apply(mode);
    try {
      localStorage.setItem(KEY, mode);
    } catch {
      /* not persisted */
    }
  }, [mode]);
  return (
    <div className="seg" role="radiogroup" aria-label="Giao diện">
      {MODES.map(({ mode: m, label, Icon }) => (
        <button key={m} role="radio" aria-checked={mode === m} className={mode === m ? "on" : ""} title={label} onClick={() => setMode(m)}>
          <Icon size={15} />
        </button>
      ))}
    </div>
  );
}
