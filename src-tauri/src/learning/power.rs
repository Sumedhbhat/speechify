//! Detect external power without changing operating-system settings.
#[cfg(target_os = "linux")]
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;

pub fn mains() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux_mains(Path::new("/sys/class/power_supply"))
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("/usr/bin/pmset")
            .args(["-g", "batt"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("AC Power"))
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        struct Power {
            ac: u8,
            flag: u8,
            percent: u8,
            reserved: u8,
            life: u32,
            full: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetSystemPowerStatus(p: *mut Power) -> i32;
        }
        let mut p = Power {
            ac: 255,
            flag: 0,
            percent: 0,
            reserved: 0,
            life: 0,
            full: 0,
        };
        unsafe { GetSystemPowerStatus(&mut p) != 0 && p.ac == 1 }
    }
}
#[cfg(target_os = "linux")]
pub(super) fn linux_mains(root: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    let mut battery = false;
    for d in entries.flatten() {
        let p = d.path();
        let kind = std::fs::read_to_string(p.join("type")).unwrap_or_default();
        if kind.trim() == "Battery" {
            battery = true;
            let status = std::fs::read_to_string(p.join("status")).unwrap_or_default();
            if ["Charging", "Full"].contains(&status.trim()) {
                return true;
            }
        }
        if std::fs::read_to_string(p.join("online")).is_ok_and(|service| service.trim() == "1") {
            return true;
        }
    }
    !battery
}
