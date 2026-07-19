import { beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  calls: [] as string[],
  eventHandlers: new Map<string, (event: { payload: unknown }) => void>(),
  unlisteners: [] as string[],
  invoke: vi.fn(async (command: string) => {
    api.calls.push(`start:${command}`);
    await Promise.resolve();
    api.calls.push(`end:${command}`);
  }),
  currentMonitor: vi.fn(async () => ({
    workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1040 } },
  })),
  listen: vi.fn(async (event: string, handler: (event: { payload: unknown }) => void) => {
    api.eventHandlers.set(event, handler);
    return () => api.unlisteners.push(event);
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: api.invoke }));
vi.mock("@tauri-apps/api/window", () => ({ currentMonitor: api.currentMonitor }));
vi.mock("@tauri-apps/api/event", () => ({ listen: api.listen }));

beforeEach(() => {
  vi.clearAllMocks();
  api.calls.length = 0;
  api.eventHandlers.clear();
  api.unlisteners.length = 0;
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
});

describe("widget transitions", () => {
  it("passes the monitor work area to the Rust expansion command", async () => {
    const { setWidgetExpanded } = await import("./bridge");
    await setWidgetExpanded(true);
    expect(api.invoke).toHaveBeenCalledWith("expand_widget", {
      workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1040 } },
    });
  });

  it("passes an optional settings panel size to the Rust expansion command", async () => {
    const { setWidgetExpanded } = await import("./bridge");
    await setWidgetExpanded(true, { width: 460, height: 520 });
    expect(api.invoke).toHaveBeenCalledWith("expand_widget", {
      workArea: { position: { x: 0, y: 0 }, size: { width: 1920, height: 1040 } },
      logicalSize: { width: 460, height: 520 },
    });
  });

  it("serializes rapid expand and collapse requests", async () => {
    const { setWidgetExpanded } = await import("./bridge");
    await Promise.all([setWidgetExpanded(true), setWidgetExpanded(false)]);
    expect(api.calls).toEqual([
      "start:expand_widget",
      "end:expand_widget",
      "start:collapse_widget",
      "end:collapse_widget",
    ]);
  });

  it("listens for tray theme panel requests", async () => {
    const { listenDesktopEvents } = await import("./bridge");
    const onThemePanel = vi.fn();
    const cleanup = await listenDesktopEvents({
      onPreferences: vi.fn(),
      onRefresh: vi.fn(),
      onUpdate: vi.fn(),
      onThemePanel,
    });

    api.eventHandlers.get("theme-panel-requested")?.({ payload: null });
    cleanup();

    expect(onThemePanel).toHaveBeenCalledTimes(1);
    expect(api.unlisteners).toContain("theme-panel-requested");
  });
});
