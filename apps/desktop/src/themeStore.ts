/**
 * Singleton theme store — shared by useTheme hook and keyboard shortcuts.
 * All reads/writes go through this module so state stays in sync.
 */
export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "fan-files-theme";

export function readTheme(): Theme {
  if (typeof window === "undefined") return "system";
  const v = localStorage.getItem(STORAGE_KEY);
  if (v === "light" || v === "dark" || v === "system") return v;
  return "system";
}

export function writeTheme(t: Theme) {
  localStorage.setItem(STORAGE_KEY, t);
  applyThemeClass(t);
}

export function applyThemeClass(theme: Theme) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.classList.remove("light", "dark");
  if (theme === "system") {
    const dark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
    root.classList.add(dark ? "dark" : "light");
  } else {
    root.classList.add(theme);
  }
}

export function setThemeFromKeyboard(theme: Theme) {
  writeTheme(theme);
  listeners.forEach((cb) => cb(theme));
}

// Pub/sub so keyboard shortcut can notify the hook
const listeners = new Set<(t: Theme) => void>();
export function subscribeTheme(fn: (t: Theme) => void): () => void {
  listeners.add(fn);
  return () => { listeners.delete(fn); };
}
