//! RAII wrapper around a Windows Job Object with `KILL_ON_JOB_CLOSE`.
//!
//! Assigned processes (and the children they spawn afterwards) are terminated
//! when the last handle to the job closes — i.e. when this value drops. On
//! non-Windows targets it is a no-op, since the production target is Windows.

use crate::{CoreError, Result};

fn job_err(msg: impl std::fmt::Display) -> CoreError {
    CoreError::Runtime {
        runtime: "supervisor".into(),
        message: msg.to_string(),
    }
}

#[cfg(windows)]
mod imp {
    use super::{job_err, Result};
    use win32job::{ExtendedLimitInfo, Job};

    /// Owns a job object configured to kill its processes on close.
    /// `win32job::Job` is already `Send + Sync`, so no `unsafe impl` is needed.
    #[derive(Debug)]
    pub struct JobObject {
        job: Job,
    }

    impl JobObject {
        pub fn new() -> Result<Self> {
            let mut info = ExtendedLimitInfo::new();
            info.limit_kill_on_job_close();
            let job = Job::create_with_limit_info(&info)
                .map_err(|e| job_err(format!("create job object: {e}")))?;
            Ok(Self { job })
        }

        pub fn assign_child(&self, child: &tokio::process::Child) -> Result<()> {
            let handle = child
                .raw_handle()
                .ok_or_else(|| job_err("child process has no handle (already exited?)"))?;
            self.job
                .assign_process(handle as isize)
                .map_err(|e| job_err(format!("assign process to job object: {e}")))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Result;

    #[derive(Debug)]
    pub struct JobObject;

    impl JobObject {
        pub fn new() -> Result<Self> {
            Ok(Self)
        }
        pub fn assign_child(&self, _child: &tokio::process::Child) -> Result<()> {
            Ok(())
        }
    }
}

pub use imp::JobObject;

#[cfg(all(test, windows))]
mod tests {
    use std::time::Duration;

    use super::JobObject;

    fn pid_alive(pid: u32) -> bool {
        // `tasklist` is always present on Windows.
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .expect("run tasklist");
        String::from_utf8_lossy(&out.stdout).contains(&pid.to_string())
    }

    #[tokio::test]
    async fn dropping_the_job_kills_assigned_processes() {
        let child = tokio::process::Command::new("ping")
            .args(["-n", "3600", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn ping");
        let pid = child.id().expect("pid");

        let job = JobObject::new().unwrap();
        job.assign_child(&child).unwrap();
        assert!(pid_alive(pid), "child should be running after assignment");

        drop(job);
        // Leak the Child handle so std doesn't reap it; the job kill is what we test.
        std::mem::forget(child);

        // KILL_ON_JOB_CLOSE is effectively immediate; allow scheduling slack.
        for _ in 0..40 {
            if !pid_alive(pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("child {pid} survived the job object closing");
    }
}
