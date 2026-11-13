//! User settings. Every field has a default so older files keep loading.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Capitals and full punctuation.
    #[default]
    Formal,
    /// Capitals, no full stop at the end of a short message.
    Casual,
    /// No capitals, no closing full stop.
    VeryCasual,
    /// Like formal, ending on an exclamation mark.
    Excited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Spelling {
    /// Indian and British spellings: colour, organise, centre.
    #[default]
    Indian,
    American,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NumberSystem {
    /// 2,50,000
    #[default]
    Indian,
    /// 250,000
    International,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Cleanup {
    /// Paste exactly what was recognised.
    None,
    /// Remove fillers, fix corrections, format lists.
    #[default]
    Light,
    /// Also tighten wording.
    Medium,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    #[default]
    Keep,
    /// Delete dictations older than a day.
    Day,
    /// Store nothing; history and learning are off.
    Never,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub push_to_talk: String,
    pub hands_free: String,
    pub command: String,
    pub paste_last: String,
    pub copy_last: String,
    pub cancel: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        let mac = cfg!(target_os = "macos");
        let pick = |other: &str, on_mac: &str| if mac { on_mac } else { other }.to_string();
        Self {
            push_to_talk: pick("Ctrl+Meta", "Ctrl+Alt"),
            hands_free: pick("Ctrl+Meta+Space", "Ctrl+Alt+Space"),
            command: pick("Ctrl+Meta+Alt", "Ctrl+Alt+Meta"),
            paste_last: pick("Alt+Shift+Z", "Ctrl+Meta+V"),
            copy_last: pick("Alt+Shift+X", "Ctrl+Meta+C"),
            cancel: "Escape".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Styles {
    pub personal: Style,
    pub work: Style,
    pub email: Style,
    pub other: Style,
}

impl Default for Styles {
    fn default() -> Self {
        Self {
            personal: Style::Casual,
            work: Style::Formal,
            email: Style::Formal,
            other: Style::Formal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Speech {
    /// Catalog id of the on-device model.
    pub model: String,
    /// "auto", "cpu", "vulkan", "metal" or "cuda".
    pub device: String,
    pub language: String,
    /// Optional OpenAI-compatible transcription server used instead of the local model.
    pub remote_url: String,
    pub remote_key: String,
    pub remote_model: String,
}

impl Default for Speech {
    fn default() -> Self {
        Self {
            model: "whisper-large-v3-turbo".into(),
            device: "auto".into(),
            language: "en".into(),
            remote_url: String::new(),
            remote_key: String::new(),
            remote_model: "whisper-large-v3-turbo".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rewrite {
    pub level: Cleanup,
    /// Use a local language model for cleanup; the rules still run without it.
    pub enabled: bool,
    /// Ollama by default; any OpenAI-compatible server works.
    pub url: String,
    pub key: String,
    pub model: String,
    /// Past this the rule-based result is pasted instead.
    pub timeout_ms: u64,
    /// Shorter dictations skip the model: nothing to rewrite, and it is faster.
    pub min_words: usize,
}

impl Default for Rewrite {
    fn default() -> Self {
        Self {
            level: Cleanup::Light,
            enabled: true,
            url: "http://127.0.0.1:11434".into(),
            key: String::new(),
            model: "gemma3:4b".into(),
            timeout_ms: 6000,
            min_words: 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Teacher {
    pub enabled: bool,
    /// Local hour (0-23) after which the nightly review may run.
    pub hour: u32,
    pub speech_model: String,
    pub rewrite_model: String,
    /// A correction seen this many times is added to the dictionary.
    pub learn_after: u32,
    /// Most dictations reviewed per run.
    pub batch: usize,
}

impl Default for Teacher {
    fn default() -> Self {
        Self {
            enabled: true,
            hour: 2,
            speech_model: "whisper-large-v3".into(),
            rewrite_model: "qwen3.5:9b-q4_K_M".into(),
            learn_after: 2,
            batch: 200,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkeys: Hotkeys,
    pub styles: Styles,
    pub speech: Speech,
    pub rewrite: Rewrite,
    pub teacher: Teacher,
    pub spelling: Spelling,
    pub numbers: NumberSystem,
    /// Input device name; empty uses the system default.
    pub microphone: String,
    pub sounds: bool,
    pub mute_while_dictating: bool,
    pub show_bar_always: bool,
    pub launch_at_login: bool,
    pub retention: Retention,
    pub keep_audio: bool,
    /// Learn dictionary words from corrections.
    pub auto_learn: bool,
    /// Saying "press enter" at the end sends Enter after pasting.
    pub press_enter: bool,
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkeys: Hotkeys::default(),
            styles: Styles::default(),
            speech: Speech::default(),
            rewrite: Rewrite::default(),
            teacher: Teacher::default(),
            spelling: Spelling::Indian,
            numbers: NumberSystem::Indian,
            microphone: String::new(),
            sounds: true,
            mute_while_dictating: false,
            show_bar_always: true,
            launch_at_login: true,
            retention: Retention::Keep,
            keep_audio: true,
            auto_learn: true,
            press_enter: false,
            theme: "system".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_files_fill_in_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"rewrite":{"model":"qwen3:4b"},"sounds":false}"#).unwrap();
        assert_eq!(settings.rewrite.model, "qwen3:4b");
        assert_eq!(settings.rewrite.timeout_ms, 6000);
        assert!(!settings.sounds);
        assert_eq!(settings.hotkeys, Hotkeys::default());
        assert_eq!(settings.styles.personal, Style::Casual);
        assert_eq!(settings.teacher.hour, 2);
        assert_eq!(settings.speech.language, "en");
    }

    #[test]
    fn round_trips() {
        let settings = Settings::default();
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        assert!(json.contains(r#""spelling":"indian""#));
    }
}
