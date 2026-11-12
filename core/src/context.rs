//! Where the text is going: which kind of app is focused and what that
//! means for tone and for how to paste.

use crate::settings::{Style, Styles};
use serde::{Deserialize, Serialize};

/// The focused application when dictation started.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    /// Display name, e.g. "Slack".
    pub app: String,
    /// Executable or bundle id, e.g. "slack.exe".
    pub process: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Personal,
    Work,
    Email,
    Code,
    Terminal,
    Assistant,
    Document,
    #[default]
    Other,
}

/// Matched against the app name and process first, then the window title
/// (which is how web apps in a browser are told apart).
const APPS: &[(Category, &[&str])] = &[
    (
        Category::Terminal,
        &[
            "terminal",
            "powershell",
            "cmd.exe",
            "iterm",
            "kitty",
            "alacritty",
            "wezterm",
            "konsole",
            "warp",
            "ghostty",
            "tmux",
            "conhost",
        ],
    ),
    (
        Category::Code,
        &[
            "visual studio",
            "vs code",
            "code.exe",
            "cursor",
            "windsurf",
            "intellij",
            "pycharm",
            "webstorm",
            "rustrover",
            "goland",
            "android studio",
            "xcode",
            "sublime",
            "neovim",
            "zed",
        ],
    ),
    (
        Category::Email,
        &[
            "outlook",
            "thunderbird",
            "apple mail",
            "mail.app",
            "spark",
            "superhuman",
        ],
    ),
    (
        Category::Work,
        &[
            "slack",
            "microsoft teams",
            "teams",
            "zoom",
            "webex",
            "linear",
            "jira",
            "asana",
        ],
    ),
    (
        Category::Personal,
        &[
            "whatsapp",
            "telegram",
            "signal",
            "messages",
            "imessage",
            "discord",
            "messenger",
            "instagram",
        ],
    ),
    (
        Category::Assistant,
        &[
            "chatgpt",
            "claude",
            "gemini",
            "copilot",
            "perplexity",
            "t3 code",
            "ollama",
        ],
    ),
    (
        Category::Document,
        &[
            "word", "winword", "notion", "obsidian", "pages", "onenote", "notepad", "textedit", "evernote", "bear",
            "logseq",
        ],
    ),
];
const TITLES: &[(Category, &[&str])] = &[
    (
        Category::Email,
        &["gmail", "outlook", "inbox", "proton mail", "yahoo mail", "- mail"],
    ),
    (
        Category::Work,
        &["slack", "microsoft teams", "linear", "jira", "confluence", "asana"],
    ),
    (
        Category::Personal,
        &["whatsapp", "telegram", "messenger", "instagram", "discord"],
    ),
    (
        Category::Assistant,
        &["chatgpt", "claude", "gemini", "perplexity", "copilot"],
    ),
    (Category::Code, &["github", "gitlab", "stack overflow"]),
    (
        Category::Document,
        &["google docs", "notion", "google sheets", "google slides"],
    ),
];

/// A user rule: apps whose name contains `pattern` belong to `category`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub pattern: String,
    pub category: Category,
}

fn find(table: &[(Category, &[&str])], text: &str) -> Option<Category> {
    table
        .iter()
        .find(|(_, names)| names.iter().any(|name| text.contains(name)))
        .map(|(category, _)| *category)
}

pub fn categorize(target: &Target, rules: &[Rule]) -> Category {
    let app = format!("{} {}", target.app, target.process).to_lowercase();
    let title = target.title.to_lowercase();
    let custom = rules.iter().find(|rule| {
        let pattern = rule.pattern.trim().to_lowercase();
        !pattern.is_empty() && (app.contains(&pattern) || title.contains(&pattern))
    });
    match custom {
        Some(rule) => rule.category,
        None => find(APPS, &app).or_else(|| find(TITLES, &title)).unwrap_or_default(),
    }
}

/// How to send the paste keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PasteKeys {
    /// Ctrl+V, or Cmd+V on a Mac.
    Standard,
    /// Ctrl+Shift+V, for Linux terminals.
    WithShift,
    /// Shift+Insert, which VS Code-family editors on Windows take more reliably.
    ShiftInsert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    #[cfg(target_os = "windows")]
    const CURRENT: Os = Os::Windows;
    #[cfg(target_os = "macos")]
    const CURRENT: Os = Os::Mac;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    const CURRENT: Os = Os::Linux;

    pub fn current() -> Os {
        Self::CURRENT
    }
}

impl Category {
    pub fn style(self, styles: &Styles) -> Style {
        match self {
            Category::Personal => styles.personal,
            Category::Work => styles.work,
            Category::Email => styles.email,
            _ => styles.other,
        }
    }

    /// Code and terminals take the words as spoken: no respelling, no restyling.
    pub fn literal(self) -> bool {
        matches!(self, Category::Code | Category::Terminal)
    }

    pub fn label(self) -> &'static str {
        match self {
            Category::Personal => "Personal messages",
            Category::Work => "Work messages",
            Category::Email => "Email",
            Category::Code => "Code",
            Category::Terminal => "Terminal",
            Category::Assistant => "AI prompts",
            Category::Document => "Documents",
            Category::Other => "Other",
        }
    }

    pub fn paste_keys(self, os: Os) -> PasteKeys {
        match (self, os) {
            (Category::Terminal, Os::Linux) => PasteKeys::WithShift,
            (Category::Code, Os::Windows) => PasteKeys::ShiftInsert,
            _ => PasteKeys::Standard,
        }
    }
}

/// How long to wait before putting the user's clipboard back. Slow apps
/// read the clipboard late; restoring too early pastes the old contents.
pub fn restore_delay_ms(target: &Target, os: Os) -> u64 {
    let app = format!("{} {} {}", target.app, target.process, target.title).to_lowercase();
    let has = |names: &[&str]| names.iter().any(|name| app.contains(name));
    if has(&[
        "remote desktop",
        "mstsc",
        "anydesk",
        "teamviewer",
        "citrix",
        "vnc",
        "parsec",
    ]) {
        5000
    } else if has(&["outlook", "winword", "word", "whatsapp"]) {
        3000
    } else if os == Os::Mac {
        1500
    } else {
        700
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(app: &str, process: &str, title: &str) -> Target {
        Target {
            app: app.into(),
            process: process.into(),
            title: title.into(),
        }
    }

    #[test]
    fn recognises_native_apps() {
        let cases = [
            ("Slack", "slack.exe", "general", Category::Work),
            ("WhatsApp", "", "", Category::Personal),
            ("Microsoft Outlook", "OUTLOOK.EXE", "", Category::Email),
            ("Cursor", "Cursor.exe", "main.rs", Category::Code),
            ("Windows Terminal", "WindowsTerminal.exe", "", Category::Terminal),
            ("Claude", "", "", Category::Assistant),
            ("Notion", "", "", Category::Document),
            ("Calculator", "calc.exe", "", Category::Other),
        ];
        for (app, process, title, expected) in cases {
            assert_eq!(categorize(&target(app, process, title), &[]), expected, "{app}");
        }
    }

    #[test]
    fn browsers_are_classified_by_title() {
        let chrome = |title| categorize(&target("Google Chrome", "chrome.exe", title), &[]);
        assert_eq!(chrome("Inbox (3) - asha@gmail.com - Gmail"), Category::Email);
        assert_eq!(chrome("WhatsApp"), Category::Personal);
        assert_eq!(chrome("Pull requests · GitHub"), Category::Code);
        assert_eq!(chrome("Quarterly plan - Google Docs"), Category::Document);
        assert_eq!(chrome("ChatGPT"), Category::Assistant);
        assert_eq!(chrome("Cricket scores"), Category::Other);
    }

    #[test]
    fn user_rules_win() {
        let rules = [
            Rule {
                pattern: " ".into(),
                category: Category::Email,
            },
            Rule {
                pattern: "Slack".into(),
                category: Category::Personal,
            },
        ];
        assert_eq!(categorize(&target("Slack", "", ""), &rules), Category::Personal);
        assert_eq!(
            categorize(&target("Chrome", "", "Team Slack"), &rules),
            Category::Personal
        );
        assert_eq!(categorize(&target("Zoom", "", ""), &rules), Category::Work);
    }

    #[test]
    fn category_behaviour() {
        let styles = Styles {
            personal: Style::VeryCasual,
            work: Style::Casual,
            email: Style::Excited,
            other: Style::Formal,
        };
        assert_eq!(Category::Personal.style(&styles), Style::VeryCasual);
        assert_eq!(Category::Work.style(&styles), Style::Casual);
        assert_eq!(Category::Email.style(&styles), Style::Excited);
        assert_eq!(Category::Assistant.style(&styles), Style::Formal);
        assert!(Category::Code.literal() && Category::Terminal.literal() && !Category::Document.literal());
        assert_eq!(Category::Terminal.paste_keys(Os::Linux), PasteKeys::WithShift);
        assert_eq!(Category::Terminal.paste_keys(Os::Mac), PasteKeys::Standard);
        assert_eq!(Category::Code.paste_keys(Os::Windows), PasteKeys::ShiftInsert);
        let labels: std::collections::HashSet<_> = [
            Category::Personal,
            Category::Work,
            Category::Email,
            Category::Code,
            Category::Terminal,
            Category::Assistant,
            Category::Document,
            Category::Other,
        ]
        .map(Category::label)
        .into();
        assert_eq!(labels.len(), 8);
        assert!([Os::Windows, Os::Mac, Os::Linux].contains(&Os::current()));
    }

    #[test]
    fn slow_apps_get_a_longer_clipboard_restore() {
        assert_eq!(
            restore_delay_ms(&target("Remote Desktop Connection", "mstsc.exe", ""), Os::Windows),
            5000
        );
        assert_eq!(restore_delay_ms(&target("Outlook", "", ""), Os::Windows), 3000);
        assert_eq!(restore_delay_ms(&target("Notes", "", ""), Os::Mac), 1500);
        assert_eq!(restore_delay_ms(&target("Notepad", "", ""), Os::Windows), 700);
    }
}
