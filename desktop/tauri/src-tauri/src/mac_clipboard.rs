//! NSPasteboard snapshots retain every data type, not just plain text. The
//! change count prevents restoring over a later copy of identical text.
use crate::config::Result;
use std::ffi::{c_char, c_void, CString};
type Id = *mut c_void;
#[link(name = "AppKit", kind = "framework")]
extern "C" {}
#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Id;
    fn objc_msgSend();
}
unsafe fn sel(s: &str) -> Id {
    sel_registerName(CString::new(s).unwrap().as_ptr())
}
unsafe fn class(s: &str) -> Id {
    objc_getClass(CString::new(s).unwrap().as_ptr())
}
unsafe fn obj(o: Id, s: &str) -> Id {
    let f: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
    f(o, sel(s))
}
unsafe fn count(o: Id, s: &str) -> usize {
    let f: unsafe extern "C" fn(Id, Id) -> usize = std::mem::transmute(objc_msgSend as *const ());
    f(o, sel(s))
}
unsafe fn arg(o: Id, s: &str, a: Id) -> Id {
    let f: unsafe extern "C" fn(Id, Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
    f(o, sel(s), a)
}
unsafe fn at(o: Id, i: usize) -> Id {
    let f: unsafe extern "C" fn(Id, Id, usize) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    f(o, sel("objectAtIndex:"), i)
}
unsafe fn string(s: &str) -> Id {
    arg(
        class("NSString"),
        "stringWithUTF8String:",
        CString::new(s).unwrap().as_ptr() as Id,
    )
}
unsafe fn utf8(s: Id) -> Result<String> {
    let p = obj(s, "UTF8String") as *const c_char;
    if p.is_null() {
        return Err("Invalid pasteboard type".into());
    }
    Ok(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
}
unsafe fn data(bytes: &[u8]) -> Id {
    let f: unsafe extern "C" fn(Id, Id, *const u8, usize) -> Id =
        std::mem::transmute(objc_msgSend as *const ());
    f(
        class("NSData"),
        sel("dataWithBytes:length:"),
        bytes.as_ptr(),
        bytes.len(),
    )
}
unsafe fn set(item: Id, bytes: &[u8], kind: &str) -> bool {
    let f: unsafe extern "C" fn(Id, Id, Id, Id) -> bool =
        std::mem::transmute(objc_msgSend as *const ());
    f(item, sel("setData:forType:"), data(bytes), string(kind))
}
unsafe fn write(pb: Id, items: &[Vec<(String, Vec<u8>)>]) -> Result<()> {
    let array = obj(class("NSMutableArray"), "array");
    for formats in items {
        let item = obj(obj(class("NSPasteboardItem"), "alloc"), "init");
        for (kind, bytes) in formats {
            if !set(item, bytes, kind) {
                obj(item, "release");
                return Err("Pasteboard data refused".into());
            }
        }
        arg(array, "addObject:", item);
        obj(item, "release");
    }
    count(pb, "clearContents");
    let f: unsafe extern "C" fn(Id, Id, Id) -> bool =
        std::mem::transmute(objc_msgSend as *const ());
    if !items.is_empty() && !f(pb, sel("writeObjects:"), array) {
        return Err("Pasteboard write failed".into());
    }
    Ok(())
}
struct Pool(Id);
impl Pool {
    unsafe fn new() -> Self {
        Self(obj(obj(class("NSAutoreleasePool"), "alloc"), "init"))
    }
}
impl Drop for Pool {
    fn drop(&mut self) {
        unsafe {
            obj(self.0, "drain");
        }
    }
}
pub fn snapshot_and_write(text: &str) -> Result<impl FnOnce() -> Result<()> + Send> {
    unsafe {
        let _pool = Pool::new();
        let pb = obj(class("NSPasteboard"), "generalPasteboard");
        let before = count(pb, "changeCount");
        let items = obj(pb, "pasteboardItems");
        let mut snapshot = Vec::new();
        let mut total = 0;
        for i in 0..count(items, "count") {
            let item = at(items, i);
            let types = obj(item, "types");
            let mut formats = Vec::new();
            for j in 0..count(types, "count") {
                let kind = at(types, j);
                let value = arg(item, "dataForType:", kind);
                if value.is_null() {
                    return Err("Clipboard promised data is unavailable; use manual copy".into());
                }
                let len = count(value, "length");
                total += len;
                if total > 32 * 1024 * 1024 {
                    return Err("Clipboard snapshot exceeds 32 MB; use manual copy".into());
                }
                let p = obj(value, "bytes") as *const u8;
                let bytes = if len == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(p, len).to_vec()
                };
                formats.push((utf8(kind)?, bytes));
            }
            snapshot.push(formats);
        }
        if count(pb, "changeCount") != before {
            return Err("Clipboard changed during snapshot; retry".into());
        }
        write(
            pb,
            &[vec![(
                "public.utf8-plain-text".into(),
                text.as_bytes().to_vec(),
            )]],
        )?;
        let generation = count(pb, "changeCount");
        Ok(move || {
            let _pool = Pool::new();
            let pb = obj(class("NSPasteboard"), "generalPasteboard");
            if count(pb, "changeCount") == generation {
                write(pb, &snapshot)?;
            }
            Ok(())
        })
    }
}
