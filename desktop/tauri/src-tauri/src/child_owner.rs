//! Every helper starts inside a crash-owned process tree. Windows children are
//! suspended until assignment, so they cannot fork outside the Job Object.
use crate::config::Result;
use std::process::{Child, Command};
#[cfg(not(windows))]
pub struct Owner;
#[cfg(not(windows))]
pub fn spawn(cmd: &mut Command) -> Result<(Child, Owner)> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
    unsafe {
        cmd.pre_exec(|| {
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 || libc::getppid() == 1 {
                    return Err(std::io::Error::other("Cannot establish parent ownership"));
                }
            }
            Ok(())
        });
    }
    Ok((cmd.spawn().map_err(|e| e.to_string())?, Owner))
}
#[cfg(windows)]
mod windows {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{io::AsRawHandle, process::CommandExt},
    };
    type Handle = *mut c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Basic {
        process_time: i64,
        job_time: i64,
        flags: u32,
        min: usize,
        max: usize,
        active: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Extended {
        basic: Basic,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: Handle, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, data: *const c_void, size: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtResumeProcess(process: Handle) -> i32;
    }
    pub struct Owner(Handle);
    unsafe impl Send for Owner {}
    impl Drop for Owner {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn spawn(cmd: &mut Command) -> Result<(Child, Owner)> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                return Err("Cannot create child ownership job".into());
            }
            let owner = Owner(job);
            let mut limits = Extended::default();
            limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            if SetInformationJobObject(
                job,
                9,
                &limits as *const _ as _,
                std::mem::size_of::<Extended>() as u32,
            ) == 0
            {
                return Err("Cannot configure child ownership job".into());
            }
            cmd.creation_flags(0x00000004 | 0x08000000); // suspended, no console
            let mut child = cmd.spawn().map_err(|e| e.to_string())?;
            if AssignProcessToJobObject(job, child.as_raw_handle()) == 0
                || NtResumeProcess(child.as_raw_handle()) < 0
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Cannot establish child ownership; helper was stopped".into());
            }
            Ok((child, owner))
        }
    }
}
#[cfg(windows)]
pub use windows::{spawn, Owner};
