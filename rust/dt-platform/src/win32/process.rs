use super::{error, last_error, owned, raw};
use dt_core::{Error, Result};
use std::{os::windows::io::OwnedHandle, process::Child};
use windows::{
    Win32::{
        Foundation::*,
        System::{Diagnostics::ToolHelp::*, JobObjects::*, Threading::*},
    },
    core::PCWSTR,
};

pub(crate) const HIDDEN: u32 = CREATE_NO_WINDOW.0;
pub(crate) const HIDDEN_SUSPENDED: u32 = CREATE_NO_WINDOW.0 | CREATE_SUSPENDED.0;

pub(crate) fn owned_process_image(pid: u32) -> Option<String> {
    // SAFETY: read-only open with an owned handle; UTF-16 buffer and length stay valid.
    unsafe {
        let handle = owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            raw(&handle),
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .ok()?;
        Some(String::from_utf16_lossy(&buffer[..length as usize]))
    }
}

pub(crate) fn create_kill_job() -> Result<OwnedHandle> {
    // SAFETY: unnamed non-inheritable job; limits point to the correctly sized initialized struct.
    unsafe {
        let handle = owned(CreateJobObjectW(None, PCWSTR::null()).map_err(error)?);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            raw(&handle),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
        .map_err(error)?;
        Ok(handle)
    }
}

pub(crate) fn assign_job(job: &OwnedHandle, child: &Child) -> Result<()> {
    // SAFETY: both handles remain owned during assignment; child was created suspended.
    unsafe { AssignProcessToJobObject(raw(job), raw(child)).map_err(error) }
}

pub(crate) fn duplicate_child(child: &Child) -> Result<OwnedHandle> {
    // SAFETY: duplication uses valid current-process pseudo handles and a stack output.
    unsafe {
        let current = GetCurrentProcess();
        let mut duplicate = HANDLE::default();
        DuplicateHandle(
            current,
            raw(child),
            current,
            &mut duplicate,
            0,
            false,
            DUPLICATE_SAME_ACCESS,
        )
        .map_err(error)?;
        Ok(owned(duplicate))
    }
}

pub(crate) fn terminate_job(job: &OwnedHandle) -> Result<()> {
    // SAFETY: job handle is owned and the job contains only this API's owned descendants.
    unsafe { TerminateJobObject(raw(job), 1).map_err(error) }
}

pub(crate) fn single_instance(name: &str) -> Result<Option<OwnedHandle>> {
    if name.is_empty() || name.encode_utf16().any(|c| c == 0) {
        return Err(Error::InvalidInput("mutex name"));
    }
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: name is terminated and lives through the call; no inherited handle or initial owner.
    let handle = unsafe {
        let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr())).map_err(error)?;
        let exists = GetLastError() == ERROR_ALREADY_EXISTS;
        (owned(handle), exists)
    };
    Ok(if handle.1 { None } else { Some(handle.0) })
}

pub(crate) fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: read-only process open; output points to initialized stack memory; handle is RAII.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let handle = owned(handle);
        let mut exit = 0;
        GetExitCodeProcess(raw(&handle), &mut exit).is_ok() && exit == STILL_ACTIVE.0 as u32
    }
}

pub(crate) fn resume_child(child: &Child) -> Result<()> {
    // SAFETY: snapshot is RAII-owned; thread entry's size is initialized before enumeration. The
    // suspended process has exactly its primary thread; open/resume uses only that child's PID.
    unsafe {
        let snapshot = owned(CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0).map_err(error)?);
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        Thread32First(raw(&snapshot), &mut entry).map_err(error)?;
        loop {
            if entry.th32OwnerProcessID == child.id() {
                let thread = owned(
                    OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID).map_err(error)?,
                );
                if ResumeThread(raw(&thread)) == u32::MAX {
                    return Err(last_error());
                }
                return Ok(());
            }
            if Thread32Next(raw(&snapshot), &mut entry).is_err() {
                break;
            }
        }
    }
    Err(Error::Protocol("suspended child thread not found"))
}

pub(crate) fn wait_process(process: &OwnedHandle) -> Result<()> {
    // SAFETY: monitor owns this duplicate for the complete wait; INFINITE is intentional here.
    if unsafe { WaitForSingleObject(raw(process), INFINITE) } == WAIT_OBJECT_0 {
        Ok(())
    } else {
        Err(last_error())
    }
}

pub(crate) fn job_active_processes(job: &OwnedHandle) -> Result<u32> {
    let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
    // SAFETY: live job, writable structure and exact structure size; no pointers outlive the call.
    unsafe {
        QueryInformationJobObject(
            Some(raw(job)),
            JobObjectBasicAccountingInformation,
            (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
            std::mem::size_of_val(&info) as u32,
            None,
        )
        .map_err(error)?;
    }
    Ok(info.ActiveProcesses)
}
