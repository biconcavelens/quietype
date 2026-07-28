import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Theme = "system" | "light" | "dark";

let current: Theme = "system";

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

function resolve(theme: Theme): "light" | "dark" {
  if (theme === "system") return darkQuery.matches ? "dark" : "light";
  return theme;
}

export function applyTheme(theme: Theme) {
  current = theme;
  document.documentElement.dataset.theme = resolve(theme);
}

/// Applies the stored theme and keeps it in sync: with the OS while set to
/// "system", and with the settings window whenever it saves.
export async function initTheme() {
  try {
    const settings = await invoke<{ theme?: Theme }>("get_settings");
    applyTheme(settings.theme ?? "system");
  } catch {
    // Falling back rather than failing: a theme lookup should never be what
    // stops the window from rendering.
    applyTheme("system");
  }

  listen<{ theme?: Theme }>("settings-changed", (event) => {
    applyTheme(event.payload.theme ?? "system");
  });

  darkQuery.addEventListener("change", () => {
    if (current === "system") applyTheme("system");
  });
}
