import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

export function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args);
}

export function listenTo<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (message) => handler(message.payload));
}
