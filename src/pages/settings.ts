// Settings: shortcuts, microphone, behaviour, privacy, appearance.

import type { Settings } from "../api";
import { select, text, toggle, type Binding } from "../controls";
import type { Deps, Page } from "../deps";
import { h, pageHeader, section } from "../ui";

type Hotkey = keyof Settings["hotkeys"];

const HOTKEYS: [Hotkey, string, string][] = [
  ["push_to_talk", "Push to talk", "Hold to dictate, let go to type."],
  ["hands_free", "Hands-free", "Press once to start, again to stop."],
  ["command", "Command", "Speak an instruction for the selected text."],
  ["paste_last", "Paste last", "Type the last dictation again."],
  ["copy_last", "Copy last", "Copy the last dictation."],
  ["cancel", "Cancel", "Stop listening and throw it away."],
];

const LANGUAGES: [string, string][] = [
  ["auto", "Detect automatically"],
  ["en", "English"],
  ["hi", "Hindi"],
  ["bn", "Bengali"],
  ["ta", "Tamil"],
  ["te", "Telugu"],
  ["mr", "Marathi"],
  ["gu", "Gujarati"],
  ["kn", "Kannada"],
  ["ml", "Malayalam"],
  ["es", "Spanish"],
  ["fr", "French"],
  ["de", "German"],
  ["pt", "Portuguese"],
  ["ja", "Japanese"],
  ["zh", "Chinese"],
];

export function createSettingsPage({ api, settings: store }: Deps): Page {
  const body = h("div");
  const el = h("div", { className: "page settings" }, [
    pageHeader("Settings", "Changes are saved as you make them."),
    body,
  ]);

  return {
    el,
    async show() {
      const [settings, status] = await Promise.all([store.load(), api.getStatus()]);
      const on = (binding: Binding<boolean>) => toggle(store, settings, binding);
      body.replaceChildren(
        section(
          "Shortcuts",
          status.shortcuts
            ? "Write keys joined by +, like Ctrl+Meta+Space or Alt+Shift+D."
            : status.wayland
              ? "Global shortcuts are not available on Wayland. Bind “speechify --toggle” to a system shortcut instead."
              : status.platform === "macos"
                ? "Speechify cannot see your keyboard yet. Allow it under System Settings › Privacy & Security › Accessibility, then restart it."
                : "Speechify cannot read the keyboard on this system. Bind “speechify --toggle” to a system shortcut instead.",
          HOTKEYS.map(([key, label, hint]) =>
            text(store, settings, {
              label,
              hint,
              placeholder: "Ctrl+Meta+Space",
              get: (s) => s.hotkeys[key],
              set: (s, v) => (s.hotkeys[key] = v),
            }),
          ),
        ),
        section("Dictation", "", [
          select(store, settings, {
            label: "Microphone",
            options: [
              ["", "System default"],
              ...status.microphones.map((name): [string, string] => [name, name]),
            ],
            get: (s) => s.microphone,
            set: (s, v) => (s.microphone = v),
          }),
          select(store, settings, {
            label: "Language",
            hint: "Choosing your language is faster and more accurate than detecting it.",
            options: LANGUAGES,
            get: (s) => s.speech.language,
            set: (s, v) => (s.speech.language = v),
          }),
          on({
            label: "Say “press enter” to send",
            hint: "Ending with “press enter” presses Enter after typing.",
            get: (s) => s.press_enter,
            set: (s, v) => (s.press_enter = v),
          }),
          on({
            label: "Learn from my corrections",
            hint: "Words you fix in History, and mistakes the nightly review finds, are added to your dictionary.",
            get: (s) => s.auto_learn,
            set: (s, v) => (s.auto_learn = v),
          }),
        ]),
        section("While dictating", "", [
          on({ label: "Play sounds", get: (s) => s.sounds, set: (s, v) => (s.sounds = v) }),
          on({
            label: "Mute other audio",
            hint: "Mutes the speakers while you speak and restores the sound afterwards.",
            get: (s) => s.mute_while_dictating,
            set: (s, v) => (s.mute_while_dictating = v),
          }),
          on({
            label: "Always show the bar",
            hint: "Otherwise it appears only while you dictate.",
            get: (s) => s.show_bar_always,
            set: (s, v) => (s.show_bar_always = v),
          }),
          on({
            label: "Open at login",
            get: (s) => s.launch_at_login,
            set: (s, v) => (s.launch_at_login = v),
          }),
        ]),
        section(
          "Privacy",
          "Everything stays on this computer unless you choose a remote server in Models.",
          [
            select(store, settings, {
              label: "History",
              hint: "“Don’t keep history” also turns off learning and the nightly review.",
              options: [
                ["keep", "Keep"],
                ["day", "Delete after a day"],
                ["never", "Don't keep history"],
              ],
              get: (s) => s.retention,
              set: (s, v) => (s.retention = v as Settings["retention"]),
            }),
            on({
              label: "Keep recordings",
              hint: "Lets you play dictations back and lets the nightly teacher re-listen.",
              get: (s) => s.keep_audio,
              set: (s, v) => (s.keep_audio = v),
            }),
          ],
        ),
        section("Appearance", "", [
          select(store, settings, {
            label: "Theme",
            options: [
              ["system", "Match the system"],
              ["light", "Light"],
              ["dark", "Dark"],
            ],
            get: (s) => s.theme,
            set: (s, v) => (s.theme = v),
          }),
        ]),
        h("p", {
          className: "about hint",
          textContent: `Speechify ${status.version} · ${status.platform}`,
        }),
      );
    },
  };
}
