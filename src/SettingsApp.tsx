import { useCallback, useEffect, useRef, useState } from "react";
import { WidgetSettingsPanel } from "./components/WidgetSettingsPanel";
import { closeCurrentWindow, getPreferences, listenDesktopEvents, setAlwaysOnTop, updatePreferences } from "./lib/bridge";
import { normalizeLanguage } from "./lib/i18n";
import type { ProgressStyle, ThemeName, WidgetPreferences } from "./types";

const DEFAULT_PREFS: WidgetPreferences = { locked: false, alwaysOnTop: true, stayExpanded: false, showStatusBarProgress: false, pinnedProvider: null, autoRotateSeconds: 12, expandedSize: 320, language: "zh-CN", theme: "aurora", progressStyle: "solid" };

export function SettingsApp() {
  const [preferences, setPreferences] = useState(DEFAULT_PREFS);
  const [operationError, setOperationError] = useState<string | null>(null);
  const operationNoticeTimer = useRef<number | null>(null);

  const showTransientOperationNotice = useCallback((message: string, ttl = 3500) => {
    if (operationNoticeTimer.current !== null) {
      window.clearTimeout(operationNoticeTimer.current);
      operationNoticeTimer.current = null;
    }
    setOperationError(message);
    operationNoticeTimer.current = window.setTimeout(() => {
      setOperationError((current) => (current === message ? null : current));
      operationNoticeTimer.current = null;
    }, ttl);
  }, []);

  useEffect(() => {
    void getPreferences()
      .then((value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }))
      .catch(() => showTransientOperationNotice("Unable to read settings. Defaults are in use."));
    return () => {
      if (operationNoticeTimer.current !== null) window.clearTimeout(operationNoticeTimer.current);
    };
  }, [showTransientOperationNotice]);

  useEffect(() => {
    let cancelled = false;
    let cleanup: () => void = () => {};
    void listenDesktopEvents({
      onPreferences: (value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }),
      onRefresh: () => undefined,
      onUpdate: () => undefined,
    }).then((value) => {
      if (cancelled) value(); else cleanup = value;
    }).catch(() => showTransientOperationNotice("Desktop event listener failed to start."));
    return () => { cancelled = true; cleanup(); };
  }, [showTransientOperationNotice]);

  useEffect(() => {
    document.documentElement.dataset.theme = preferences.theme;
  }, [preferences.theme]);

  const savePreferences = useCallback((next: WidgetPreferences) => {
    const previous = preferences;
    setPreferences(next);
    setOperationError(null);
    void updatePreferences(next).catch(() => {
      setPreferences(previous);
      showTransientOperationNotice("Settings could not be saved. Previous state restored.");
    });
  }, [preferences, showTransientOperationNotice]);

  const setTheme = useCallback((theme: ThemeName) => {
    if (preferences.theme === theme) return;
    savePreferences({ ...preferences, theme });
  }, [preferences, savePreferences]);

  const setProgressStyle = useCallback((progressStyle: ProgressStyle) => {
    if (preferences.progressStyle === progressStyle) return;
    savePreferences({ ...preferences, progressStyle });
  }, [preferences, savePreferences]);

  const toggleAlwaysOnTop = useCallback(() => {
    setOperationError(null);
    void setAlwaysOnTop(!preferences.alwaysOnTop)
      .then((value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }))
      .catch(() => showTransientOperationNotice("Always-on-top toggle failed."));
  }, [preferences.alwaysOnTop, showTransientOperationNotice]);

  return (
    <main className="settings-window">
      {operationError ? <div className="operation-notice operation-notice--settings" role="status">{operationError}</div> : null}
      <WidgetSettingsPanel
        preferences={preferences}
        onClose={() => void closeCurrentWindow()}
        onThemeChange={setTheme}
        onProgressStyleChange={setProgressStyle}
        onToggleAlwaysOnTop={toggleAlwaysOnTop}
        onToggleStayExpanded={() => savePreferences({ ...preferences, stayExpanded: !preferences.stayExpanded })}
        onToggleStatusBarProgress={() => savePreferences({ ...preferences, showStatusBarProgress: !preferences.showStatusBarProgress })}
        onAutoRotateChange={(autoRotateSeconds) => savePreferences({ ...preferences, autoRotateSeconds })}
        onExpandedSizeChange={(expandedSize) => savePreferences({ ...preferences, expandedSize })}
      />
    </main>
  );
}
