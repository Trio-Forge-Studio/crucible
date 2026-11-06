use std::io;
use std::mem::size_of;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

pub struct Job(OwnedHandle);

impl Job {
    pub fn spawn(command: &mut Command) -> io::Result<(Child, Self)> {
        let job = Self::new()?;
        let mut child = command.creation_flags(CREATE_SUSPENDED).spawn()?;
        let assigned = unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), child.as_raw_handle()) };
        let result = if assigned == 0 { Err(io::Error::last_os_error()) } else { resume(&child) };
        if let Err(error) = result {
            drop(job);
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok((child, job))
    }

    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = unsafe {
            SetInformationJobObject(
                job.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if set == 0 { Err(io::Error::last_os_error()) } else { Ok(job) }
    }

    pub fn terminate(self) -> io::Result<()> {
        let terminated = unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
        if terminated == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
}

fn resume(child: &Child) -> io::Result<()> {
    // std's main_thread_handle is unstable. Before the suspended process runs, its only thread is the main thread.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    let mut entry = THREADENTRY32 { dwSize: size_of::<THREADENTRY32>() as u32, ..Default::default() };
    let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
    while found != 0 {
        if entry.th32OwnerProcessID == child.id() {
            let handle = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            let thread = unsafe { OwnedHandle::from_raw_handle(handle) };
            return if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            };
        }
        found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
    }
    Err(io::Error::last_os_error())
}
