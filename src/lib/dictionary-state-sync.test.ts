import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createDictionaryStateSync } from "./dictionary-state-sync";
import { DEFAULT_SNAPSHOT } from "../types/contracts";
import {
  DEFAULT_DICTIONARY_STATE,
  DICTIONARY_EVENT_NAMES,
  DICTIONARY_STATE_CHANGED_EVENT,
  type DictionaryState,
} from "../types/dictionary";

const { invokeCommand, listenTo } = vi.hoisted(() => ({
  invokeCommand: vi.fn(),
  listenTo: vi.fn<(name: string, handler: (payload: unknown) => void) => Promise<() => void>>(),
}));
vi.mock("./tauri", () => ({ invokeCommand, listenTo }));

const preparing: DictionaryState = { ...DEFAULT_DICTIONARY_STATE, status: "preparing" };
const ready: DictionaryState = { ...preparing, status: "ready", installedRelease: "v1" };
const handlers = new Map<string, (payload: unknown) => void>();
const unlisteners: Array<ReturnType<typeof vi.fn>> = [];
const cleanups: Array<() => void> = [];

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function createSync() {
  const onChange = vi.fn<(state: DictionaryState) => void>();
  const onUpdate = vi.fn();
  const onError = vi.fn();
  const sync = createDictionaryStateSync(DEFAULT_DICTIONARY_STATE, onChange);
  const subscribe = () => {
    const stop = sync.subscribe(onUpdate, onError);
    cleanups.push(stop);
    return stop;
  };
  return { sync, subscribe, onChange, onUpdate, onError };
}

function emitStateChanged() {
  handlers.get(DICTIONARY_STATE_CHANGED_EVENT)!(null);
}

beforeEach(() => {
  vi.useFakeTimers();
  invokeCommand.mockReset().mockResolvedValue(preparing);
  listenTo.mockReset().mockImplementation(async (name, handler) => {
    handlers.set(name, handler);
    const unlisten = vi.fn();
    unlisteners.push(unlisten);
    return unlisten;
  });
});

afterEach(() => {
  cleanups.splice(0).forEach((stop) => stop());
  handlers.clear();
  unlisteners.length = 0;
  vi.useRealTimers();
});

describe("dictionary state subscription", () => {
  it("registers before reading and recovers a notification sent before registration", async () => {
    const registration = deferred<() => void>();
    const unlisten = vi.fn();
    listenTo.mockImplementationOnce((_name, handler) => {
      handlers.set(DICTIONARY_STATE_CHANGED_EVENT, handler);
      return registration.promise;
    });
    const { subscribe, onChange } = createSync();
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(listenTo.mock.calls[0]?.[0]).toBe(DICTIONARY_STATE_CHANGED_EVENT);
    expect(invokeCommand).not.toHaveBeenCalled();

    // Backend initialization has finished before this listener is registered.
    invokeCommand.mockResolvedValue(ready);
    registration.resolve(unlisten);
    await vi.advanceTimersByTimeAsync(0);
    expect(invokeCommand).toHaveBeenCalledExactlyOnceWith("get_dictionary_state");
    expect(onChange).toHaveBeenLastCalledWith(ready);
  });

  it("rereads on each null notification and never periodically queries while preparing", async () => {
    const { subscribe, onChange, onUpdate } = createSync();
    subscribe();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(invokeCommand).toHaveBeenCalledExactlyOnceWith("get_dictionary_state");
    expect(onChange).toHaveBeenLastCalledWith(preparing);
    expect(vi.getTimerCount()).toBe(0);

    invokeCommand.mockResolvedValueOnce(ready).mockResolvedValueOnce(DEFAULT_DICTIONARY_STATE);
    emitStateChanged();
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenLastCalledWith(ready);
    emitStateChanged();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(onChange).toHaveBeenLastCalledWith(DEFAULT_DICTIONARY_STATE);
    expect(invokeCommand).toHaveBeenCalledTimes(3);
    expect(onUpdate).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(["state", "error"])("discards an old %s response after a newer notification read", async (outcome) => {
    const old = deferred<DictionaryState>();
    const latest = deferred<DictionaryState>();
    invokeCommand.mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
    const { subscribe, onChange } = createSync();
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    emitStateChanged();
    latest.resolve(ready);
    await vi.advanceTimersByTimeAsync(0);
    if (outcome === "state") old.resolve(preparing);
    else old.reject("旧请求失败");
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenCalledExactlyOnceWith(ready);
  });

  it("shares ordering with progress changes and ignores a command response after completion", async () => {
    const old = deferred<DictionaryState>();
    invokeCommand.mockReturnValueOnce(old.promise);
    const { sync, subscribe, onChange, onUpdate } = createSync();
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    onUpdate.mockImplementation((event) => {
      if (event.type === "downloadProgress") {
        sync.update((state) => ({ ...state, status: "updating", downloadedBytes: event.downloadedBytes }));
      }
    });
    handlers.get("dictionary_download_progress")!({ operationId: "update-1", downloadedBytes: 50, totalBytes: 100 });
    old.resolve(preparing);
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ status: "updating", downloadedBytes: 50 }));

    const commandRequest = sync.beginUpdateRequest();
    sync.update(ready);
    expect(sync.applyUpdateRequest(commandRequest, { ...ready, status: "updating" })).toBe(false);
    expect(onChange).toHaveBeenLastCalledWith(ready);
  });

  it.each(["completed", "failed"])("accepts the command outcome after progress when the %s listener is unavailable", async (outcome) => {
    listenTo.mockImplementation(async (name, handler) => {
      if (name === `dictionary_update_${outcome}`) throw new Error("registration failed");
      handlers.set(name, handler);
      const unlisten = vi.fn();
      unlisteners.push(unlisten);
      return unlisten;
    });
    const { sync, subscribe, onUpdate, onChange, onError } = createSync();
    onUpdate.mockImplementation((event) => {
      if (event.type === "downloadProgress") {
        sync.update((state) => ({ ...state, status: "updating", downloadedBytes: event.downloadedBytes }));
      }
    });
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(onError).toHaveBeenLastCalledWith("registration failed");
    sync.update((state) => ({ ...state, status: "updating" }));
    const request = sync.beginUpdateRequest();
    handlers.get("dictionary_download_progress")!({ operationId: "update-1", downloadedBytes: 50, totalBytes: 100 });
    const finalState: DictionaryState = outcome === "completed"
      ? ready
      : { ...preparing, status: "failed", error: "下载失败" };
    expect(sync.applyUpdateRequest(request, finalState)).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith(finalState);
  });

  it("rejects an older update command while a newer command is updating", () => {
    const { sync, onChange } = createSync();
    sync.update({ ...ready, status: "updating" });
    const old = sync.beginUpdateRequest();
    sync.update(ready);
    sync.update({ ...ready, status: "updating" });
    const latest = sync.beginUpdateRequest();
    expect(sync.applyUpdateRequest(old, { ...ready, installedRelease: "old" })).toBe(false);
    expect(sync.applyUpdateRequest(latest, { ...ready, installedRelease: "new" })).toBe(true);
    expect(onChange).toHaveBeenLastCalledWith({ ...ready, installedRelease: "new" });
  });

  it("resyncs ignored terminal events after a missed start without adopting stale event state", async () => {
    const updating: DictionaryState = { ...preparing, status: "updating" };
    invokeCommand.mockResolvedValue(updating);
    const { sync, subscribe, onChange } = createSync();
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenLastCalledWith(updating);

    // A previous operation completes while the current operation is still active.
    handlers.get("dictionary_update_completed")!({ operationId: "old", state: ready });
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenLastCalledWith(updating);

    // This operation's start happened before mounting, so App ignores its ID too.
    invokeCommand.mockResolvedValue(ready);
    handlers.get("dictionary_update_completed")!({ operationId: "current", state: { ...ready, installedRelease: "stale" } });
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenLastCalledWith(ready);
    expect(sync.mergeSnapshot({ dictionary: updating }).dictionary).toEqual(ready);
    expect(invokeCommand).toHaveBeenCalledTimes(3);
  });

  it.each([false, true])("preserves current dictionary in late full snapshots (initial snapshot seeded: %s)", async (seeded) => {
    const { sync, subscribe } = createSync();
    if (seeded) {
      expect(sync.mergeSnapshot({ ...DEFAULT_SNAPSHOT, dictionary: preparing }).dictionary).toEqual(preparing);
    }
    const oldSnapshot = deferred<typeof DEFAULT_SNAPSHOT>();
    const refresh = oldSnapshot.promise.then(sync.mergeSnapshot);
    invokeCommand.mockResolvedValue(ready);
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    const snapshot = {
      ...DEFAULT_SNAPSHOT,
      dictionary: preparing,
      settings: { ...DEFAULT_SNAPSHOT.settings, paragraphLearningModeEnabled: true },
    };
    oldSnapshot.resolve(snapshot);
    const merged = await refresh;
    expect(merged.dictionary).toEqual(ready);
    expect(merged.settings.paragraphLearningModeEnabled).toBe(true);
    // Completion/settings refreshes starting later also keep the event state.
    expect(sync.mergeSnapshot(snapshot).dictionary).toEqual(ready);
  });

  it("releases registrations that finish after teardown while other registrations remain pending", async () => {
    const registrations = new Map<string, ReturnType<typeof deferred<() => void>>>();
    listenTo.mockImplementation((name, handler) => {
      handlers.set(name, handler);
      const registration = deferred<() => void>();
      registrations.set(name, registration);
      return registration.promise;
    });
    const { subscribe, onChange, onUpdate } = createSync();
    const stop = subscribe();
    const first = vi.fn();
    registrations.get(DICTIONARY_STATE_CHANGED_EVENT)!.resolve(first);
    await vi.advanceTimersByTimeAsync(0);
    stop();
    expect(first).toHaveBeenCalledOnce();

    const late = vi.fn();
    registrations.get(DICTIONARY_EVENT_NAMES[0])!.resolve(late);
    await vi.advanceTimersByTimeAsync(0);
    expect(late).toHaveBeenCalledOnce();
    emitStateChanged();
    handlers.get("dictionary_update_started")!({ operationId: "old", state: preparing });
    for (const name of DICTIONARY_EVENT_NAMES.slice(1)) {
      const unlisten = vi.fn();
      unlisteners.push(unlisten);
      registrations.get(name)!.resolve(unlisten);
    }
    await vi.advanceTimersByTimeAsync(0);
    expect(unlisteners.every((unlisten) => unlisten.mock.calls.length === 1)).toBe(true);
    expect(invokeCommand).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
    expect(onUpdate).not.toHaveBeenCalled();
  });

  it("ignores old events and reads after StrictMode cleanup and resubscription", async () => {
    const oldRead = deferred<DictionaryState>();
    invokeCommand.mockReturnValueOnce(oldRead.promise).mockResolvedValue(ready);
    const { subscribe, onChange, onUpdate } = createSync();
    const stop = subscribe();
    await vi.advanceTimersByTimeAsync(0);
    const oldStateHandler = handlers.get(DICTIONARY_STATE_CHANGED_EVENT)!;
    const oldUpdateHandler = handlers.get("dictionary_update_started")!;
    stop();
    subscribe();
    await vi.advanceTimersByTimeAsync(0);
    oldRead.resolve(preparing);
    oldStateHandler(null);
    oldUpdateHandler({ operationId: "old", state: preparing });
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenCalledExactlyOnceWith(ready);
    expect(onUpdate).not.toHaveBeenCalled();
    expect(invokeCommand).toHaveBeenCalledTimes(2);
    expect(unlisteners.slice(0, DICTIONARY_EVENT_NAMES.length + 1).every((unlisten) => unlisten.mock.calls.length === 1)).toBe(true);
  });

  it("reports registration failure, still reads once, and displays backend initialization failure", async () => {
    listenTo.mockRejectedValueOnce(undefined);
    const failed: DictionaryState = { ...preparing, status: "failed", error: "词典初始化失败" };
    invokeCommand.mockResolvedValue(failed);
    const { subscribe, onChange, onError } = createSync();
    const stop = subscribe();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(onError).toHaveBeenLastCalledWith("词典事件监听初始化失败");
    expect(onChange).toHaveBeenLastCalledWith(failed);
    expect(invokeCommand).toHaveBeenCalledExactlyOnceWith("get_dictionary_state");
    expect(vi.getTimerCount()).toBe(0);
    stop();
    expect(unlisteners.every((unlisten) => unlisten.mock.calls.length === 1)).toBe(true);
  });

  it("finishes in a readable failed state on read failure and recovers only on another event", async () => {
    invokeCommand.mockRejectedValueOnce("读取词典失败");
    const { sync, subscribe, onChange } = createSync();
    sync.mergeSnapshot({ dictionary: preparing });
    subscribe();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(onChange).toHaveBeenLastCalledWith({ ...preparing, status: "failed", error: "读取词典失败" });
    expect(invokeCommand).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
    invokeCommand.mockResolvedValue(ready);
    emitStateChanged();
    await vi.advanceTimersByTimeAsync(0);
    expect(onChange).toHaveBeenLastCalledWith(ready);
  });
});
