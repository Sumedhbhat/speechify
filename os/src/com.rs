//! Balanced COM initialization, owned by the calling thread.

use std::marker::PhantomData;
use std::rc::Rc;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

pub(crate) struct Apartment(PhantomData<Rc<()>>);

impl Apartment {
    pub(crate) fn new() -> Option<Self> {
        // S_FALSE also needs a matching CoUninitialize. An incompatible
        // apartment is left alone and the optional OS feature is skipped.
        if unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok() {
            Some(Self(PhantomData))
        } else {
            None
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
