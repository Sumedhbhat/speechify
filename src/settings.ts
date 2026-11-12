// One shared copy of the settings. Changes are applied to a copy and saved;
// the copy only becomes current once the shell accepts it.

import type { Settings } from "./api";
import type { Api } from "./deps";
import { errorText } from "./ui";

export interface SettingsStore {
  /** Fetches the settings the first time; later calls reuse them. */
  load(): Promise<Settings>;
  /**
   * Applies `change` to a copy and saves it. Resolves to "" once saved, or to
   * the shell's error sentence when the save was rejected.
   */
  update(change: (settings: Settings) => void): Promise<string>;
  /** Called after every accepted save. */
  subscribe(listener: (settings: Settings) => void): void;
}

export function createSettingsStore(
  api: Pick<Api, "getSettings" | "saveSettings">,
  onSaved: () => void,
  onError: (message: string) => void,
): SettingsStore {
  let loaded: Promise<Settings> | null = null;
  let queue: Promise<unknown> = Promise.resolve();
  const listeners: ((settings: Settings) => void)[] = [];

  const load = () => {
    // A failed fetch is forgotten so the next call can try again.
    loaded ??= api.getSettings().catch((error: unknown) => {
      loaded = null;
      throw error;
    });
    return loaded;
  };

  const save = async (change: (settings: Settings) => void) => {
    try {
      const next = JSON.parse(JSON.stringify(await load())) as Settings;
      change(next);
      await api.saveSettings(next);
      loaded = Promise.resolve(next);
      listeners.forEach((listener) => listener(next));
      onSaved();
      return "";
    } catch (error) {
      const message = errorText(error);
      onError(message);
      return message;
    }
  };

  return {
    load,
    // Saves run one after another so a quick second change builds on the first.
    update(change) {
      const result = queue.then(() => save(change));
      queue = result;
      return result;
    },
    subscribe(listener) {
      listeners.push(listener);
    },
  };
}
