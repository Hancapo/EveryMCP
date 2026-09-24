use super::{optional_str, required_str};
use serde_json::{Value, json};
use std::path::Path;

pub fn open(args: &Value) -> Result<Value, String> {
    let mode = optional_str(args, "mode")?.unwrap_or("open");
    if mode != "open" && mode != "reveal" {
        return Err("'mode' must be 'open' or 'reveal'.".into());
    }
    let supplied = Path::new(required_str(args, "path")?);
    let path = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| format!("Cannot resolve current directory: {e}"))?
            .join(supplied)
    };
    let metadata = std::fs::metadata(&path)
        .map_err(|_| "'path' must name an existing file or directory.".to_owned())?;
    if !metadata.is_file() && !metadata.is_dir() {
        return Err("'path' must name an existing file or directory.".into());
    }
    open_platform(&path, mode)?;
    Ok(json!({"path":path,"mode":mode,"shellRequestAccepted":true}))
}

#[cfg(not(windows))]
fn open_platform(_path: &Path, _mode: &str) -> Result<(), String> {
    Err("file_open is only available on Windows.".into())
}

#[cfg(windows)]
fn open_platform(path: &Path, mode: &str) -> Result<(), String> {
    windows::open(path, mode)
}

#[cfg(windows)]
mod windows {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::{null, null_mut};

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
        fn CoUninitialize();
        fn CoTaskMemFree(memory: *mut c_void);
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            window: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show: i32,
        ) -> isize;
        fn SHParseDisplayName(
            name: *const u16,
            bind_context: *mut c_void,
            item: *mut *mut c_void,
            attributes: u32,
            output_attributes: *mut u32,
        ) -> i32;
        fn SHOpenFolderAndSelectItems(
            folder: *const c_void,
            count: u32,
            items: *const *const c_void,
            flags: u32,
        ) -> i32;
    }

    struct ComApartment;

    impl ComApartment {
        fn initialize() -> Result<Self, String> {
            // COINIT_APARTMENTTHREADED is required by shell extensions used here.
            let result = unsafe { CoInitializeEx(null_mut(), 0x2) };
            if result < 0 {
                return Err(format!(
                    "Cannot initialize Windows shell COM apartment: 0x{:08X}.",
                    result as u32
                ));
            }
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn wide(value: &OsStr) -> Result<Vec<u16>, String> {
        let mut units: Vec<u16> = value.encode_wide().collect();
        if units.contains(&0) {
            return Err("Path contains a null character.".into());
        }
        units.push(0);
        Ok(units)
    }

    pub(super) fn open(path: &Path, mode: &str) -> Result<(), String> {
        let _apartment = ComApartment::initialize()?;
        let name = wide(path.as_os_str())?;
        if mode == "open" {
            let parent = path.parent().ok_or("Path has no parent directory.")?;
            let directory = wide(parent.as_os_str())?;
            let result = unsafe {
                ShellExecuteW(
                    null_mut(),
                    null(),
                    name.as_ptr(),
                    null(),
                    directory.as_ptr(),
                    1,
                )
            };
            if result <= 32 {
                return Err(format!(
                    "Windows could not open the path (ShellExecute code {result})."
                ));
            }
            return Ok(());
        }
        let mut item: *mut c_void = null_mut();
        let parsed =
            unsafe { SHParseDisplayName(name.as_ptr(), null_mut(), &mut item, 0, null_mut()) };
        if parsed < 0 {
            return Err(format!(
                "Windows could not resolve the path for Explorer: 0x{:08X}.",
                parsed as u32
            ));
        }
        let selected = unsafe { SHOpenFolderAndSelectItems(item, 0, null(), 0) };
        unsafe { CoTaskMemFree(item) };
        if selected < 0 {
            return Err(format!(
                "Explorer could not select the path: 0x{:08X}.",
                selected as u32
            ));
        }
        Ok(())
    }
}
