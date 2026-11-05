import { useCallback, useEffect, useSyncExternalStore } from "react";

/**
 * Class-based light/dark theme, same semantics as the client's next-themes
 * setup: "system" by default (follows prefers-color-scheme live), an explicit
 * choice is persisted under the same `theme` key. `public/theme-init.js`
 * applies it before first paint.
 */
export type ThemePref = "light" | "dark" | "system";

const KEY = "theme";
const listeners = new Set<() => void>();
const media = () => window.matchMedia("(prefers-color-scheme: dark)");

function readPref(): ThemePref {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

function resolve(pref: ThemePref): "light" | "dark" {
  return pref === "system" ? (media().matches ? "dark" : "light") : pref;
}

function apply() {
  const dark = resolve(readPref()) === "dark";
  document.documentElement.classList.toggle("dark", dark);
  listeners.forEach((l) => l());
}

function subscribe(l: () => void) {
  listeners.add(l);
  return () => listeners.delete(l);
}

export function useTheme() {
  const pref = useSyncExternalStore(subscribe, readPref);
  const resolved = useSyncExternalStore(subscribe, () => resolve(readPref()));

  useEffect(() => {
    const mq = media();
    const onChange = () => {
      if (readPref() === "system") apply();
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const setTheme = useCallback((next: ThemePref) => {
    try {
      if (next === "system") localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, next);
    } catch {
      /* private mode — the choice just won't persist */
    }
    apply();
  }, []);

  return { theme: pref, resolvedTheme: resolved, setTheme };
}
