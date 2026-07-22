import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { QuotaCard, QuotaOrb } from "./components/QuotaCard";
import { fetchSnapshots, getPreferences, listenDesktopEvents, publishTrayPreview, setAlwaysOnTop, setTrayProgress, setWidgetExpanded, setWidgetVisible, startDragging, updatePreferences } from "./lib/bridge";
import { clampPercent, needsFastRefresh, quotaTier } from "./lib/format";
import { checkForAppUpdate, openReleasePage, type AvailableUpdateAction } from "./lib/appUpdate";
import { copy, normalizeLanguage } from "./lib/i18n";
import { applyApiBalanceProgress, loadApiBalanceBaselines, mergeSnapshots, saveApiBalanceBaselines } from "./lib/snapshots";
import type { ProviderSnapshot, WidgetPreferences } from "./types";

const DEFAULT_PREFS: WidgetPreferences = { locked: false, alwaysOnTop: true, stayExpanded: false, showStatusBarProgress: false, pinnedProvider: null, autoRotateSeconds: 12, expandedSize: 320, language: "zh-CN", theme: "aurora", progressStyle: "solid" };

export default function App() {
  const [snapshots, setSnapshots] = useState<ProviderSnapshot[]>([]);
  const [preferences, setPreferences] = useState(DEFAULT_PREFS);
  const [activeIndex, setActiveIndex] = useState(0);
  const [hovered, setHovered] = useState(false);
  const [compact, setCompact] = useState(true);
  const [closing, setClosing] = useState(false);
  const [consumingProviders, setConsumingProviders] = useState<Set<string>>(() => new Set());
  const [operationError, setOperationError] = useState<string | null>(null);
  const [showUpdateFallback, setShowUpdateFallback] = useState(false);
  const [pendingUpdate, setPendingUpdate] = useState<AvailableUpdateAction | null>(null);
  const failures = useRef(0);
  const previousPrimary = useRef(new Map<string, number>());
  const apiBalanceBaselines = useRef(loadApiBalanceBaselines());
  const consumptionTimers = useRef(new Map<string, number>());
  const collapseTimer = useRef<number | null>(null);
  const updateNoticeTimer = useRef<number | null>(null);
  const operationNoticeTimer = useRef<number | null>(null);
  const hoverSequence = useRef(0);
  const language = normalizeLanguage(preferences.language);
  const t = copy[language];

  const showTransientOperationNotice = useCallback((message: string, ttl = 3500) => {
    if (operationNoticeTimer.current !== null) {
      window.clearTimeout(operationNoticeTimer.current);
      operationNoticeTimer.current = null;
    }
    setShowUpdateFallback(false);
    setOperationError(message);
    operationNoticeTimer.current = window.setTimeout(() => {
      setOperationError((current) => (current === message ? null : current));
      operationNoticeTimer.current = null;
    }, ttl);
  }, []);

  const checkUpdate = useCallback((manual = false) => {
    setShowUpdateFallback(false);
    void checkForAppUpdate(language, {
      checking: t.updateChecking,
      current: t.updateCurrent,
      downloading: t.updateDownloading,
      installing: t.updateInstalling,
      availableWindows: t.updateAvailableWindows,
      availableMac: t.updateAvailableMac,
      failed: t.updateFailed,
    }, (message) => {
      if (updateNoticeTimer.current !== null) {
        window.clearTimeout(updateNoticeTimer.current);
        updateNoticeTimer.current = null;
      }
      setOperationError(message);
      if (message === t.updateFailed) setShowUpdateFallback(true);
      else setShowUpdateFallback(false);
      if (message === t.updateCurrent || message === t.updateFailed) setPendingUpdate(null);
      if (message === t.updateCurrent) {
        updateNoticeTimer.current = window.setTimeout(() => {
          setOperationError((current) => (current === message ? null : current));
          updateNoticeTimer.current = null;
        }, 1800);
      }
    }, (update) => {
      setPendingUpdate(update);
      if (!manual) return;
      if (collapseTimer.current !== null) {
        window.clearTimeout(collapseTimer.current);
        collapseTimer.current = null;
      }
      setClosing(false);
      setCompact(false);
      void setWidgetVisible(true).catch(() => undefined);
      void setWidgetExpanded(true, { width: preferences.expandedSize, height: preferences.expandedSize }).catch(() => undefined);
    }, manual);
  }, [language, preferences.expandedSize, t]);

  const hideUpdateNotice = useCallback(() => {
    setPendingUpdate(null);
    setOperationError(null);
    if (preferences.showStatusBarProgress) {
      void setWidgetExpanded(false).catch(() => undefined);
      void setWidgetVisible(false).catch(() => undefined);
    }
  }, [preferences.showStatusBarProgress]);

  const runPendingUpdate = useCallback(() => {
    if (!pendingUpdate) return;
    const update = pendingUpdate;
    setPendingUpdate(null);
    setShowUpdateFallback(false);
    void update.run()
      .then(() => {
        if (update.kind === "openRelease") hideUpdateNotice();
      })
      .catch(() => {
        setOperationError(t.updateFailed);
        setShowUpdateFallback(true);
      });
  }, [hideUpdateNotice, pendingUpdate, t.updateFailed]);

  const refresh = useCallback(async (force = false) => {
    try {
      const values = await fetchSnapshots(force);
      const balancedValues = applyApiBalanceProgress(values, apiBalanceBaselines.current);
      saveApiBalanceBaselines(apiBalanceBaselines.current);
      const hasFailure = balancedValues.some((item) => item.status !== "ok");
      if (hasFailure) failures.current += 1;
      else failures.current = 0;
      for (const item of balancedValues) {
        const nextPrimary = item.shortWindow?.remainingPercent;
        const previous = previousPrimary.current.get(item.provider);
        if (nextPrimary !== undefined && previous !== undefined && nextPrimary < previous) {
          setConsumingProviders((current) => new Set(current).add(item.provider));
          const oldTimer = consumptionTimers.current.get(item.provider);
          if (oldTimer !== undefined) window.clearTimeout(oldTimer);
          const timer = window.setTimeout(() => {
            setConsumingProviders((current) => { const next = new Set(current); next.delete(item.provider); return next; });
            consumptionTimers.current.delete(item.provider);
          }, 5 * 60_000);
          consumptionTimers.current.set(item.provider, timer);
        }
        if (nextPrimary !== undefined) previousPrimary.current.set(item.provider, nextPrimary);
      }
      setSnapshots((current) => mergeSnapshots(current, balancedValues));
    } catch {
      failures.current += 1;
      setSnapshots((current) => current.length > 0
        ? current.map((item) => ({ ...item, status: "stale", message: "Refresh failed. Please try again later." }))
        : [{ provider: "codex", displayName: "CODEX", plan: null, shortWindow: null, weeklyWindow: null, resetCredits: null, resetCreditExpiresAt: [], updatedAt: new Date().toISOString(), status: "unavailable", message: "Quota is temporarily unavailable. It will retry automatically." }]);
    }
  }, []);

  useEffect(() => {
    void refresh(true);
    void getPreferences().then((value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) })).catch(() => showTransientOperationNotice("Unable to read settings. Defaults are in use."));
    return () => {
      for (const timer of consumptionTimers.current.values()) window.clearTimeout(timer);
      consumptionTimers.current.clear();
      if (collapseTimer.current !== null) window.clearTimeout(collapseTimer.current);
      if (updateNoticeTimer.current !== null) window.clearTimeout(updateNoticeTimer.current);
      if (operationNoticeTimer.current !== null) window.clearTimeout(operationNoticeTimer.current);
    };
  }, [refresh, showTransientOperationNotice]);

  useEffect(() => {
    let cancelled = false;
    let cleanup: () => void = () => {};
    void listenDesktopEvents({
      onPreferences: (value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }),
      onRefresh: () => void refresh(true),
      onUpdate: () => checkUpdate(true),
    }).then((value) => {
      if (cancelled) value(); else cleanup = value;
    }).catch(() => showTransientOperationNotice("Desktop event listener failed to start."));
    return () => { cancelled = true; cleanup(); };
  }, [checkUpdate, refresh, showTransientOperationNotice]);

  useEffect(() => {
    const timer = window.setTimeout(() => checkUpdate(false), 12_000);
    return () => window.clearTimeout(timer);
  }, [checkUpdate]);

  const refreshMs = useMemo(() => {
    const backoff = failures.current === 0 ? 5 * 60_000 : Math.min(30 * 60_000, 30_000 * 2 ** (failures.current - 1));
    if (failures.current === 0 && snapshots.some((item) => item.status === "ok" && needsFastRefresh(item))) return 60_000;
    return backoff;
  }, [snapshots]);

  useEffect(() => {
    const id = window.setInterval(() => void refresh(), refreshMs);
    return () => window.clearInterval(id);
  }, [refresh, refreshMs]);

  useEffect(() => {
    const refreshWhenActive = () => { if (document.visibilityState === "visible") void refresh(true); };
    window.addEventListener("focus", refreshWhenActive);
    document.addEventListener("visibilitychange", refreshWhenActive);
    return () => {
      window.removeEventListener("focus", refreshWhenActive);
      document.removeEventListener("visibilitychange", refreshWhenActive);
    };
  }, [refresh]);

  useEffect(() => {
    if (hovered || preferences.pinnedProvider || snapshots.length < 2) return;
    const id = window.setInterval(() => setActiveIndex((value) => (value + 1) % snapshots.length), preferences.autoRotateSeconds * 1000);
    return () => window.clearInterval(id);
  }, [hovered, preferences.autoRotateSeconds, preferences.pinnedProvider, snapshots.length]);

  const current = preferences.pinnedProvider
    ? snapshots.find((item) => item.provider === preferences.pinnedProvider) ?? snapshots[0]
    : snapshots[activeIndex % Math.max(1, snapshots.length)];

  const trayWindowPercent = current?.shortWindow
    ? clampPercent(current.shortWindow.remainingPercent)
    : current?.weeklyWindow
      ? clampPercent(current.weeklyWindow.remainingPercent)
      : null;
  const trayBalance = current?.balance?.trim() || null;
  const trayBalancePercent = current?.balancePercent == null ? 100 : clampPercent(current.balancePercent);
  const trayPercent = trayWindowPercent ?? (trayBalance ? trayBalancePercent : 0);
  const trayTier = current ? quotaTier(trayBalance ? trayBalancePercent : trayWindowPercent) : "stale";
  const trayTooltip = current
    ? (trayWindowPercent !== null
      ? (current.shortWindow ? t.availableLabel(trayWindowPercent) : t.weeklyAvailableLabel(trayWindowPercent))
      : trayBalance
        ? `${current.balanceLabel?.trim() || t.apiBalance}: ${trayBalance}`
        : current.message ?? t.unavailableStatus)
    : t.loadingQuota;
  const trayPreview = useMemo(() => current ? {
    snapshot: current,
    preferences,
    percent: trayPercent,
    tier: trayTier,
    label: trayWindowPercent !== null
      ? (current.shortWindow ? t.shortRemaining : t.weeklyShortRemaining)
      : trayBalance
        ? current.balanceLabel?.trim() || t.apiBalance
        : t.unavailableStatus,
    value: trayWindowPercent !== null ? `${trayWindowPercent}%` : trayBalance ?? "--",
    detail: trayBalance ? `${trayBalancePercent}%` : trayTooltip,
  } : null, [current, preferences, t, trayBalance, trayBalancePercent, trayPercent, trayTier, trayTooltip, trayWindowPercent]);

  useEffect(() => {
    void setTrayProgress(preferences.showStatusBarProgress, trayPercent, trayTooltip, trayTier).catch(() => undefined);
  }, [preferences.showStatusBarProgress, trayPercent, trayTooltip, trayTier]);

  useEffect(() => {
    void setWidgetVisible(!preferences.showStatusBarProgress).catch(() => undefined);
  }, [preferences.showStatusBarProgress]);

  useEffect(() => {
    if (!preferences.showStatusBarProgress || !trayPreview) return;
    void publishTrayPreview(trayPreview).catch(() => undefined);
  }, [preferences.showStatusBarProgress, trayPreview]);

  const savePreferences = useCallback((next: WidgetPreferences) => {
    const previous = preferences;
    setPreferences(next);
    setOperationError(null);
    void updatePreferences(next).catch(() => { setPreferences(previous); showTransientOperationNotice("Settings could not be saved. Previous state restored."); });
  }, [preferences, showTransientOperationNotice]);

  const toggleAlwaysOnTop = useCallback(() => {
    setOperationError(null);
    void setAlwaysOnTop(!preferences.alwaysOnTop)
      .then((value) => setPreferences({ ...DEFAULT_PREFS, ...value, language: normalizeLanguage(value.language) }))
      .catch(() => showTransientOperationNotice("Always-on-top toggle failed."));
  }, [preferences.alwaysOnTop, showTransientOperationNotice]);

  const handleHover = useCallback((value: boolean) => {
    if (collapseTimer.current !== null) {
      window.clearTimeout(collapseTimer.current);
      collapseTimer.current = null;
    }
    setHovered(value);
    if (!value && preferences.stayExpanded) return;
    if (value) void refresh(true);
    if (value) {
      const sequence = ++hoverSequence.current;
      setClosing(false);
      setCompact(false);
      void setWidgetExpanded(true, { width: preferences.expandedSize, height: preferences.expandedSize })
        .then(() => {
          if (hoverSequence.current !== sequence) return;
        })
        .catch(() => {
          setCompact(true);
          if (hoverSequence.current === sequence) showTransientOperationNotice("Widget expand failed.");
        });
      return;
    }
    const sequence = ++hoverSequence.current;
    if (compact) return;
    setClosing(true);
    collapseTimer.current = window.setTimeout(() => {
      if (hoverSequence.current !== sequence) return;
      collapseTimer.current = null;
      void setWidgetExpanded(false)
        .then(() => {
          if (hoverSequence.current === sequence) {
            setClosing(false);
            setCompact(true);
          }
        })
        .catch(() => {
          setClosing(false);
          setCompact(true);
          showTransientOperationNotice("Widget collapse failed.");
        });
    }, 130);
  }, [compact, preferences.expandedSize, preferences.stayExpanded, refresh, showTransientOperationNotice]);

  useEffect(() => {
    if (!preferences.stayExpanded) return;
    if (collapseTimer.current !== null) window.clearTimeout(collapseTimer.current);
    setClosing(false);
    setCompact(false);
    void setWidgetExpanded(true, { width: preferences.expandedSize, height: preferences.expandedSize }).catch(() => showTransientOperationNotice("Widget expand failed."));
  }, [preferences.expandedSize, preferences.stayExpanded, showTransientOperationNotice]);

  useEffect(() => {
    document.documentElement.dataset.theme = preferences.theme;
  }, [preferences.theme]);

  if (!current) return <div className="loading-card" aria-label={t.loadingQuota}><span /><span /><span /></div>;

  if (preferences.showStatusBarProgress && !pendingUpdate && !operationError) return null;

  if (compact) {
    return <QuotaOrb snapshot={current} language={language} onDrag={() => startDragging()} onHover={handleHover} />;
  }

  return (
    <QuotaCard
      snapshot={current}
      preferences={preferences}
      providerCount={snapshots.length}
      onPrevious={() => setActiveIndex((value) => (value - 1 + snapshots.length) % snapshots.length)}
      onNext={() => setActiveIndex((value) => (value + 1) % snapshots.length)}
      onTogglePin={() => savePreferences({ ...preferences, pinnedProvider: preferences.pinnedProvider ? null : current.provider })}
      onToggleStayExpanded={() => savePreferences({ ...preferences, stayExpanded: !preferences.stayExpanded })}
      onLock={toggleAlwaysOnTop}
      onDrag={() => startDragging()}
      onHover={handleHover}
      onRefresh={() => refresh(true)}
      isClosing={closing}
      isConsuming={consumingProviders.has(current.provider)}
      notice={pendingUpdate ? <><span>{pendingUpdate.message}</span><button type="button" onMouseDown={(event) => event.stopPropagation()} onClick={runPendingUpdate}>{pendingUpdate.kind === "install" ? t.updateInstall : t.updateOpen}</button><button type="button" onMouseDown={(event) => event.stopPropagation()} onClick={hideUpdateNotice}>{t.updateLater}</button></> : showUpdateFallback && operationError ? <><span>{operationError}</span><button type="button" onMouseDown={(event) => event.stopPropagation()} onClick={() => void openReleasePage().catch(() => setOperationError("Could not open GitHub Releases."))}>GitHub Releases</button></> : operationError}
    />
  );
}


