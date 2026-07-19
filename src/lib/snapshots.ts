import type { ProviderSnapshot } from "../types";

const API_BALANCE_BASELINES_KEY = "quota-float.apiBalanceBaselines.v5";
const LEGACY_API_BALANCE_BASELINE_KEYS = [
  "quota-float.apiBalanceBaselines.v1",
  "quota-float.apiBalanceBaselines.v2",
  "quota-float.apiBalanceBaselines.v3",
  "quota-float.apiBalanceBaselines.v4",
];

interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem?(key: string): void;
}

export function mergeSnapshots(current: ProviderSnapshot[], incoming: ProviderSnapshot[]): ProviderSnapshot[] {
  return incoming.map((next) => {
    if (next.status === "ok") return next;
    if (next.status === "signed_out") return next;
    const previous = current.find((item) => item.provider === next.provider && item.shortWindow);
    return previous
      ? { ...previous, status: "stale", message: next.message, updatedAt: previous.updatedAt }
      : next;
  });
}

function parseBalance(value: string): number | null {
  const match = /-?\d+(?:\.\d+)?/.exec(value.replace(/,/g, ""));
  if (!match) return null;
  const number = Number(match[0]);
  return Number.isFinite(number) ? number : null;
}

function browserStorage(): StorageLike | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

export function loadApiBalanceBaselines(storage: StorageLike | null = browserStorage()): Map<string, number> {
  const baselines = new Map<string, number>();
  if (!storage) return baselines;
  try {
    for (const key of LEGACY_API_BALANCE_BASELINE_KEYS) {
      storage.removeItem?.(key);
    }
    const raw = storage.getItem(API_BALANCE_BASELINES_KEY);
    if (!raw) return baselines;
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return baselines;
    for (const [key, value] of Object.entries(parsed)) {
      if (typeof key !== "string" || typeof value !== "number") continue;
      if (Number.isFinite(value) && value > 0) baselines.set(key, value);
    }
  } catch {
    return baselines;
  }
  return baselines;
}

export function saveApiBalanceBaselines(baselines: Map<string, number>, storage: StorageLike | null = browserStorage()): void {
  if (!storage) return;
  const values: Record<string, number> = {};
  for (const [key, value] of baselines) {
    if (Number.isFinite(value) && value > 0) values[key] = value;
  }
  try {
    for (const key of LEGACY_API_BALANCE_BASELINE_KEYS) {
      storage.removeItem?.(key);
    }
    storage.setItem(API_BALANCE_BASELINES_KEY, JSON.stringify(values));
  } catch {
    // Local storage is best-effort; quota display still works during the current run.
  }
}

export function applyApiBalanceProgress(incoming: ProviderSnapshot[], baselines: Map<string, number>): ProviderSnapshot[] {
  return incoming.map((snapshot) => {
    if (snapshot.shortWindow || snapshot.weeklyWindow) return snapshot;
    const balance = snapshot.balance?.trim();
    if (!balance) return snapshot;
    const currentBalance = parseBalance(balance);
    if (currentBalance === null) return snapshot;
    const key = snapshot.balanceSourceKey ?? snapshot.provider;
    const previous = baselines.get(key);
    const nextBaseline = previous == null || currentBalance > previous ? currentBalance : previous;
    baselines.set(key, nextBaseline);
    return { ...snapshot, balancePercent: nextBaseline > 0 ? (currentBalance / nextBaseline) * 100 : 0 };
  });
}
