use std::ffi::c_void;

use eyre::{bail, Result};
use tracing::error;
use windows::Win32::{
    Foundation::GetLastError,
    System::{
        Diagnostics::Debug::WriteProcessMemory,
        Memory::{VirtualAllocEx, MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE},
    },
};

use crate::{helpers::OwnedHandle, popup::warn_popup};

pub fn write_in<T>(process: &OwnedHandle, data: *const T, size: usize) -> Result<*const c_void> {
    let alloc_addr = {
        let addr = unsafe {
            VirtualAllocEx(
                process.as_raw_handle(),
                None,
                size,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_EXECUTE_READWRITE,
            )
        };

        if addr.is_null() {
            let error = unsafe { GetLastError() };
            error!(?error, "virtualallocex failed to allocate memory");

            warn_popup(
                "Process injection failure",
                format!("Failed to allocate memory in target process\n\nThis could be due to multiple reasons, but in any case, winapi returned an error that we cannot recover from. Recommend restarting game and trying again\n\nError: {error:?}"),
            );

            bail!("failed to allocate");
        }

        addr
    };

    // Write the data to the process
    let res =
        unsafe { WriteProcessMemory(process.as_raw_handle(), alloc_addr, data.cast(), size, None) };

    if let Err(e) = res {
        error!(?e, "Failed to write to process");

        warn_popup(
            "Process injection failure",
            format!("Failed to write to process memory\n\nThis could be due to multiple reasons, but in any case, winapi returned an error that we cannot recover from. Recommend restarting game and trying again\n\nError: {e}"),
        );

        bail!("failed to write to process");
    }

    Ok(alloc_addr)
}