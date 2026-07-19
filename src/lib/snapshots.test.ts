import { describe, expect, it } from "vitest";
import type { ProviderSnapshot } from "../types";
import { applyApiBalanceProgress, loadApiBalanceBaselines, mergeSnapshots, saveApiBalanceBaselines } from "./snapshots";

const success: ProviderSnapshot = {
  provider: "codex",
  displayName: "CODEX",
  plan: "PRO",
  shortWindow: { remainingPercent: 74, resetsAt: "2026-07-07T02:00:00Z", windowSeconds: 18_000 },
  weeklyWindow: { remainingPercent: 42, resetsAt: "2026-07-10T00:00:00Z", windowSeconds: 604_800 },
  resetCredits: 1,
  updatedAt: "2026-07-07T00:00:00Z",
  status: "ok",
  message: null,
};

describe("snapshot failure handling", () => {
  it("retains the last successful values during a transient failure", () => {
    const failure: ProviderSnapshot = { ...success, shortWindow: null, weeklyWindow: null, status: "unavailable", message: "Network unavailable", updatedAt: "2026-07-07T01:00:00Z" };
    expect(mergeSnapshots([success], [failure])[0]).toEqual({ ...success, status: "stale", message: "Network unavailable" });
  });

  it("shows a failure when no successful snapshot exists", () => {
    const signedOut: ProviderSnapshot = { ...success, shortWindow: null, weeklyWindow: null, status: "signed_out", message: "Please sign in" };
    expect(mergeSnapshots([], [signedOut])[0].status).toBe("signed_out");
  });

  it("does not hide an expired login behind stale quota data", () => {
    const signedOut: ProviderSnapshot = { ...success, shortWindow: null, weeklyWindow: null, status: "signed_out", message: "Please sign in" };
    expect(mergeSnapshots([success], [signedOut])[0].status).toBe("signed_out");
  });

  it("replaces stale data after recovery", () => {
    expect(mergeSnapshots([{ ...success, status: "stale" }], [{ ...success, shortWindow: { ...success.shortWindow!, remainingPercent: 88 } }])[0].shortWindow?.remainingPercent).toBe(88);
  });

  it("treats the first api balance as 100 and scales down from the highest seen balance", () => {
    const baselines = new Map<string, number>();
    const first = applyApiBalanceProgress([
      { provider: "codex", displayName: "CODEX", plan: "API", shortWindow: null, weeklyWindow: null, balance: "$233.35", balanceSourceKey: "api-a", resetCredits: null, resetCreditExpiresAt: [], updatedAt: "2026-07-07T00:00:00Z", status: "ok", message: null }
    ], baselines)[0];
    expect(first.balancePercent).toBe(100);

    const second = applyApiBalanceProgress([
      { provider: "codex", displayName: "CODEX", plan: "API", shortWindow: null, weeklyWindow: null, balance: "$116.68", balanceSourceKey: "api-a", resetCredits: null, resetCreditExpiresAt: [], updatedAt: "2026-07-07T00:10:00Z", status: "ok", message: null }
    ], baselines)[0];
    expect(second.balancePercent).toBeCloseTo(50, 0);

    const topUp = applyApiBalanceProgress([
      { provider: "codex", displayName: "CODEX", plan: "API", shortWindow: null, weeklyWindow: null, balance: "$300.00", balanceSourceKey: "api-a", resetCredits: null, resetCreditExpiresAt: [], updatedAt: "2026-07-07T00:20:00Z", status: "ok", message: null }
    ], baselines)[0];
    expect(topUp.balancePercent).toBe(100);

    const otherApi = applyApiBalanceProgress([
      { provider: "codex", displayName: "CODEX", plan: "API", shortWindow: null, weeklyWindow: null, balance: "$50.00", balanceSourceKey: "api-b", resetCredits: null, resetCreditExpiresAt: [], updatedAt: "2026-07-07T00:30:00Z", status: "ok", message: null }
    ], baselines)[0];
    expect(otherApi.balancePercent).toBe(100);
  });

  it("keeps the api balance baseline across app restarts", () => {
    const store = new Map<string, string>([["quota-float.apiBalanceBaselines.v5", JSON.stringify({ "api-a": 200 })]]);
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
    };

    const reloaded = loadApiBalanceBaselines(storage);
    const snapshot = applyApiBalanceProgress([
      { provider: "codex", displayName: "CODEX", plan: "API", shortWindow: null, weeklyWindow: null, balance: "$161.24", balanceSourceKey: "api-a", resetCredits: null, resetCreditExpiresAt: [], updatedAt: "2026-07-07T00:40:00Z", status: "ok", message: null }
    ], reloaded)[0];

    expect(snapshot.balancePercent).toBeCloseTo(80.62, 2);
  });

  it("ignores old poisoned api balance baseline caches", () => {
    const store = new Map<string, string>([
      ["quota-float.apiBalanceBaselines.v2", JSON.stringify({ "api-a": 895 })],
      ["quota-float.apiBalanceBaselines.v3", JSON.stringify({ "api-a": 895 })],
      ["quota-float.apiBalanceBaselines.v4", JSON.stringify({ "api-a": 895 })],
    ]);
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
    };

    const reloaded = loadApiBalanceBaselines(storage);
    const snapshot = applyApiBalanceProgress([
      { provider: "codex", displayName: "CODEX", plan: "API", shortWindow: null, weeklyWindow: null, balance: "$170.03", balanceSourceKey: "api-a", resetCredits: null, resetCreditExpiresAt: [], updatedAt: "2026-07-07T00:45:00Z", status: "ok", message: null }
    ], reloaded)[0];

    expect(snapshot.balancePercent).toBe(100);
    expect(store.has("quota-float.apiBalanceBaselines.v2")).toBe(false);
    expect(store.has("quota-float.apiBalanceBaselines.v3")).toBe(false);
    expect(store.has("quota-float.apiBalanceBaselines.v4")).toBe(false);
  });

  it("writes only the v5 api balance baseline cache", () => {
    const store = new Map<string, string>([["quota-float.apiBalanceBaselines.v4", JSON.stringify({ "api-a": 895 })]]);
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => { store.set(key, value); },
      removeItem: (key: string) => { store.delete(key); },
    };

    saveApiBalanceBaselines(new Map([["api-a", 200]]), storage);

    expect(store.has("quota-float.apiBalanceBaselines.v4")).toBe(false);
    expect(store.get("quota-float.apiBalanceBaselines.v5")).toBe(JSON.stringify({ "api-a": 200 }));
  });

  it("ignores provider balance percent and only uses the local highest balance", () => {
    const baselines = new Map<string, number>();
    const snapshot = applyApiBalanceProgress([
      { provider: "codex", displayName: "CODEX", plan: "API", shortWindow: null, weeklyWindow: null, balance: "$176.00", balancePercent: 19, balanceSourceKey: "api-a", resetCredits: null, resetCreditExpiresAt: [], updatedAt: "2026-07-07T00:50:00Z", status: "ok", message: null }
    ], baselines)[0];

    expect(snapshot.balancePercent).toBe(100);
    expect(baselines.get("api-a")).toBe(176);
  });
});
