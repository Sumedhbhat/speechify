#!/usr/bin/env python3
"""Generate Rust ABI declarations from the pinned upstream ctypes declarations."""
import importlib.util,pathlib,ctypes
root=pathlib.Path(__file__).resolve().parents[1]
p=root/'.build/bindings/transcribe_cpp/_generated.py'
s=importlib.util.spec_from_file_location('abi',p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m)
types={ctypes.c_uint64:'u64',ctypes.c_uint32:'u32',ctypes.c_int32:'i32',ctypes.c_int:'i32',ctypes.c_float:'f32',ctypes.c_bool:'bool',ctypes.c_char_p:'*const std::ffi::c_char',ctypes.c_void_p:'*mut std::ffi::c_void',ctypes.c_size_t:'usize',ctypes.c_int64:'i64'}
def typ(t):
 if t in types:return types[t]
 if issubclass(t,ctypes._Pointer):return '*const '+typ(t._type_)
 return t.__name__
out=['// Generated from transcribe.cpp 0.3.1; PUBLIC_HEADER_HASH '+m.PUBLIC_HEADER_HASH,'#![allow(non_camel_case_types, dead_code)]']
for name in m.STRUCT_LAYOUT:
 cls=getattr(m,name)
 out+=['#[repr(C)]',f'pub struct {name} {{']+[f'    pub {n}: {typ(t)},' for n,t in cls._fields_]+['}']
out+=['pub fn verify(lib: &libloading::Library) -> Result<(), String> {','unsafe {','let size: libloading::Symbol<unsafe extern "C" fn(i32)->usize> = lib.get(b"transcribe_abi_struct_size\\0").map_err(|e|e.to_string())?;','let align: libloading::Symbol<unsafe extern "C" fn(i32)->usize> = lib.get(b"transcribe_abi_struct_align\\0").map_err(|e|e.to_string())?;']
for name,i in m.ABI_STRUCT_IDS.items():out+=[f'if size({i}) != std::mem::size_of::<{name}>() || align({i}) != std::mem::align_of::<{name}>() {{ return Err("native ABI mismatch: {name}".into()); }}']
out+=['} Ok(()) }']
(root/'src-tauri/src/ffi.rs').write_text('\n'.join(out)+'\n')
