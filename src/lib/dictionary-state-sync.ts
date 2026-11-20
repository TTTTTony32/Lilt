import { describeError } from "./errors";
import { invokeCommand, listenTo } from "./tauri";
import {
  DICTIONARY_EVENT_NAMES,
  DICTIONARY_STATE_CHANGED_EVENT,
  decodeDictionaryState,
  decodeDictionaryUpdateEvent,
  type DictionaryState,
  type DictionaryUpdateEvent,
} from "../types/dictionary";

type StateUpdate = DictionaryState | ((current: DictionaryState) => DictionaryState);

export function createDictionaryStateSync(initial: DictionaryState, onChange: (state: DictionaryState) => void) {
  let current = initial;
  let sequence = 0;
  let updateSequence = 0;
  let seeded = false;

  const beginRequest = () => ++sequence;

  const update = (next: StateUpdate) => {
    ++sequence;
    seeded = true;
    current = typeof next === "function" ? next(current) : next;
    onChange(current);
  };

  const applyRequest = (request: number, next: StateUpdate) => {
    if (request !== sequence) return false;
    update(next);
    return true;
  };

  const beginUpdateRequest = () => ({ sequence: beginRequest(), updateSequence: ++updateSequence });

  const applyUpdateRequest = (request: ReturnType<typeof beginUpdateRequest>, next: StateUpdate) => {
    if (request.updateSequence !== updateSequence) return false;
    // Progress can supersede the initial command state without settling it.
    // Keep the command result usable when its terminal listener is unavailable.
    if (request.sequence !== sequence && current.status !== "updating") return false;
    ++updateSequence;
    update(next);
    return true;
  };

  const mergeSnapshot = <T extends { dictionary: DictionaryState }>(snapshot: T): T => {
    // Only the initial snapshot seeds dictionary state. Subsequent state reads
    // and update events own it, even when another full snapshot finishes later.
    if (!seeded) {
      current = snapshot.dictionary;
      seeded = true;
    }
    return { ...snapshot, dictionary: current };
  };

  const subscribe = (
    onUpdateEvent: (event: DictionaryUpdateEvent) => void,
    onEventsError: (message: string | null) => void,
  ) => {
    let disposed = false;
    const unlisteners = new Set<() => void>();
    onEventsError(null);

    const refresh = async () => {
      const request = beginRequest();
      try {
        const raw = await invokeCommand<unknown>("get_dictionary_state");
        const state = decodeDictionaryState(raw);
        if (!state) throw new Error("词典状态返回了无法识别的结果");
        if (!disposed) applyRequest(request, state);
      } catch (reason) {
        if (disposed) return;
        applyRequest(request, (state) => ({
          ...state,
          status: "failed",
          error: describeError(reason, "无法读取词典状态"),
        }));
      }
    };

    const register = async (name: string, handler: (payload: unknown) => void) => {
      const unlisten = await listenTo<unknown>(name, (payload) => {
        if (!disposed) handler(payload);
      });
      // Each registration cleans itself up immediately if it finishes after
      // teardown, without waiting for the other registrations to settle.
      if (disposed) unlisten();
      else unlisteners.add(unlisten);
    };

    const initialise = async () => {
      const results = await Promise.allSettled([
        register(DICTIONARY_STATE_CHANGED_EVENT, () => { void refresh(); }),
        ...DICTIONARY_EVENT_NAMES.map((name) => register(name, (payload) => {
          const event = decodeDictionaryUpdateEvent(name, payload);
          if (!event) return;
          const previousSequence = sequence;
          onUpdateEvent(event);
          // A remount can miss `started`, so App may reject an unknown operation.
          // Read the current source instead of adopting an unrecognized event.
          if (previousSequence === sequence && (event.type === "completed" || event.type === "failed")) {
            void refresh();
          }
        })),
      ]);
      if (disposed) return;
      const failure = results.find((result) => result.status === "rejected");
      if (failure?.status === "rejected") {
        onEventsError(describeError(failure.reason, "词典事件监听初始化失败"));
      }
      // Reading after registration also covers notifications sent before mount.
      await refresh();
    };

    void initialise();
    return () => {
      disposed = true;
      ++sequence;
      ++updateSequence;
      unlisteners.forEach((unlisten) => unlisten());
      unlisteners.clear();
    };
  };

  return { beginRequest, update, applyRequest, beginUpdateRequest, applyUpdateRequest, mergeSnapshot, subscribe };
}
