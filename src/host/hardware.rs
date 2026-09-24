use serde_json::{Value, json};

pub fn inventory() -> Result<Value, String> {
    if !cfg!(windows) {
        return Err("hardware_inventory is available only on Windows.".into());
    }
    let mut result = super::windows::powershell_json(
        "$cpu = @(Get-CimInstance Win32_Processor -ErrorAction Stop | ForEach-Object { [pscustomobject]@{ name=([string]$_.Name).Trim(); manufacturer=$_.Manufacturer; cores=$_.NumberOfCores; logicalProcessors=$_.NumberOfLogicalProcessors; maxClockMHz=$_.MaxClockSpeed } }); \
         $gpu = @(Get-CimInstance Win32_VideoController -ErrorAction Stop | ForEach-Object { [pscustomobject]@{ name=$_.Name; reportedAdapterRAMBytes=$_.AdapterRAM; driverVersion=$_.DriverVersion; driverDate=$_.DriverDate; videoProcessor=$_.VideoProcessor; status=$_.Status } }); \
         $ram = @(Get-CimInstance Win32_PhysicalMemory -ErrorAction Stop | ForEach-Object { [pscustomobject]@{ bankLabel=$_.BankLabel; deviceLocator=$_.DeviceLocator; manufacturer=$_.Manufacturer; partNumber=([string]$_.PartNumber).Trim(); capacityBytes=$_.Capacity; speedMHz=$_.Speed; configuredClockMHz=$_.ConfiguredClockSpeed } }); \
         $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop; [pscustomobject]@{ cpu=$cpu; gpu=$gpu; memoryModules=$ram; os=[pscustomobject]@{ caption=$os.Caption; version=$os.Version; buildNumber=$os.BuildNumber; architecture=$os.OSArchitecture }; totalPhysicalMemoryBytes=$os.TotalVisibleMemorySize * 1024 }",
        &json!({}),
        30000,
    )?;
    #[cfg(windows)]
    match dxgi_adapters() {
        Ok(adapters) => result["dxgiAdapters"] = json!(adapters),
        Err(error) => result["dxgiError"] = json!(error),
    }
    Ok(result)
}

#[cfg(windows)]
fn dxgi_adapters() -> Result<Vec<Value>, String> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIFactory1};
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("DXGI factory failed: {e}"))?;
    let mut adapters = Vec::new();
    for index in 0..32 {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(value) => value,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(format!("DXGI enumeration failed: {error}")),
        };
        let desc = unsafe { adapter.GetDesc1() }
            .map_err(|e| format!("DXGI adapter description failed: {e}"))?;
        let end = desc
            .Description
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(desc.Description.len());
        adapters.push(json!({"name":String::from_utf16_lossy(&desc.Description[..end]),
            "vendorId":format!("0x{:04x}",desc.VendorId),"deviceId":format!("0x{:04x}",desc.DeviceId),
            "dedicatedVideoMemoryBytes":desc.DedicatedVideoMemory,
            "dedicatedSystemMemoryBytes":desc.DedicatedSystemMemory,
            "sharedSystemMemoryBytes":desc.SharedSystemMemory}));
    }
    Ok(adapters)
}
