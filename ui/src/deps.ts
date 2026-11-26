// What pages receive: the command and event objects (real or fake), shared
// settings, and a way to move to another page.

import type { api, events } from "./api";
import type { SettingsStore } from "./settings";

export type Api = typeof api;
export type Events = typeof events;

export interface Deps {
  api: Api;
  events: Events;
  settings: SettingsStore;
  navigate: (page: string) => void;
}

/** A page is built once and refreshed every time it is shown. */
export interface Page {
  el: HTMLElement;
  show(): Promise<void>;
}
