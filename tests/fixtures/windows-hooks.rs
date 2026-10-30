//! Replace one imported OS function in an isolated behavioral-test child.
use std::ffi::{c_void, CStr};
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> *mut u8;
    fn VirtualProtect(address: *mut c_void, length: usize, protection: u32, old: *mut u32) -> i32;
}
pub struct Hook {
    slot: *mut usize,
    original: usize,
}
unsafe fn write(slot: *mut usize, value: usize) {
    let mut protection = 0;
    assert_ne!(
        VirtualProtect(
            slot.cast(),
            std::mem::size_of::<usize>(),
            4,
            &mut protection
        ),
        0
    );
    slot.write(value);
    assert_ne!(
        VirtualProtect(
            slot.cast(),
            std::mem::size_of::<usize>(),
            protection,
            &mut protection
        ),
        0
    );
}
impl Hook {
    pub unsafe fn new(name: &str, replacement: *const ()) -> Self {
        let base = GetModuleHandleW(std::ptr::null());
        assert!(!base.is_null());
        let nt = base.add(base.add(0x3c).cast::<u32>().read() as usize);
        let optional = nt.add(24);
        assert_eq!(
            optional.cast::<u16>().read(),
            0x20b,
            "test fixture requires a 64-bit PE image"
        );
        let rva = optional.add(112 + 8).cast::<u32>().read();
        let mut descriptor = base.add(rva as usize);
        while descriptor.add(12).cast::<u32>().read() != 0 {
            let names = descriptor.cast::<u32>().read();
            let addresses = descriptor.add(16).cast::<u32>().read();
            assert_ne!(names, 0, "PE import names unavailable");
            let mut index = 0;
            loop {
                let import = base.add(names as usize).cast::<usize>().add(index).read();
                if import == 0 {
                    break;
                }
                if import >> 63 == 0
                    && CStr::from_ptr(base.add(import + 2).cast()).to_bytes() == name.as_bytes()
                {
                    let slot = base.add(addresses as usize).cast::<usize>().add(index);
                    let original = slot.read();
                    write(slot, replacement as usize);
                    return Self { slot, original };
                }
                index += 1;
            }
            descriptor = descriptor.add(20);
        }
        panic!("OS boundary {name} was not imported");
    }
    pub fn original(&self) -> usize {
        self.original
    }
}
impl Drop for Hook {
    fn drop(&mut self) {
        unsafe {
            write(self.slot, self.original);
        }
    }
}
