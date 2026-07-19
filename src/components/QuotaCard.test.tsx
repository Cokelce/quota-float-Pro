/**
 * @vitest-environment jsdom
 */
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { ProviderSnapshot, WidgetPreferences } from "../types";
import { QuotaCard, QuotaOrb } from "./QuotaCard";

const preferences: WidgetPreferences = {
  locked: false,
  alwaysOnTop: true,
  stayExpanded: false,
  pinnedProvider: null,
  autoRotateSeconds: 12,
  language: "zh-CN",
  theme: "aurora",
  progressStyle: "solid",
};

const apiBalance: ProviderSnapshot = {
  provider: "codex",
  displayName: "CODEX",
  plan: "API",
  shortWindow: null,
  weeklyWindow: null,
  balance: "$35.00",
  balancePercent: 35,
  resetCredits: null,
  resetCreditExpiresAt: [],
  updatedAt: "2026-07-19T00:00:00Z",
  status: "ok",
  message: null,
};

describe("QuotaCard API balance styling", () => {
  it("uses balance progress for the card color tier", () => {
    const { container } = render(
      <QuotaCard
        snapshot={apiBalance}
        preferences={preferences}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onTogglePin={() => {}}
        onLock={() => {}}
        onToggleStayExpanded={() => {}}
        onDrag={() => {}}
        onHover={() => {}}
      />,
    );

    expect(container.querySelector(".quota-card")?.classList.contains("quota-card--caution")).toBe(true);
  });

  it("uses balance progress for the orb color tier", () => {
    const { container } = render(<QuotaOrb snapshot={{ ...apiBalance, balancePercent: 8 }} onDrag={() => {}} onHover={() => {}} />);

    expect(container.querySelector(".quota-orb")?.classList.contains("quota-card--critical")).toBe(true);
  });

  it("shows api balance progress as a lower-left percentage", () => {
    const { container } = render(
      <QuotaCard
        snapshot={apiBalance}
        preferences={preferences}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onTogglePin={() => {}}
        onLock={() => {}}
        onToggleStayExpanded={() => {}}
        onDrag={() => {}}
        onHover={() => {}}
      />,
    );

    expect(container.querySelector(".weekly-value--balance")?.textContent).toBe("35%");
  });

  it("applies the segmented progress style preference", () => {
    const { container } = render(
      <QuotaCard
        snapshot={apiBalance}
        preferences={{ ...preferences, progressStyle: "segmented" }}
        providerCount={1}
        onPrevious={() => {}}
        onNext={() => {}}
        onTogglePin={() => {}}
        onLock={() => {}}
        onToggleStayExpanded={() => {}}
        onDrag={() => {}}
        onHover={() => {}}
      />,
    );

    expect(container.querySelector(".progress")?.classList.contains("progress--segmented")).toBe(true);
  });
});
