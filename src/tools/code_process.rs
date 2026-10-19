//! Per-run process-tree ownership for the code runtime.

/// Owns one code-runtime process range.
///
/// Windows uses a private Job Object with kill-on-close. Unix uses a private
/// process group created at spawn time and kills that group on termination.
pub struct ProcessTree {
    #[cfg(windows)]
    job: Option<isize>,
    #[cfg(unix)]
    pid: Option<i32>,
}

impl ProcessTree {
    /// Attach a freshly spawned child to its private process range.
    pub fn attach(child: &tokio::process::Child) -> Self {
        #[cfg(windows)]
        {
            let job = child.id().and_then(create_job_for_pid);
            if job.is_none() {
                tracing::warn!(
                    pid = child.id(),
                    "could not attach code runtime to a private Job Object; descendants may leak"
                );
            }
            Self { job }
        }
        #[cfg(unix)]
        {
            Self {
                pid: child.id().map(|pid| pid as i32),
            }
        }
        #[cfg(not(any(windows, unix)))]
        {
            let _ = child;
            Self {}
        }
    }

    /// Terminate every process still owned by the process range.
    pub fn terminate(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            // SAFETY: the handle was created by create_job_for_pid and is owned by us.
            unsafe {
                let _ = windows_sys::Win32::System::JobObjects::TerminateJobObject(
                    job as windows_sys::Win32::Foundation::HANDLE,
                    1,
                );
                windows_sys::Win32::Foundation::CloseHandle(
                    job as windows_sys::Win32::Foundation::HANDLE,
                );
            }
        }
        #[cfg(unix)]
        if let Some(pid) = self.pid.take() {
            // SAFETY: the child was spawned into its own process group, so a
            // negative pid targets only that group. ESRCH means it is gone.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }

    /// Close the range after the direct child has settled.
    pub fn close(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            // Closing a kill-on-close job also reaps remaining descendants.
            // SAFETY: close the owned job handle exactly once.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(
                    job as windows_sys::Win32::Foundation::HANDLE,
                );
            }
        }
        #[cfg(unix)]
        self.terminate();
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(windows)]
fn create_job_for_pid(pid: u32) -> Option<isize> {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    if pid == 0 {
        return None;
    }
    // SAFETY: all handles and pointers are local to this function; every
    // failure closes the job and every process handle is closed.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set_ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if set_ok == 0 {
            CloseHandle(job);
            return None;
        }
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            CloseHandle(job);
            return None;
        }
        let assign_ok = AssignProcessToJobObject(job, process);
        CloseHandle(process);
        if assign_ok == 0 {
            CloseHandle(job);
            return None;
        }
        Some(job as isize)
    }
}
