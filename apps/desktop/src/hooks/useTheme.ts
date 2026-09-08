import { useEffect, useState } from "react";
import { applyThemeClass, readTheme, subscribeTheme, type Theme } from "../themeStore";

export type { Theme };

export function useTheme(): [Theme, (t: Theme) => void] {
  const [theme, setTheme] = useState<Theme>(readTheme);

  // Apply class on mount
  useEffect(() => {
    applyThemeClass(theme);
  }, []);

  // Persist and broadcast changes
  useEffect(() => {
    localStorage.setItem("fan-files-theme", theme);
    applyThemeClass(theme);
  }, [theme]);

  // Listen to keyboard shortcut changes (pub/sub via themeStore).
  // setThemeFromKeyboard already applied the class + persisted; here we only
  // sync React state so ThemeToggle's pressed state updates.
  useEffect(() => {
    const unsub = subscribeTheme((t) => setTheme(t));
    return unsub;
  }, []);

  return [theme, setTheme];
}
