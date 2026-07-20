import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { ProviderSnapshot, WidgetPreferences } from "./types";

interface TrayPreviewPayload {
  snapshot: ProviderSnapshot;
  preferences: WidgetPreferences;
  percent: number;
  tier: string;
  label: string;
  value: string;
  detail: string;
}

const fallback: TrayPreviewPayload = {
  snapshot: {
    provider: "codex",
    displayName: "CODEX",
    plan: "API",
    shortWindow: null,
    weeklyWindow: null,
    balance: null,
    resetCredits: null,
    resetCreditExpiresAt: [],
    updatedAt: new Date().toISOString(),
    status: "loading",
    message: "读取中",
  },
  preferences: {
    locked: false,
    alwaysOnTop: true,
    stayExpanded: false,
    showStatusBarProgress: true,
    pinnedProvider: null,
    autoRotateSeconds: 12,
    expandedSize: 320,
    language: "zh-CN",
    theme: "aurora",
    progressStyle: "solid",
  },
  percent: 100,
  tier: "healthy",
  label: "额度",
  value: "--",
  detail: "正在读取额度",
};

export function StatusBarApp() {
  const [preview, setPreview] = useState(fallback);

  useEffect(() => {
    document.documentElement.dataset.theme = preview.preferences.theme;
  }, [preview.preferences.theme]);

  useEffect(() => {
    let mounted = true;
    let cleanup = () => {};
    void listen<TrayPreviewPayload>("tray-preview-updated", (event) => {
      if (mounted) setPreview(event.payload);
    }).then((unlisten) => {
      cleanup = unlisten;
    });
    return () => {
      mounted = false;
      cleanup();
    };
  }, []);

  return (
    <main className={`tray-preview quota-card--${preview.snapshot.status} quota-card--${preview.tier}`}>
      <div className="aurora" aria-hidden="true" />
      <header className="tray-preview-header">
        <p>{preview.snapshot.displayName} · {preview.snapshot.plan ?? "ACCOUNT"}</p>
        <i aria-hidden="true" />
      </header>
      <section className="tray-preview-value">
        <small>{preview.label}</small>
        <strong>{preview.value}</strong>
      </section>
      <div className="tray-preview-progress" aria-hidden="true">
        <span style={{ width: `${preview.percent}%` }} />
      </div>
    </main>
  );
}
