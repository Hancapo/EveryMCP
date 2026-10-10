use super::{optional_u64, required_u64};
use serde_json::{Value, json};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    let pid = required_u64(args, "pid")?;
    if pid == 0 || pid > u32::MAX as u64 {
        return Err("'pid' must be between 1 and 4294967295.".into());
    }
    let limit = optional_u64(args, "limit", 200, 2000)?;
    if limit == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    match name {
        "process_connections" => connections(pid as u32, limit),
        "process_memory_map" => {
            let offset = optional_u64(args, "offset", 0, super::MAX_COLLECTION_ENTRIES as u64 - 1)?;
            memory_map(pid as u32, offset as usize, limit as usize)
        }
        _ => Err(format!("Unknown inspection tool '{name}'.")),
    }
}

fn connections(pid: u32, limit: u64) -> Result<Value, String> {
    super::windows::powershell_json(
        r#"
        Get-Process -Id ([int]$a.pid) -ErrorAction Stop | Out-Null
        $tcp = @(); $udp = @()
        try { $tcp = @(Get-NetTCPConnection -OwningProcess ([uint32]$a.pid) -ErrorAction Stop | Select-Object -First ([int]$a.limit + 1)) }
        catch { if ($_.FullyQualifiedErrorId -notlike '*CmdletizationQuery_NotFound*') { throw } }
        if ($tcp.Count -le [int]$a.limit) {
            try { $udp = @(Get-NetUDPEndpoint -OwningProcess ([uint32]$a.pid) -ErrorAction Stop | Select-Object -First ([int]$a.limit + 1 - $tcp.Count)) }
            catch { if ($_.FullyQualifiedErrorId -notlike '*CmdletizationQuery_NotFound*') { throw } }
        }
        $rows = @($tcp | ForEach-Object { [pscustomobject]@{
            protocol='tcp'; addressFamily=$(if ($_.LocalAddress.Contains(':')) {'IPv6'} else {'IPv4'});
            localAddress=$_.LocalAddress; localPort=$_.LocalPort; remoteAddress=$_.RemoteAddress;
            remotePort=$_.RemotePort; state=[string]$_.State; pid=$_.OwningProcess
        } }) + @($udp | ForEach-Object { [pscustomobject]@{
            protocol='udp'; addressFamily=$(if ($_.LocalAddress.Contains(':')) {'IPv6'} else {'IPv4'});
            localAddress=$_.LocalAddress; localPort=$_.LocalPort; remoteAddress=$null;
            remotePort=$null; state=$null; pid=$_.OwningProcess
        } })
        [pscustomobject]@{ pid=$a.pid; connections=@($rows | Select-Object -First ([int]$a.limit)); truncated=($rows.Count -gt [int]$a.limit) }
        "#,
        &json!({"pid":pid,"limit":limit}),
        30_000,
    )
}

#[cfg(not(windows))]
fn memory_map(_pid: u32, _offset: usize, _limit: usize) -> Result<Value, String> {
    Err("process_memory_map is available only on Windows.".into())
}

#[cfg(windows)]
fn memory_map(pid: u32, offset: usize, limit: usize) -> Result<Value, String> {
    use super::process_dump::windows_impl::ProcessHandle;
    use std::collections::HashMap;
    use std::ffi::c_void;
    use windows::Win32::Foundation::{ERROR_INVALID_PARAMETER, GetLastError};
    use windows::Win32::System::Memory::{
        MEM_COMMIT, MEM_FREE, MEM_IMAGE, MEM_MAPPED, MEM_PRIVATE, MEM_RESERVE,
        MEMORY_BASIC_INFORMATION, VirtualQueryEx,
    };
    use windows::Win32::System::ProcessStatus::K32GetMappedFileNameW;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION};

    let process = ProcessHandle(
        unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) }
            .map_err(|e| format!("Cannot open PID {pid} for memory queries: {e}"))?,
    );
    let mut regions = Vec::new();
    let mut modules = HashMap::new();
    let mut address = 0usize;
    let mut index = 0usize;
    let mut more = false;
    let mut scan_limit_reached = false;
    loop {
        if index == super::MAX_COLLECTION_ENTRIES {
            scan_limit_reached = true;
            break;
        }
        let mut info = MEMORY_BASIC_INFORMATION::default();
        let queried = unsafe {
            VirtualQueryEx(
                process.0,
                Some(address as *const c_void),
                &mut info,
                std::mem::size_of_val(&info),
            )
        };
        if queried == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_INVALID_PARAMETER && index > 0 {
                break;
            }
            return Err(format!("VirtualQueryEx failed at 0x{address:x}: {error:?}"));
        }
        if index >= offset && regions.len() == limit {
            more = true;
            break;
        }
        let base = info.BaseAddress as usize;
        let next = base
            .checked_add(info.RegionSize)
            .filter(|&value| value > address)
            .ok_or("VirtualQueryEx returned an invalid region extent.")?;
        if index >= offset {
            let allocation = info.AllocationBase as usize;
            let (module, module_error) = if info.Type == MEM_IMAGE {
                modules.entry(allocation).or_insert_with(|| {
                    let mut name = vec![0u16; 32768];
                    let length = unsafe {
                        K32GetMappedFileNameW(process.0, info.BaseAddress, &mut name)
                    } as usize;
                    if length == 0 || length >= name.len() {
                        (Value::Null, json!(format!("Cannot resolve image path: {:?}", unsafe { GetLastError() })))
                    } else {
                        (json!({"baseAddress":format!("0x{allocation:x}"),"path":String::from_utf16_lossy(&name[..length]),"pathKind":"device"}), Value::Null)
                    }
                }).clone()
            } else {
                (Value::Null, Value::Null)
            };
            let state = match info.State {
                MEM_COMMIT => "MEM_COMMIT",
                MEM_RESERVE => "MEM_RESERVE",
                MEM_FREE => "MEM_FREE",
                _ => "UNKNOWN",
            };
            let kind = match info.Type {
                MEM_IMAGE => "MEM_IMAGE",
                MEM_MAPPED => "MEM_MAPPED",
                MEM_PRIVATE => "MEM_PRIVATE",
                _ => "NONE",
            };
            regions.push(json!({
                "baseAddress":format!("0x{base:x}"),"allocationBase":format!("0x{allocation:x}"),
                "sizeBytes":info.RegionSize,"state":state,"type":kind,
                "protect":format!("0x{:x}",info.Protect.0),"protectFlags":protection_flags(info.Protect.0),
                "allocationProtect":format!("0x{:x}",info.AllocationProtect.0),"module":module,"moduleError":module_error,
            }));
        }
        address = next;
        index += 1;
    }
    Ok(
        json!({"pid":pid,"offset":offset,"limit":limit,"regions":regions,
        "nextOffset":more.then_some(index),"truncated":more || scan_limit_reached,"scanLimitReached":scan_limit_reached}),
    )
}

#[cfg(windows)]
fn protection_flags(protect: u32) -> Vec<&'static str> {
    [
        (0x01, "PAGE_NOACCESS"),
        (0x02, "PAGE_READONLY"),
        (0x04, "PAGE_READWRITE"),
        (0x08, "PAGE_WRITECOPY"),
        (0x10, "PAGE_EXECUTE"),
        (0x20, "PAGE_EXECUTE_READ"),
        (0x40, "PAGE_EXECUTE_READWRITE"),
        (0x80, "PAGE_EXECUTE_WRITECOPY"),
        (0x100, "PAGE_GUARD"),
        (0x200, "PAGE_NOCACHE"),
        (0x400, "PAGE_WRITECOMBINE"),
        (0x40000000, "PAGE_TARGETS_INVALID"),
    ]
    .into_iter()
    .filter_map(|(flag, name)| (protect & flag != 0).then_some(name))
    .collect()
}
