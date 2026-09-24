use super::required_str;
use serde_json::{Value, json};
use std::path::Path;

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::OsStrExt;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct UniqueProcess {
        pid: u32,
        start: FileTime,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ProcessInfo {
        process: UniqueProcess,
        app_name: [u16; 256],
        service_name: [u16; 64],
        app_type: u32,
        app_status: u32,
        session_id: u32,
        restartable: i32,
    }
    impl Default for ProcessInfo {
        fn default() -> Self {
            Self {
                process: UniqueProcess::default(),
                app_name: [0; 256],
                service_name: [0; 64],
                app_type: 0,
                app_status: 0,
                session_id: 0,
                restartable: 0,
            }
        }
    }
    #[link(name = "Rstrtmgr")]
    unsafe extern "system" {
        fn RmStartSession(handle: *mut u32, flags: u32, key: *mut u16) -> u32;
        fn RmRegisterResources(
            handle: u32,
            file_count: u32,
            files: *const *const u16,
            process_count: u32,
            processes: *const UniqueProcess,
            service_count: u32,
            services: *const *const u16,
        ) -> u32;
        fn RmGetList(
            handle: u32,
            needed: *mut u32,
            count: *mut u32,
            info: *mut ProcessInfo,
            reasons: *mut u32,
        ) -> u32;
        fn RmEndSession(handle: u32) -> u32;
    }
    struct Session(u32);
    impl Drop for Session {
        fn drop(&mut self) {
            unsafe {
                RmEndSession(self.0);
            }
        }
    }
    fn wide_text(value: &[u16]) -> String {
        let end = value.iter().position(|&v| v == 0).unwrap_or(value.len());
        String::from_utf16_lossy(&value[..end])
    }
    pub fn holders(args: &Value) -> Result<Value, String> {
        let path = Path::new(required_str(args, "path")?);
        let path = path
            .canonicalize()
            .map_err(|e| super::super::io_error("Cannot resolve file", e))?;
        if !path.is_file() {
            return Err("'path' must be an existing file.".into());
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut key = [0u16; 64];
        let mut handle = 0u32;
        let code = unsafe { RmStartSession(&mut handle, 0, key.as_mut_ptr()) };
        if code != 0 {
            return Err(format!("RmStartSession failed with code {code}."));
        }
        let _session = Session(handle);
        let file_ptr = wide.as_ptr();
        let code = unsafe {
            RmRegisterResources(
                handle,
                1,
                &file_ptr,
                0,
                std::ptr::null(),
                0,
                std::ptr::null(),
            )
        };
        if code != 0 {
            return Err(format!("RmRegisterResources failed with code {code}."));
        }
        let mut needed = 0u32;
        let mut count = 0u32;
        let mut reasons = 0u32;
        let code = unsafe {
            RmGetList(
                handle,
                &mut needed,
                &mut count,
                std::ptr::null_mut(),
                &mut reasons,
            )
        };
        if code != 0 && code != 234 {
            return Err(format!("RmGetList failed with code {code}."));
        }
        if needed > 4096 {
            return Err("Too many lock holders to return.".into());
        }
        let mut rows = vec![ProcessInfo::default(); needed as usize];
        if needed > 0 {
            count = needed;
            let code = unsafe {
                RmGetList(
                    handle,
                    &mut needed,
                    &mut count,
                    rows.as_mut_ptr(),
                    &mut reasons,
                )
            };
            if code != 0 {
                return Err(format!("RmGetList failed with code {code}."));
            }
        }
        let processes: Vec<_> = rows.iter().take(count as usize).map(|row| json!({
            "pid":row.process.pid,"name":wide_text(&row.app_name),"service":wide_text(&row.service_name),
            "applicationType":row.app_type,"restartable":row.restartable != 0,"sessionId":row.session_id
        })).collect();
        Ok(
            json!({"path":path.to_string_lossy(),"processes":processes,"count":processes.len(),"rebootReasons":reasons}),
        )
    }
}

pub fn holders(args: &Value) -> Result<Value, String> {
    #[cfg(windows)]
    {
        windows::holders(args)
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Err("file_lock_holders is available only on Windows.".into())
    }
}
