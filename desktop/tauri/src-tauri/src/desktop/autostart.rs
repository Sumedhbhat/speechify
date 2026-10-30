//! Manage only this candidate's startup-at-login entry.
use crate::config::Result;

pub(super) fn set_enabled(enabled: bool) -> Result<()> {
    let exe = std::env::current_exe().map_err(crate::config::err)?;
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
            })
            .ok_or("HOME or XDG_CONFIG_HOME missing")?;
        let path = base.join("autostart/speechify-tauri-candidate.desktop");
        if enabled {
            std::fs::create_dir_all(path.parent().unwrap()).map_err(crate::config::err)?;
            let exe = exe
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$");
            let entry = format!(
                concat!(
                    "[Desktop Entry]\n",
                    "Type=Application\n",
                    "Name=Speechify Tauri Candidate\n",
                    "Exec=\"{}\"\n",
                    "X-GNOME-Autostart-enabled=true\n",
                ),
                exe
            );
            std::fs::write(path, entry).map_err(crate::config::err)?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(crate::config::err)?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let path = std::path::PathBuf::from(std::env::var_os("HOME").ok_or("HOME missing")?)
            .join("Library/LaunchAgents/org.sumedhbhat.speechify.tauri.plist");
        if enabled {
            std::fs::create_dir_all(path.parent().unwrap()).map_err(crate::config::err)?;
            let exe = exe
                .to_string_lossy()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let entry = format!(
                concat!(
                    "<?xml version=\"1.0\"?><plist version=\"1.0\"><dict>",
                    "<key>Label</key><string>org.sumedhbhat.speechify.tauri</string>",
                    "<key>ProgramArguments</key><array><string>{}</string></array>",
                    "<key>RunAtLoad</key><true/></dict></plist>",
                ),
                exe
            );
            std::fs::write(path, entry).map_err(crate::config::err)?;
        } else if path.exists() {
            std::fs::remove_file(path).map_err(crate::config::err)?;
        }
    }
    #[cfg(windows)]
    {
        let key = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
        let status = if enabled {
            std::process::Command::new("reg")
                .args([
                    "add",
                    key,
                    "/v",
                    "SpeechifyTauriCandidate",
                    "/t",
                    "REG_SZ",
                    "/d",
                    &format!("\"{}\"", exe.display()),
                    "/f",
                ])
                .status()
        } else {
            std::process::Command::new("reg")
                .args(["delete", key, "/v", "SpeechifyTauriCandidate", "/f"])
                .status()
        }
        .map_err(crate::config::err)?;
        if !status.success() {
            return Err("Could not update candidate login entry".into());
        }
    }
    Ok(())
}
