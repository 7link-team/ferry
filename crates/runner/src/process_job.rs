//! Windows child ownership shared by the service supervisor and command streams.
use anyhow::{Context, ensure};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use tokio::process::Child;
use windows_sys::Win32::System::JobObjects::*;

pub(crate) struct ProcessJob(OwnedHandle);

impl ProcessJob {
    pub(crate) fn terminate(&self) -> anyhow::Result<()> {
        ensure!(
            unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } != 0,
            "无法终止子进程作业"
        );
        Ok(())
    }

    pub(crate) fn attach(child: &Child) -> anyhow::Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            ensure!(!handle.is_null(), "无法创建子进程作业");
            let job = Self(OwnedHandle::from_raw_handle(handle));
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            ensure!(
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const _,
                    std::mem::size_of_val(&limits) as u32
                ) != 0,
                "无法配置子进程退出边界"
            );
            let process = child.raw_handle().context("子进程句柄不可用")?;
            ensure!(
                AssignProcessToJobObject(job.0.as_raw_handle(), process as _) != 0,
                "无法绑定子进程作业"
            );
            Ok(job)
        }
    }
}
