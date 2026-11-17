//! The focused application, and whether its focused control takes text.

/// The window that has keyboard focus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Window {
    pub app: String,
    /// Executable file name, e.g. "slack.exe".
    pub process: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Editable {
    Yes,
    /// Focus is on something that is clearly not a text field.
    No,
    /// Could not tell. Callers should try to paste.
    Unknown,
}

pub fn active_window() -> Option<Window> {
    let window = active_win_pos_rs::get_active_window().ok()?;
    let process = window
        .process_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some(Window {
        app: window.app_name,
        process,
        title: window.title,
    })
}

/// Whether typing would land in a text field. This errs towards `Unknown`:
/// wrongly refusing to paste is worse than pasting into nothing.
pub fn editable() -> Editable {
    imp::editable()
}

#[cfg(windows)]
mod imp {
    use super::Editable;
    use uiautomation::types::ControlType;
    use uiautomation::UIAutomation;

    struct Probe {
        // Drop the COM interface before balancing its apartment.
        automation: UIAutomation,
        _apartment: crate::com::Apartment,
    }

    thread_local! {
        static PROBE: Option<Probe> = (|| {
            let apartment = crate::com::Apartment::new()?;
            let automation = UIAutomation::new_direct().ok()?;
            Some(Probe { automation, _apartment: apartment })
        })();
    }

    pub fn editable() -> Editable {
        PROBE.with(|probe| {
            probe
                .as_ref()
                .map_or(Editable::Unknown, |probe| inspect(&probe.automation))
        })
    }

    fn inspect(automation: &UIAutomation) -> Editable {
        let Ok(element) = automation.get_focused_element() else {
            return Editable::Unknown;
        };
        let class = element.get_classname().unwrap_or_default();
        // The desktop and the taskbar: nothing to type into.
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return Editable::No;
        }
        match element.get_control_type() {
            Ok(ControlType::Edit | ControlType::Document | ControlType::ComboBox) => Editable::Yes,
            Ok(
                ControlType::Button
                | ControlType::CheckBox
                | ControlType::RadioButton
                | ControlType::Hyperlink
                | ControlType::Menu
                | ControlType::MenuBar
                | ControlType::MenuItem
                | ControlType::ProgressBar
                | ControlType::ScrollBar
                | ControlType::Slider
                | ControlType::StatusBar
                | ControlType::Tab
                | ControlType::TabItem
                | ControlType::TitleBar
                | ControlType::ToolBar,
            ) => Editable::No,
            // Panes, groups and custom controls are how terminals, games and
            // many web views present themselves; they may well accept text.
            _ => Editable::Unknown,
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Editable;
    use std::ffi::{c_char, c_void, CStr};

    type CFTypeRef = *const c_void;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> CFTypeRef;
        fn AXUIElementCopyAttributeValue(element: CFTypeRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringCreateWithCString(allocator: CFTypeRef, text: *const c_char, encoding: u32) -> CFTypeRef;
        fn CFStringGetCString(string: CFTypeRef, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
        fn CFGetTypeID(value: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFRelease(value: CFTypeRef);
    }

    const UTF8: u32 = 0x0800_0100;

    /// Copies an accessibility attribute. The caller releases the result.
    unsafe fn attribute(element: CFTypeRef, name: &CStr) -> Option<CFTypeRef> {
        let key = CFStringCreateWithCString(std::ptr::null(), name.as_ptr(), UTF8);
        if key.is_null() {
            return None;
        }
        let mut value: CFTypeRef = std::ptr::null();
        let status = AXUIElementCopyAttributeValue(element, key, &mut value);
        CFRelease(key);
        (status == 0 && !value.is_null()).then_some(value)
    }

    pub fn editable() -> Editable {
        unsafe {
            let system = AXUIElementCreateSystemWide();
            if system.is_null() {
                return Editable::Unknown;
            }
            let focused = attribute(system, c"AXFocusedUIElement");
            CFRelease(system);
            // No focused element usually means Accessibility access is missing.
            let Some(focused) = focused else {
                return Editable::Unknown;
            };
            let role = attribute(focused, c"AXRole");
            CFRelease(focused);
            let Some(role) = role else {
                return Editable::Unknown;
            };
            let mut buffer = [0 as c_char; 128];
            let read = CFGetTypeID(role) == CFStringGetTypeID()
                && CFStringGetCString(role, buffer.as_mut_ptr(), buffer.len() as isize, UTF8);
            CFRelease(role);
            if !read {
                return Editable::Unknown;
            }
            match CStr::from_ptr(buffer.as_ptr()).to_string_lossy().as_ref() {
                "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField" => Editable::Yes,
                "AXButton" | "AXMenuItem" | "AXMenuBarItem" | "AXCheckBox" | "AXRadioButton" | "AXSlider"
                | "AXLink" | "AXToolbar" => Editable::No,
                // Web areas, groups and scroll areas may hold an editable region.
                _ => Editable::Unknown,
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::Editable;

    pub fn editable() -> Editable {
        // A failed window lookup also covers unavailable WM metadata; it
        // does not prove that the focused control cannot accept a paste.
        Editable::Unknown
    }
}
