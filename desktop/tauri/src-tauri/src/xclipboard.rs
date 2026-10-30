//! X11 selection ownership is the generation guard: a new copy, even identical
//! text, takes ownership away from our unique window. Restoration and the owner
//! check run under a server grab. Rich MIME targets are kept as bounded bytes.
use crate::config::Result;
use libloading::Library;
use std::{
    collections::BTreeMap,
    ffi::{c_char, c_int, c_long, c_ulong, c_void, CString},
    process::Command,
    sync::mpsc,
    time::{Duration, Instant},
};
type Display = *mut c_void;
type Atom = c_ulong;
#[repr(C)]
#[derive(Clone, Copy)]
struct Request {
    kind: c_int,
    serial: c_ulong,
    send: c_int,
    display: Display,
    owner: c_ulong,
    requestor: c_ulong,
    selection: Atom,
    target: Atom,
    property: Atom,
    time: c_ulong,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Notify {
    kind: c_int,
    serial: c_ulong,
    send: c_int,
    display: Display,
    requestor: c_ulong,
    selection: Atom,
    target: Atom,
    property: Atom,
    time: c_ulong,
}
#[repr(C)]
union Event {
    kind: c_int,
    request: Request,
    notify: Notify,
    pad: [c_long; 24],
}
struct X {
    lib: Library,
    fixes: Option<Library>,
    d: Display,
    window: c_ulong,
}
impl X {
    unsafe fn call<T: Copy>(&self, name: &[u8]) -> T {
        *self.lib.get::<T>(name).expect("standard Xlib ABI")
    }
    unsafe fn atom(&self, name: &str) -> Atom {
        let f: unsafe extern "C" fn(Display, *const c_char, c_int) -> Atom =
            self.call(b"XInternAtom\0");
        f(self.d, CString::new(name).unwrap().as_ptr(), 0)
    }
    unsafe fn owner(&self, selection: Atom) -> c_ulong {
        let f: unsafe extern "C" fn(Display, Atom) -> c_ulong = self.call(b"XGetSelectionOwner\0");
        f(self.d, selection)
    }
    unsafe fn flush(&self) {
        let f: unsafe extern "C" fn(Display) -> c_int = self.call(b"XFlush\0");
        f(self.d);
    }
}
impl Drop for X {
    fn drop(&mut self) {
        unsafe {
            let f: unsafe extern "C" fn(Display) -> c_int = self.call(b"XCloseDisplay\0");
            f(self.d);
        }
    }
}
fn read(target: &str) -> Result<Vec<u8>> {
    use std::io::Read;
    use std::process::Stdio;
    // Bound both response time and memory for an unresponsive/oversized owner.
    let mut child = Command::new("timeout")
        .args([
            "2s",
            "xclip",
            "-selection",
            "clipboard",
            "-o",
            "-target",
            target,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(crate::config::err)?;
    let mut bytes = Vec::new();
    let result = child
        .stdout
        .take()
        .unwrap()
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes);
    if result.is_err() || bytes.len() > 128 * 1024 {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "Clipboard target {target} exceeds 128 KiB or cannot be read; use manual copy"
        ));
    }
    let status = child.wait().map_err(crate::config::err)?;
    if !status.success() {
        return Err(format!(
            "Cannot snapshot clipboard target {target}; use manual copy"
        ));
    }
    Ok(bytes)
}

fn supported(target: &str) -> bool {
    ["UTF8_STRING", "STRING", "TEXT", "COMPOUND_TEXT"].contains(&target)
        || target.starts_with("text/")
        || target.starts_with("image/")
        || target == "application/rtf"
}
pub fn begin(text: &str) -> Result<()> {
    use std::io::{BufRead, Write};
    use std::process::Stdio;
    let mut cmd = Command::new(std::env::current_exe().map_err(crate::config::err)?);
    cmd.arg("--clipboard-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let (mut child, owner) = crate::child_owner::spawn(&mut cmd)?;
    let pid = child.id();
    let result = (|| {
        writeln!(child.stdin.take().unwrap(), "{}", serde_json::json!(text))
            .map_err(crate::config::err)?;
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = std::io::BufReader::new(stdout)
                .read_line(&mut line)
                .map_err(crate::config::err)
                .and_then(|_| serde_json::from_str::<Result<()>>(&line).map_err(crate::config::err))
                .and_then(|v| v);
            let _ = tx.send(result);
        });
        rx.recv_timeout(Duration::from_secs(15))
            .map_err(|_| "Clipboard snapshot timed out".to_string())?
    })();
    if result.is_err() {
        crate::worker::terminate(pid);
    }
    std::thread::spawn(move || {
        let _owner = owner;
        let _ = child.wait();
    });
    result
}
pub fn worker() -> Result<()> {
    use std::io::{BufRead, Write};
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(crate::config::err)?;
    let text: String = serde_json::from_str(&line).map_err(crate::config::err)?;
    let (tx, rx) = mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        if let Err(e) = serve(text.into_bytes(), &tx) {
            let _ = tx.send(Err(e));
        }
    });
    let result = rx.recv().map_err(crate::config::err)?;
    println!("{}", serde_json::to_string(&result).unwrap());
    std::io::stdout().flush().map_err(crate::config::err)?;
    result?;
    thread
        .join()
        .map_err(|_| "Clipboard owner stopped".to_string())?;
    Ok(())
}
fn serve(text: Vec<u8>, ready: &mpsc::SyncSender<Result<()>>) -> Result<()> {
    unsafe {
        let lib = Library::new("libX11.so.6").map_err(crate::config::err)?;
        let open = lib
            .get::<unsafe extern "C" fn(*const c_char) -> Display>(b"XOpenDisplay\0")
            .map_err(crate::config::err)?;
        let d = open(std::ptr::null());
        if d.is_null() {
            return Err("Cannot open clipboard display".into());
        }
        let mut x = X {
            lib,
            fixes: None,
            d,
            window: 0,
        };
        let root: unsafe extern "C" fn(Display) -> c_ulong = x.call(b"XDefaultRootWindow\0");
        let create: unsafe extern "C" fn(
            Display,
            c_ulong,
            c_int,
            c_int,
            c_ulong,
            c_ulong,
            c_ulong,
            c_ulong,
            c_ulong,
        ) -> c_ulong = x.call(b"XCreateSimpleWindow\0");
        x.window = create(d, root(d), 0, 0, 1, 1, 0, 0, 0);
        let selection = x.atom("CLIPBOARD");
        let targets = x.atom("TARGETS");
        // XFixes observes every SetSelectionOwner, including same-owner/same-text
        // copies during snapshot acquisition. Owners need not implement TIMESTAMP.
        let fixes = Library::new("libXfixes.so.3")
            .map_err(|_| "XFixes is required for safe clipboard snapshots")?;
        let query = fixes
            .get::<unsafe extern "C" fn(Display, *mut c_int, *mut c_int) -> c_int>(
                b"XFixesQueryExtension\0",
            )
            .map_err(crate::config::err)?;
        let select = fixes
            .get::<unsafe extern "C" fn(Display, c_ulong, Atom, c_ulong)>(
                b"XFixesSelectSelectionInput\0",
            )
            .map_err(crate::config::err)?;
        let (mut event_base, mut error_base) = (0, 0);
        if query(d, &mut event_base, &mut error_base) == 0 {
            return Err("XFixes selection notifications unavailable; use manual copy".into());
        }
        select(d, x.window, selection, 7);
        drop(select);
        drop(query);
        // XCloseDisplay invokes extension callbacks: keep XFixes loaded until
        // after X::drop has closed the connection.
        x.fixes = Some(fixes);
        let sync: unsafe extern "C" fn(Display, c_int) -> c_int = x.call(b"XSync\0");
        let pending: unsafe extern "C" fn(Display) -> c_int = x.call(b"XPending\0");
        let next: unsafe extern "C" fn(Display, *mut Event) -> c_int = x.call(b"XNextEvent\0");
        sync(d, 0);
        while pending(d) > 0 {
            let mut event = Event { pad: [0; 24] };
            next(d, &mut event);
        }
        let owner = x.owner(selection);
        let mut previous = BTreeMap::new();
        if owner != 0 {
            let list =
                String::from_utf8(read("TARGETS")?).map_err(|_| "Invalid clipboard targets")?;
            let mut bytes = 0;
            for name in list.lines().map(str::trim) {
                if ["TARGETS", "TIMESTAMP", "MULTIPLE", "SAVE_TARGETS"].contains(&name) {
                    continue;
                }
                if !supported(name) {
                    return Err(format!(
                        "Clipboard format {name} cannot be preserved safely; use manual copy"
                    ));
                }
                let data = read(name)?;
                bytes += data.len();
                if bytes > 32 * 1024 * 1024 {
                    return Err("Clipboard snapshot exceeds 32 MB; use manual copy".into());
                }
                previous.insert(x.atom(name), data);
            }
            if previous.is_empty() || x.owner(selection) != owner {
                return Err("Clipboard changed while snapshotting; retry".into());
            }
        }
        let set: unsafe extern "C" fn(Display, Atom, c_ulong, c_ulong) -> c_int =
            x.call(b"XSetSelectionOwner\0");
        let grab: unsafe extern "C" fn(Display) -> c_int = x.call(b"XGrabServer\0");
        let ungrab: unsafe extern "C" fn(Display) -> c_int = x.call(b"XUngrabServer\0");
        grab(d);
        sync(d, 0);
        let mut generation_changed = false;
        while pending(d) > 0 {
            let mut event = Event { pad: [0; 24] };
            next(d, &mut event);
            if event.kind == event_base {
                generation_changed = true;
            }
        }
        if generation_changed || x.owner(selection) != owner {
            ungrab(d);
            x.flush();
            return Err("Clipboard changed before paste; retry".into());
        }
        set(d, selection, x.window, 0);
        ungrab(d);
        x.flush();
        if x.owner(selection) != x.window {
            return Err("Clipboard ownership refused".into());
        }
        let mut content = BTreeMap::new();
        for name in ["UTF8_STRING", "text/plain;charset=utf-8", "text/plain"] {
            content.insert(x.atom(name), text.clone());
        }
        if ready.send(Ok(())).is_err() {
            return Ok(());
        }
        let start = Instant::now();
        let mut restored = false;
        let change: unsafe extern "C" fn(
            Display,
            c_ulong,
            Atom,
            Atom,
            c_int,
            c_int,
            *const u8,
            c_int,
        ) -> c_int = x.call(b"XChangeProperty\0");
        let send: unsafe extern "C" fn(Display, c_ulong, c_int, c_long, *mut Event) -> c_int =
            x.call(b"XSendEvent\0");
        loop {
            if !restored && start.elapsed() > Duration::from_millis(500) {
                grab(d);
                let ours = x.owner(selection) == x.window;
                if ours {
                    content = std::mem::take(&mut previous);
                    restored = true;
                }
                ungrab(d);
                x.flush();
                if !ours {
                    return Ok(());
                }
            }
            if x.owner(selection) != x.window {
                return Ok(());
            }
            while pending(d) > 0 {
                let mut event = Event { pad: [0; 24] };
                next(d, &mut event);
                if event.kind == 29 {
                    return Ok(());
                } // SelectionClear, including same-text copy
                if event.kind != 30 {
                    continue;
                }
                let r = event.request;
                let property = if r.property == 0 {
                    r.target
                } else {
                    r.property
                };
                let mut accepted = false;
                if r.target == targets {
                    let mut atoms: Vec<Atom> = content.keys().copied().collect();
                    atoms.push(targets);
                    change(
                        d,
                        r.requestor,
                        property,
                        4,
                        32,
                        0,
                        atoms.as_ptr().cast(),
                        atoms.len() as c_int,
                    );
                    accepted = true;
                } else if let Some(data) = content.get(&r.target) {
                    // Bound direct X requests. Large snapshots are retained but safely refused
                    // to readers rather than exceeding XMaxRequestSize (INCR not implemented).
                    if data.len() <= 128 * 1024 {
                        change(
                            d,
                            r.requestor,
                            property,
                            r.target,
                            8,
                            0,
                            data.as_ptr(),
                            data.len() as c_int,
                        );
                        accepted = true;
                    }
                }
                let mut response = Event { pad: [0; 24] };
                response.notify = Notify {
                    kind: 31,
                    serial: 0,
                    send: 1,
                    display: d,
                    requestor: r.requestor,
                    selection: r.selection,
                    target: r.target,
                    property: if accepted { property } else { 0 },
                    time: r.time,
                };
                send(d, r.requestor, 0, 0, &mut response);
                x.flush();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_formats_are_explicit() {
        assert!(supported("text/html"));
        assert!(supported("image/png"));
        assert!(!supported("application/x-private-handle"));
    }
}
