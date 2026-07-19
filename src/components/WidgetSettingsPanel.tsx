import { Check, X } from "@phosphor-icons/react";
import { copy, normalizeLanguage } from "../lib/i18n";
import type { ProgressStyle, ThemeName, WidgetPreferences } from "../types";

interface Props {
  preferences: WidgetPreferences;
  onClose: () => void;
  onThemeChange: (theme: ThemeName) => void;
  onProgressStyleChange: (style: ProgressStyle) => void;
  onToggleAlwaysOnTop: () => void;
  onToggleStayExpanded: () => void;
  onAutoRotateChange: (seconds: number) => void;
}

export function WidgetSettingsPanel({
  preferences,
  onClose,
  onThemeChange,
  onProgressStyleChange,
  onToggleAlwaysOnTop,
  onToggleStayExpanded,
  onAutoRotateChange,
}: Props) {
  const language = normalizeLanguage(preferences.language);
  const t = copy[language];
  const themeOptions: Array<{ value: ThemeName; label: string; description: string; colors: string[] }> = [
    { value: "aurora", label: t.themeAurora, description: t.themeAuroraDescription, colors: ["#B9D5EE", "#DFF4E5", "#91BAF0"] },
    { value: "dark", label: t.themeDark, description: t.themeDarkDescription, colors: ["#101927", "#23395F", "#7EE7C7"] },
    { value: "qingci", label: t.themeQingci, description: t.themeQingciDescription, colors: ["#C2DB50", "#BED09B", "#8EB722"] },
    { value: "bamboo", label: t.themeBamboo, description: t.themeBambooDescription, colors: ["#327B42", "#4B7748", "#A4BC6E"] },
    { value: "peacock", label: t.themePeacock, description: t.themePeacockDescription, colors: ["#007C62", "#408D63", "#8BB798"] },
    { value: "lvyun", label: t.themeLvyun, description: t.themeLvyunDescription, colors: ["#1C241B", "#2E372B", "#4E6748"] },
    { value: "xinghe", label: t.themeXinghe, description: t.themeXingheDescription, colors: ["#2B1C70", "#C855F8", "#58D8FF"] },
  ];

  return (
    <div className="widget-settings-backdrop" role="presentation" onMouseDown={onClose}>
      <section
        className="widget-settings-sheet"
        role="dialog"
        aria-modal="true"
        aria-label={t.themePanelTitle}
        onMouseDown={(event) => event.stopPropagation()}
      >
        <header className="widget-settings-header">
          <div>
            <p className="widget-settings-kicker">{t.themePanelKicker}</p>
            <h2>{t.themePanelTitle}</h2>
          </div>
          <button type="button" className="widget-settings-close" onClick={onClose} aria-label={t.close}>
            <X weight="bold" />
          </button>
        </header>

        <div className="widget-settings-group">
          <div className="widget-settings-label-row">
            <div>
              <strong>{t.themeSectionTitle}</strong>
              <p>{t.themeSectionDescription}</p>
            </div>
          </div>
          <div className="theme-choice-grid" role="group" aria-label={t.themeSectionTitle}>
            {themeOptions.map((option) => (
              <button
                key={option.value}
                type="button"
                className={preferences.theme === option.value ? "theme-choice is-active" : "theme-choice"}
                onClick={() => onThemeChange(option.value)}
                aria-pressed={preferences.theme === option.value}
              >
                <span className="theme-choice-swatches" aria-hidden="true">
                  {option.colors.map((color) => <i key={color} style={{ background: color }} />)}
                </span>
                <span className="theme-choice-copy">
                  <strong>{option.label}</strong>
                  <small>{option.description}</small>
                </span>
                {preferences.theme === option.value ? <Check weight="bold" /> : null}
              </button>
            ))}
          </div>
        </div>

        <div className="widget-settings-group">
          <div className="widget-settings-label-row">
            <div>
              <strong>{t.progressStyleSectionTitle}</strong>
              <p>{t.progressStyleSectionDescription}</p>
            </div>
          </div>
          <div className="progress-style-grid" role="group" aria-label={t.progressStyleSectionTitle}>
            <button
              type="button"
              className={preferences.progressStyle === "solid" ? "progress-style-choice is-active" : "progress-style-choice"}
              onClick={() => onProgressStyleChange("solid")}
              aria-pressed={preferences.progressStyle === "solid"}
            >
              <span className="progress-style-preview progress-style-preview--solid"><i /></span>
              <span>{t.progressStyleSolid}</span>
            </button>
            <button
              type="button"
              className={preferences.progressStyle === "segmented" ? "progress-style-choice is-active" : "progress-style-choice"}
              onClick={() => onProgressStyleChange("segmented")}
              aria-pressed={preferences.progressStyle === "segmented"}
            >
              <span className="progress-style-preview progress-style-preview--segmented"><i /></span>
              <span>{t.progressStyleSegmented}</span>
            </button>
          </div>
        </div>

        <div className="widget-settings-group">
          <div className="widget-settings-label-row">
            <div>
              <strong>{t.rotateSectionTitle}</strong>
              <p>{t.rotateSectionDescription}</p>
            </div>
            <output>{t.rotateSeconds(preferences.autoRotateSeconds)}</output>
          </div>
          <input
            className="widget-settings-range"
            type="range"
            min={5}
            max={60}
            step={5}
            value={Math.min(60, Math.max(5, preferences.autoRotateSeconds))}
            onChange={(event) => onAutoRotateChange(Number(event.target.value))}
            aria-label={t.rotateSectionTitle}
          />
        </div>

        <div className="widget-settings-group">
          <button type="button" className="widget-toggle-row" onClick={onToggleAlwaysOnTop} aria-pressed={preferences.alwaysOnTop}>
            <span className="widget-toggle-copy">
              <strong>{t.pinOn}</strong>
              <small>{t.themeAlwaysOnTopDescription}</small>
            </span>
            <span className={preferences.alwaysOnTop ? "widget-toggle-switch is-on" : "widget-toggle-switch"} aria-hidden="true">
              <i />
            </span>
          </button>
          <button type="button" className="widget-toggle-row" onClick={onToggleStayExpanded} aria-pressed={preferences.stayExpanded}>
            <span className="widget-toggle-copy">
              <strong>{t.keepExpandedOn}</strong>
              <small>{t.themeStayExpandedDescription}</small>
            </span>
            <span className={preferences.stayExpanded ? "widget-toggle-switch is-on" : "widget-toggle-switch"} aria-hidden="true">
              <i />
            </span>
          </button>
        </div>
      </section>
    </div>
  );
}


