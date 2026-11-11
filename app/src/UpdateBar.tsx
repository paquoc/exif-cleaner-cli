import { useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Download, RefreshCw, X } from "lucide-react";
import { isInstalled } from "./api";

const RELEASES = "https://github.com/paquoc/exif-cleaner-cli/releases/latest";

type State =
  | { kind: "none" }
  | { kind: "available"; update: Update }
  | { kind: "downloading"; update: Update; got: number; total?: number }
  | { kind: "installing" }
  | { kind: "failed"; update: Update; error: string };

/** Checks GitHub for a newer release once at startup. An installed copy downloads the new installer and
 * runs it (the app closes, updates and reopens); a portable copy just links to the release page. */
export function UpdateBar({ busy, onUpdating }: { busy: boolean; onUpdating(updating: boolean): void }) {
  const [state, setState] = useState<State>({ kind: "none" });
  const [installed, setInstalled] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const started = useRef(false);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    // Offline, rate-limited or no release yet: say nothing.
    Promise.all([check({ timeout: 15000 }), isInstalled()])
      .then(([update, inst]) => {
        setInstalled(inst);
        if (update) setState({ kind: "available", update });
      })
      .catch(() => {});
  }, []);

  if (state.kind === "none" || dismissed) return null;

  const install = async (update: Update) => {
    onUpdating(true);
    setState({ kind: "downloading", update, got: 0 });
    let got = 0;
    let total: number | undefined;
    try {
      await update.download((e) => {
        if (e.event === "Started") total = e.data.contentLength;
        else if (e.event === "Progress") got += e.data.chunkLength;
        setState({ kind: "downloading", update, got, total });
      });
      setState({ kind: "installing" });
      // Windows: launches the installer and exits; the installer reopens the app when done.
      await update.install();
    } catch (e) {
      onUpdating(false);
      setState({ kind: "failed", update, error: String(e) });
    }
  };

  const version = state.kind === "installing" ? "" : `v${state.update.version}`;

  return (
    <div className="update-bar" role="status">
      {state.kind === "available" && (
        <>
          <span className="update-text">
            <b>{version}</b> is available (you have v{state.update.currentVersion}).
          </span>
          {installed ? (
            <button className="btn small primary-soft" onClick={() => install(state.update)} disabled={busy} title={busy ? "Wait for the current run to finish" : undefined}>
              <RefreshCw size={13} /> Update and restart
            </button>
          ) : (
            <button className="btn small primary-soft" onClick={() => openUrl(RELEASES)}>
              <Download size={13} /> Download
            </button>
          )}
        </>
      )}
      {state.kind === "downloading" && (
        <>
          <span className="update-text">
            Downloading {version}... {state.total ? `${Math.floor((state.got / state.total) * 100)}%` : ""}
          </span>
          <span className="update-progress">
            <span style={{ width: state.total ? `${(state.got / state.total) * 100}%` : "30%" }} />
          </span>
        </>
      )}
      {state.kind === "installing" && <span className="update-text">Installing... the app will close and reopen.</span>}
      {state.kind === "failed" && (
        <>
          <span className="update-text err" title={state.error}>
            Update failed: {state.error}
          </span>
          <button className="btn small" onClick={() => install(state.update)} disabled={busy}>
            <RefreshCw size={13} /> Retry
          </button>
          <button className="btn small" onClick={() => openUrl(RELEASES)}>
            <Download size={13} /> Download
          </button>
        </>
      )}
      {(state.kind === "available" || state.kind === "failed") && (
        <button className="icon-btn" onClick={() => setDismissed(true)} aria-label="Dismiss">
          <X size={14} />
        </button>
      )}
    </div>
  );
}
