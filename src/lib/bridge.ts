import type { ProviderSnapshot, WidgetPreferences } from "../types";

const defaultPreferences: WidgetPreferences = { locked: false, alwaysOnTop: true, stayExpanded: false, showStatusBarProgress: false, pinnedProvider: null, autoRotateSeconds: 12, expandedSize: 320, language: "zh-CN", theme: "aurora", progressStyle: "solid" };

const mockSnapshot: ProviderSnapshot = {
  provider: "codex",
  displayName: "CODEX",
  plan: "PRO",
  shortWindow: { remainingPercent: 74, resetsAt: new Date(Date.now() + 78 * 60_000).toISOString(), windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 42, resetsAt: new Date(Date.now() + 3.2 * 86_400_000).toISOString(), windowSeconds: 604_800 },
  balance: null,
  resetCredits: 1,
  resetCreditExpiresAt: [new Date(Date.now() + 9 * 86_400_000).toISOString()],
  updatedAt: new Date().toISOString(),
  status: "ok",
  message: null,
};

let widgetTransition: Promise<void> = Promise.resolve();

function wait(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

export interface WidgetExpandSize {
  width: number;
  height: number;
}

function enqueueWidgetTransition(operation: () => Promise<void>): Promise<void> {
  const next = widgetTransition.then(operation, operation);
  widgetTransition = next.catch(() => undefined);
  return next;
}

export const isTauri = () => "__TAURI_INTERNALS__" in window;

export async function fetchSnapshots(force = false): Promise<ProviderSnapshot[]> {
  if (!isTauri()) return [mockSnapshot];
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ProviderSnapshot[]>(force ? "refresh_snapshots" : "get_snapshots");
}

export async function getPreferences(): Promise<WidgetPreferences> {
  if (!isTauri()) return defaultPreferences;
  const { invoke } = await import("@tauri-apps/api/core");
  let lastError: unknown;
  for (let attempt = 0; attempt < 5; attempt += 1) {
    try {
      return await invoke<WidgetPreferences>("get_preferences");
    } catch (error) {
      lastError = error;
      await wait(120);
    }
  }
  throw lastError;
}

export async function updatePreferences(value: WidgetPreferences): Promise<void> {
  if (!isTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_preferences", { preferences: value });
}

export async function setClickThrough(locked: boolean): Promise<WidgetPreferences> {
  if (!isTauri()) return { ...defaultPreferences, locked };
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<WidgetPreferences>("set_widget_locked", { locked });
}

export async function setAlwaysOnTop(alwaysOnTop: boolean): Promise<WidgetPreferences> {
  if (!isTauri()) return { ...defaultPreferences, alwaysOnTop };
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<WidgetPreferences>("set_widget_always_on_top", { alwaysOnTop });
}

export async function startDragging(): Promise<void> {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const { invoke } = await import("@tauri-apps/api/core");
  const currentWindow = getCurrentWindow();
  await invoke("begin_widget_drag");
  await currentWindow.startDragging();
  let previous = await currentWindow.outerPosition();
  let stableTicks = 0;
  let attempts = 0;
  const finishWhenStable = window.setInterval(() => {
    void currentWindow.outerPosition()
      .then((next) => {
        attempts += 1;
        const stable = Math.abs(next.x - previous.x) <= 1 && Math.abs(next.y - previous.y) <= 1;
        stableTicks = stable ? stableTicks + 1 : 0;
        previous = next;
        if (stableTicks >= 3 || attempts >= 25) {
          window.clearInterval(finishWhenStable);
          void invoke("finish_widget_drag").catch(() => undefined);
        }
      })
      .catch(() => {
        window.clearInterval(finishWhenStable);
        void invoke("finish_widget_drag").catch(() => undefined);
      });
  }, 80);
}

export async function closeCurrentWindow(): Promise<void> {
  if (!isTauri()) return;
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  await getCurrentWindow().hide();
}

export function setWidgetExpanded(expanded: boolean, logicalSize?: WidgetExpandSize): Promise<void> {
  if (!isTauri()) return Promise.resolve();
  return enqueueWidgetTransition(async () => {
    const { invoke } = await import("@tauri-apps/api/core");
    if (!expanded) {
      await invoke("collapse_widget");
      return;
    }
    const { currentMonitor } = await import("@tauri-apps/api/window");
    const monitor = await currentMonitor().catch(() => null);
    const workArea = monitor ? {
      position: { x: monitor.workArea.position.x, y: monitor.workArea.position.y },
      size: { width: monitor.workArea.size.width, height: monitor.workArea.size.height },
    } : null;
    await invoke("expand_widget", logicalSize ? { workArea, logicalSize } : { workArea });
  });
}

export async function setStatusBarProgressVisible(visible: boolean): Promise<void> {
  if (!isTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_status_bar_progress_visible", { visible });
}

export async function setTrayProgress(visible: boolean, percent: number, tooltip: string, tier: string): Promise<void> {
  if (!isTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_tray_progress", { visible, percent, tooltip, tier });
}

export async function setWidgetVisible(visible: boolean): Promise<void> {
  if (!isTauri()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_widget_visible", { visible });
}

export async function publishTrayPreview(payload: unknown): Promise<void> {
  if (!isTauri()) return;
  const { emitTo } = await import("@tauri-apps/api/event");
  await emitTo("statusbar", "tray-preview-updated", payload);
}

export async function listenDesktopEvents(handlers: {
  onPreferences: (value: WidgetPreferences) => void;
  onRefresh: () => void;
  onUpdate: () => void;
  onThemePanel?: () => void;
}): Promise<() => void> {
  if (!isTauri()) return () => undefined;
  const { listen } = await import("@tauri-apps/api/event");
  const unlistenPreferences = await listen<WidgetPreferences>("preferences-changed", (event) => handlers.onPreferences(event.payload));
  const unlistenRefresh = await listen("refresh-requested", handlers.onRefresh);
  const unlistenUpdate = await listen("update-check-requested", handlers.onUpdate);
  const unlistenThemePanel = handlers.onThemePanel
    ? await listen("theme-panel-requested", handlers.onThemePanel)
    : () => undefined;
  return () => { unlistenPreferences(); unlistenRefresh(); unlistenUpdate(); unlistenThemePanel(); };
}
