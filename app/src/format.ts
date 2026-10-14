export function bytes(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1024 ** 3) return `${(n / 1024 ** 3).toFixed(2)} GB`;
  if (n >= 1024 ** 2) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${n} B`;
}

export function percentSaved(before: number, after: number | null | undefined): string {
  if (after == null || before <= 0) return "—";
  const p = ((before - after) / before) * 100;
  return p <= 0.05 ? "0%" : `-${p.toFixed(1)}%`;
}

export function count(n: number): string {
  return n.toLocaleString("vi-VN");
}

export function duration(ms: number): string {
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
}

/** Splits a Windows or POSIX path into folder and file name. */
export function splitPath(p: string): { folder: string; name: string } {
  const i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
  return i < 0 ? { folder: "", name: p } : { folder: p.slice(0, i), name: p.slice(i + 1) };
}
