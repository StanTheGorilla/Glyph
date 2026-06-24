//! Make spawned sidecars (the `llama-server` cleanup LLM and the ASR sidecar) die
//! together with the parent — including on a Task Manager kill or a `Quit` that
//! calls `process::exit` (neither of which runs `Drop`). Uses a Windows job object
//! set to `KILL_ON_JOB_CLOSE`: the parent holds the only handle, so when it exits
//! (or is force-killed) the job's last handle closes and every process still in the
//! job is terminated.
//!
//! We assign *each child* to the job explicitly (not only the parent) so cleanup
//! still works even if the parent itself can't be added to the job. No-op on
//! non-Windows.

#[cfg(windows)]
mod imp {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::OnceLock;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject,
        JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    /// The process-wide kill-on-close job. `HANDLE` is just a pointer, so wrap it to
    /// store in a `static`. We intentionally never close it: leaving the handle open
    /// for the whole process keeps `KILL_ON_JOB_CLOSE` armed until we die.
    struct Job(HANDLE);
    // SAFETY: the handle is only ever passed to job APIs that take it by value; we
    // never mutate it after creation.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    static JOB: OnceLock<Option<Job>> = OnceLock::new();

    fn job() -> Option<HANDLE> {
        JOB.get_or_init(create).as_ref().map(|j| j.0)
    }

    fn create() -> Option<Job> {
        unsafe {
            let job = match CreateJobObjectW(None, PCWSTR::null()) {
                Ok(h) => h,
                Err(e) => {
                    record(true, format!("CreateJobObject failed: {e}"));
                    return None;
                }
            };
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            // Best-effort: also add the current process so any descendant we don't
            // explicitly `guard` (e.g. WebView2 helpers) is covered too. Children
            // are guarded individually below, so this failing isn't fatal.
            let assign = AssignProcessToJobObject(job, GetCurrentProcess());
            record(
                set.is_err() || assign.is_err(),
                format!("init: set_kill_flag={set:?} assign_self={assign:?}"),
            );
            Some(Job(job))
        }
    }

    /// Create the kill-job (and add the current process). Call once at startup so
    /// the job is armed before any children spawn.
    pub fn init() {
        let _ = job();
    }

    /// Put a freshly-spawned child into the kill-job so it dies with us.
    pub fn guard(child: &Child) {
        let Some(job) = job() else {
            record(true, "guard: job unavailable".into());
            return;
        };
        let proc = HANDLE(child.as_raw_handle());
        let r = unsafe { AssignProcessToJobObject(job, proc) };
        record(r.is_err(), format!("guard pid {}: assign={r:?}", child.id()));
    }

    /// Log to stderr always; on failure also append to a temp-dir breadcrumb so a
    /// GUI build (no console) can be diagnosed after the fact.
    fn record(failure: bool, msg: String) {
        eprintln!("[proc_guard] {msg}");
        if failure {
            use std::io::Write;
            let path = std::env::temp_dir().join("glyph-proc-guard.log");
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(f, "{msg}");
            }
        }
    }
}

#[cfg(windows)]
pub use imp::{guard, init};

#[cfg(not(windows))]
pub fn init() {}
#[cfg(not(windows))]
pub fn guard(_child: &std::process::Child) {}
