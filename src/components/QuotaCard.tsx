import { ArrowClockwise, ArrowDown, ArrowUp, ArrowsInSimple, ArrowsOutSimple, ClockCounterClockwise, CloudSlash, Info, PushPin, PushPinSlash, SignIn, WarningCircle } from "@phosphor-icons/react";
import { memo, type CSSProperties, type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { clampPercent, formatDateTime, formatResetDate, formatResetTime, quotaTier } from "../lib/format";
import { copy, normalizeLanguage } from "../lib/i18n";
import type { Language, ProviderSnapshot, WidgetPreferences } from "../types";
import { ProviderMark } from "./ProviderMark";

interface Props {
  snapshot: ProviderSnapshot;
  preferences: WidgetPreferences;
  providerCount: number;
  onPrevious: () => void;
  onNext: () => void;
  onTogglePin: () => void;
  onLock: () => void;
  onToggleStayExpanded: () => void;
  onDrag: () => void;
  onHover: (hovered: boolean) => void;
  onRefresh?: () => void;
  isClosing?: boolean;
  isConsuming?: boolean;
  notice?: ReactNode;
  initialShowCreditTip?: boolean;
}

function StatusIcon({ status, expired = false }: { status: ProviderSnapshot["status"]; expired?: boolean }) {
  if (status === "signed_out") return <SignIn weight="duotone" />;
  if (status === "stale" || expired) return <ClockCounterClockwise weight="duotone" />;
  if (status === "unavailable") return <CloudSlash weight="duotone" />;
  return <WarningCircle weight="duotone" />;
}

function localizedBackendMessage(message: string | null, language: Language): string | null {
  if (!message) return null;
  if (language === "en") return message;
  const normalized = message.toLowerCase();
  if (normalized.includes("sign in") || normalized.includes("login")) return "Codex 登录已失效，请重新登录。";
  if (normalized.includes("rate limited")) return "请求过于频繁，将稍后自动重试。";
  if (normalized.includes("network")) return "网络不可用，将自动重试。";
  if (normalized.includes("format")) return "额度响应格式已变化。";
  if (normalized.includes("usage query") || normalized.includes("balance_url")) return "已连接 CC Switch，但未检测到 USD 余额接口。请在 CC Switch 开启 Usage Query，或在 Codex 配置 balance_url。";
  if (normalized.includes("balance endpoint") || normalized.includes("balance field")) return "已连接 API，但没有检测到可用的 USD 余额字段。";
  if (normalized.includes("missing the 5h")) return "额度响应缺少 5 小时窗口。";
  if (normalized.includes("refresh is already running")) return "额度正在刷新，请稍候。";
  return message;
}

function balanceNumber(value: string): number | null {
  const match = /-?\d+(?:\.\d+)?/.exec(value.replace(/,/g, ""));
  if (!match) return null;
  const number = Number(match[0]);
  return Number.isFinite(number) ? number : null;
}

function usdBalance(value: string): string {
  const number = balanceNumber(value);
  return number === null ? `${value} USD` : `$${number.toFixed(2)} USD`;
}

function progressStyle(percent: number): CSSProperties {
  return { width: `${percent}%`, "--progress-value": `${percent}%` } as CSSProperties;
}

export const QuotaCard = memo(function QuotaCard({
  snapshot,
  preferences,
  providerCount,
  onPrevious,
  onNext,
  onTogglePin: _onTogglePin,
  onLock,
  onToggleStayExpanded,
  onDrag,
  onHover,
  onRefresh,
  isClosing = false,
  isConsuming = false,
  notice = null,
  initialShowCreditTip = false,
}: Props) {
  const [showCreditTip, setShowCreditTip] = useState(initialShowCreditTip);
  const language = normalizeLanguage(preferences.language);
  const t = copy[language];
  const primary = snapshot.shortWindow ? clampPercent(snapshot.shortWindow.remainingPercent) : null;
  const weekly = snapshot.weeklyWindow ? clampPercent(snapshot.weeklyWindow.remainingPercent) : null;
  const displayPercent = primary ?? weekly;
  const balance = snapshot.balance?.trim() || null;
  const apiMetricLabel = snapshot.balanceLabel?.trim() || t.apiBalance;
  const currencyBalance = balance !== null && !snapshot.balanceLabel;
  const displayBalance = currencyBalance && balance ? usdBalance(balance) : balance;
  const compactBalance = displayBalance?.replace(/\s+USD$/, "") ?? null;
  const balanceProgress = snapshot.balancePercent == null ? 100 : clampPercent(snapshot.balancePercent);
  const displayWindow = snapshot.shortWindow ?? snapshot.weeklyWindow;
  const displayingWeeklyAsPrimary = primary === null && weekly !== null;
  const balanceAvailable = displayPercent === null && balance !== null && snapshot.status === "ok";
  const staleAge = Date.now() - new Date(snapshot.updatedAt).getTime();
  const staleExpired = snapshot.status === "stale" && staleAge > 30 * 60_000;
  const available = snapshot.status === "ok" || (snapshot.status === "stale" && !staleExpired);
  const tier = quotaTier(balanceAvailable ? balanceProgress : displayPercent);
  const progressClassName = preferences.progressStyle === "segmented" ? "progress progress--segmented" : "progress";
  const balanceProgressClassName = preferences.progressStyle === "segmented" ? "progress progress--balance progress--segmented" : "progress progress--balance";
  const indicatorState = isConsuming ? "active" : snapshot.status === "ok" ? "ok" : snapshot.status === "stale" ? "stale" : "error";
  const indicatorLabel = isConsuming
    ? t.active
    : snapshot.status === "ok"
      ? t.dataSynced
      : snapshot.status === "stale"
        ? t.dataStale
        : snapshot.status === "signed_out"
          ? t.notSignedIn
          : t.unavailableStatus;
  const message = localizedBackendMessage(snapshot.message, language);
  const creditExpirations = useMemo(() => (snapshot.resetCreditExpiresAt ?? []).map((value, index) => {
    return t.creditItem(index, formatDateTime(value, language));
  }), [language, snapshot.resetCreditExpiresAt, t]);

  return (
    <main
      className={`quota-card quota-card--${snapshot.status} quota-card--${tier}${isClosing ? " quota-card--closing" : ""}`}
      onMouseEnter={() => onHover(true)}
      onMouseLeave={() => onHover(false)}
    >
      <div className="aurora" aria-hidden="true" />
      <span className="sr-only" aria-live="polite">{available && displayPercent !== null ? (displayingWeeklyAsPrimary ? t.weeklyAvailableLabel(displayPercent) : t.availableLabel(displayPercent)) : balanceAvailable && displayBalance ? t.apiMetricLabel(apiMetricLabel, displayBalance) : message}</span>
      {notice ? <div className="operation-notice" role="status">{notice}</div> : null}
      <header className="card-header">
        <div className="card-drag-handle" onMouseDown={(event) => { if (event.button === 0) void onDrag(); }}>
          <p className="eyebrow">{snapshot.displayName} · {snapshot.plan ?? t.accountFallback}</p>
          {snapshot.status !== "stale" ? <p className="updated">{balanceAvailable ? apiMetricLabel : displayingWeeklyAsPrimary ? t.weeklyShortRemaining : t.shortRemaining}</p> : null}
        </div>
        {!preferences.locked ? (
          <nav className="card-actions" aria-label={t.controls} onMouseDown={(event) => event.stopPropagation()}>
            {providerCount > 1 ? <button onClick={onPrevious} aria-label={t.servicePrevious}><ArrowUp /></button> : null}
            {providerCount > 1 ? <button onClick={onNext} aria-label={t.serviceNext}><ArrowDown /></button> : null}
            <span className={`usage-indicator usage-indicator--${indicatorState}`} role="status" aria-label={indicatorLabel} title={indicatorLabel}><i /></span>
            <button className={preferences.stayExpanded ? "expand-button expand-button--active" : "expand-button"} onClick={onToggleStayExpanded} aria-pressed={preferences.stayExpanded} aria-label={preferences.stayExpanded ? t.keepExpandedOff : t.keepExpandedOn} title={preferences.stayExpanded ? t.keepExpandedOff : t.keepExpandedOn}>
              {preferences.stayExpanded ? <ArrowsInSimple weight="bold" /> : <ArrowsOutSimple />}
            </button>
            <button className={preferences.alwaysOnTop ? "pin-button pin-button--active" : "pin-button"} onClick={onLock} aria-pressed={preferences.alwaysOnTop} aria-label={preferences.alwaysOnTop ? t.pinOff : t.pinOn} title={preferences.alwaysOnTop ? t.pinOff : t.pinOn}>
              {preferences.alwaysOnTop ? <PushPin weight="fill" /> : <PushPinSlash />}
            </button>
          </nav>
        ) : null}
      </header>

      {available && displayPercent !== null ? (
        <>
          <section className="primary-metric" aria-label={displayingWeeklyAsPrimary ? t.weeklyAvailableLabel(displayPercent) : t.availableLabel(displayPercent)}>
            <span>{displayPercent}</span><small>%</small>
          </section>
          <div className={progressClassName} role="progressbar" aria-label={displayingWeeklyAsPrimary ? t.weeklyAvailableLabel(displayPercent) : t.availableLabel(displayPercent)} aria-valuemin={0} aria-valuemax={100} aria-valuenow={displayPercent}>
            <span style={progressStyle(displayPercent)} />
          </div>
          <p className="reset-time">{formatResetTime(displayWindow?.resetsAt ?? null, new Date(), language)}{displayWindow?.resetsAt ? ` · ${formatDateTime(displayWindow.resetsAt, language)}` : ""}</p>
          <footer className="card-footer">
            <div className="weekly-metric">
              {displayingWeeklyAsPrimary ? <p className="weekly-note"><Info weight="bold" aria-hidden="true" />{t.shortWindowUnavailable}</p> : <p>{t.weeklyUntil(formatResetDate(snapshot.weeklyWindow?.resetsAt ?? null, language))}</p>}
              <strong className={displayingWeeklyAsPrimary ? "weekly-value--unavailable" : undefined}>{displayingWeeklyAsPrimary ? "--" : weekly ?? "--"}<small>{displayingWeeklyAsPrimary || weekly === null ? "" : "%"}</small></strong>
              <div className="reset-credit-row" onMouseDown={(event) => event.stopPropagation()}>
                <span>{snapshot.resetCredits === null ? t.resetCreditUnknown : t.resetCredits(snapshot.resetCredits)}</span>
                {snapshot.resetCredits !== null && snapshot.resetCredits > 0 ? (
                  <button type="button" className="reset-credit-button" onClick={() => setShowCreditTip((value) => !value)} aria-expanded={showCreditTip} aria-label={t.view}>{t.view}</button>
                ) : null}
              </div>
              {showCreditTip ? (
                <div className="reset-credit-tip" role="status" onMouseDown={(event) => event.stopPropagation()}>
                  {creditExpirations.length > 0 ? creditExpirations.map((item) => <p key={item}>{item}</p>) : <p>{t.noCreditExpiration}</p>}
                </div>
              ) : null}
            </div>
            <ProviderMark />
          </footer>
        </>
      ) : balanceAvailable && displayBalance ? (
        <>
          <section className="primary-metric primary-metric--balance" aria-label={t.apiMetricLabel(apiMetricLabel, displayBalance)}>
            <span>{compactBalance}</span>{currencyBalance ? <small>USD</small> : null}
          </section>
          <div className={balanceProgressClassName} aria-hidden="true">
            <span style={progressStyle(balanceProgress)} />
          </div>
          <p className="reset-time">{t.apiBalanceRemaining}</p>
          <footer className="card-footer">
            <div className="weekly-metric">
              <p>{t.progressRemaining}</p>
              <strong className="weekly-value--balance">{balanceProgress}<small>%</small></strong>
            </div>
            <ProviderMark />
          </footer>
        </>
      ) : (
        <section className="error-state" aria-live="polite">
          <div className="status-icon" aria-hidden="true"><StatusIcon status={snapshot.status} expired={staleExpired} /></div>
          <strong>{snapshot.status === "signed_out" ? t.signedInRequired : staleExpired ? t.staleExpired : t.temporarilyUnavailable}</strong>
          <p>{message ?? t.errorUnavailable}</p>
          {snapshot.status === "stale" ? (
            <button type="button" className="error-refresh-button" onMouseDown={(event) => event.stopPropagation()} onClick={onRefresh} disabled={!onRefresh} aria-label={t.refreshQuota}>
              <ArrowClockwise />
              <span>{t.refresh}</span>
            </button>
          ) : null}
        </section>
      )}
    </main>
  );
});

export const QuotaOrb = memo(function QuotaOrb({ snapshot, onDrag, onHover, language = "zh-CN" }: Pick<Props, "snapshot" | "onDrag" | "onHover"> & { language?: Language }) {
  const [idle, setIdle] = useState(false);
  const idleTimer = useRef<number | null>(null);
  const activeLanguage = normalizeLanguage(language);
  const t = copy[activeLanguage];
  const primary = snapshot.shortWindow ? clampPercent(snapshot.shortWindow.remainingPercent) : null;
  const weekly = snapshot.weeklyWindow ? clampPercent(snapshot.weeklyWindow.remainingPercent) : null;
  const displayPercent = primary ?? weekly;
  const balance = snapshot.balance?.trim() || null;
  const apiMetricLabel = snapshot.balanceLabel?.trim() || t.apiBalance;
  const currencyBalance = balance !== null && !snapshot.balanceLabel;
  const displayBalance = currencyBalance && balance ? usdBalance(balance) : balance;
  const compactBalance = displayBalance?.replace(/\s+USD$/, "") ?? null;
  const displayingWeeklyAsPrimary = primary === null && weekly !== null;
  const available = snapshot.status === "ok" && displayPercent !== null;
  const balanceAvailable = snapshot.status === "ok" && displayPercent === null && balance !== null;
  const balanceProgress = snapshot.balancePercent == null ? 100 : clampPercent(snapshot.balancePercent);
  const tier = quotaTier(balanceAvailable ? balanceProgress : displayPercent);

  useEffect(() => {
    idleTimer.current = window.setTimeout(() => setIdle(true), 2000);
    return () => {
      if (idleTimer.current !== null) window.clearTimeout(idleTimer.current);
    };
  }, []);

  const handleMouseEnter = () => {
    if (idleTimer.current !== null) window.clearTimeout(idleTimer.current);
    setIdle(false);
    onHover(true);
  };

  return (
    <main
      className={`quota-orb quota-card--${snapshot.status} quota-card--${tier}${displayingWeeklyAsPrimary ? " quota-orb--weekly" : ""}${idle ? " quota-orb--idle" : ""}`}
      onMouseEnter={handleMouseEnter}
      onMouseLeave={() => onHover(false)}
      onMouseDown={(event) => { if (event.button === 0) void onDrag(); }}
      aria-label={available ? (displayingWeeklyAsPrimary ? t.weeklyAvailableLabel(displayPercent!) : t.availableLabel(displayPercent!)) : balanceAvailable && displayBalance ? t.apiMetricLabel(apiMetricLabel, displayBalance) : localizedBackendMessage(snapshot.message, activeLanguage) ?? t.unavailableStatus}
    >
      <div className="aurora" aria-hidden="true" />
      {available && displayingWeeklyAsPrimary ? (
        <span className="orb-weekly-badge" aria-hidden="true">
          <svg viewBox="0 0 55 17" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path d="M7.3687 52.2894C13.0674 47.8486 17 38.4172 17 27.5C17 16.5828 13.0674 7.15141 7.3687 2.71063C3.88364 -0.00516105 0 3.58172 0 8L0 47C0 51.4183 3.88364 55.0052 7.3687 52.2894Z" fill="#61A1E2" fillOpacity=".8" transform="matrix(0 1 -1 0 55 0)" />
          </svg>
          <b>W</b>
        </span>
      ) : null}
      {available ? (
        <section className="orb-metric">
          <span>{displayPercent}</span>
          <small>%</small>
        </section>
      ) : balanceAvailable && displayBalance ? (
        <section className="orb-metric orb-metric--balance" aria-label={t.apiMetricLabel(apiMetricLabel, displayBalance)}>
          <span>{compactBalance}</span>
          <small>{currencyBalance ? "USD" : "API"}</small>
        </section>
      ) : (
        <section className="orb-unavailable">
          <StatusIcon status={snapshot.status} />
        </section>
      )}
    </main>
  );
});



