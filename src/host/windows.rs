use super::{optional_str, optional_u64, required_str, required_u64};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

const SECURITY_MODULE: &str = "Import-Module ($PSHOME + '/Modules/Microsoft.PowerShell.Security/Microsoft.PowerShell.Security.psd1') -ErrorAction Stop;";

#[cfg(windows)]
use std::os::windows::process::CommandExt;

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        result.push(ALPHABET[(a >> 2) as usize] as char);
        result.push(ALPHABET[(((a & 3) << 4) | (b >> 4)) as usize] as char);
        result.push(if chunk.len() > 1 {
            ALPHABET[(((b & 15) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        result.push(if chunk.len() > 2 {
            ALPHABET[(c & 63) as usize] as char
        } else {
            '='
        });
    }
    result
}

fn powershell(
    script: &str,
    data: Option<&Value>,
    timeout: u64,
    max_output: usize,
) -> Result<Value, String> {
    if !cfg!(windows) {
        return Err("This operation is available only on Windows.".into());
    }
    let utf16 = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
        &base64(&utf16),
    ]);
    if let Some(data) = data {
        command.env("EVERYMCP_ARGS", data.to_string());
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    super::process::run_capture(command, timeout, max_output)
}

fn powershell_json(body: &str, args: &Value, timeout: u64) -> Result<Value, String> {
    let script = format!(
        "$ErrorActionPreference = 'Stop'; $a = $env:EVERYMCP_ARGS | ConvertFrom-Json; $result = & {{ {body} }}; ConvertTo-Json -InputObject $result -Depth 12 -Compress"
    );
    let output = powershell(&script, Some(args), timeout, 1024 * 1024)?;
    if output["timedOut"] == true {
        return Err("PowerShell operation timed out.".into());
    }
    if output["exitCode"] != 0 {
        return Err(format!(
            "PowerShell operation failed: {} {}",
            output["stderr"], output["stdout"]
        ));
    }
    serde_json::from_str(output["stdout"].as_str().unwrap_or(""))
        .map_err(|e| format!("PowerShell returned invalid JSON: {e}"))
}

pub(super) fn file_version(path: &Path) -> Result<Option<String>, String> {
    let data = json!({"path":path.to_string_lossy()});
    let value = powershell_json(
        "$info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo([string]$a.path); [pscustomobject]@{ fileVersion=$info.FileVersion }",
        &data,
        15000,
    )?;
    Ok(value["fileVersion"].as_str().map(str::to_owned))
}

fn file_signature(args: &Value) -> Result<Value, String> {
    let path = required_str(args, "path")?;
    if !Path::new(path).is_file() {
        return Err("'path' must be an existing file.".into());
    }
    let body = format!(
        "{SECURITY_MODULE} $sig = Get-AuthenticodeSignature -LiteralPath ([string]$a.path) -ErrorAction Stop; $info = [System.Diagnostics.FileVersionInfo]::GetVersionInfo([string]$a.path); [pscustomobject]@{{ path=$a.path; status=[string]$sig.Status; statusMessage=$sig.StatusMessage; signerSubject=$sig.SignerCertificate.Subject; signerThumbprint=$sig.SignerCertificate.Thumbprint; fileVersion=$info.FileVersion; productVersion=$info.ProductVersion }}"
    );
    powershell_json(&body, args, 30000)
}

fn acl_get(args: &Value) -> Result<Value, String> {
    let path = required_str(args, "path")?;
    if !Path::new(path).exists() {
        return Err("'path' does not exist.".into());
    }
    let body = format!(
        "{SECURITY_MODULE} $acl = Get-Acl -LiteralPath ([string]$a.path) -ErrorAction Stop; [pscustomobject]@{{ path=$a.path; owner=$acl.Owner; group=$acl.Group; sddl=$acl.Sddl; access=@($acl.Access | ForEach-Object {{ [pscustomobject]@{{ identity=[string]$_.IdentityReference; rights=[string]$_.FileSystemRights; type=[string]$_.AccessControlType; inherited=$_.IsInherited }} }}) }}"
    );
    powershell_json(&body, args, 30000)
}

fn scheduled_task(args: &Value) -> Result<Value, String> {
    let operation = required_str(args, "operation")?;
    if ![
        "list",
        "get",
        "run",
        "stop",
        "enable",
        "disable",
        "register",
        "unregister",
    ]
    .contains(&operation)
    {
        return Err("Unsupported scheduled task operation.".into());
    }
    let limit = optional_u64(args, "limit", 100, 1000)?;
    if limit == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    if operation != "list" {
        required_str(args, "name")?;
    }
    let mut data = args.clone();
    data["limit"] = json!(limit);
    if data.get("taskPath").is_none() {
        data["taskPath"] = json!("\\");
    }
    match operation {
        "list" => powershell_json(
            "$rows = Get-ScheduledTask -ErrorAction Stop | Where-Object { -not $a.name -or $_.TaskName -like ('*' + [string]$a.name + '*') } | Select-Object -First ([int]$a.limit) | ForEach-Object { [pscustomobject]@{ name=$_.TaskName; taskPath=$_.TaskPath; state=[string]$_.State } }; [pscustomobject]@{ tasks=@($rows) }",
            &data,
            60000,
        ),
        "get" => powershell_json(
            "$task = Get-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -ErrorAction Stop; $info = Get-ScheduledTaskInfo -InputObject $task -ErrorAction Stop; [pscustomobject]@{ name=$task.TaskName; taskPath=$task.TaskPath; state=[string]$task.State; enabled=$task.Settings.Enabled; actions=@($task.Actions | ForEach-Object { [pscustomobject]@{ execute=$_.Execute; arguments=$_.Arguments; workingDirectory=$_.WorkingDirectory } }); lastRunTime=$info.LastRunTime.ToString('o'); nextRunTime=$info.NextRunTime.ToString('o'); lastTaskResult=$info.LastTaskResult }",
            &data,
            30000,
        ),
        "run" | "stop" | "enable" | "disable" => powershell_json(
            "switch ($a.operation) { 'run' { Start-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -ErrorAction Stop } 'stop' { Stop-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -ErrorAction Stop } 'enable' { Enable-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -ErrorAction Stop | Out-Null } 'disable' { Disable-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -ErrorAction Stop | Out-Null } }; $task = Get-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -ErrorAction Stop; [pscustomobject]@{ name=$task.TaskName; taskPath=$task.TaskPath; state=[string]$task.State; operation=$a.operation }",
            &data,
            60000,
        ),
        "register" => {
            required_str(args, "executable")?;
            let trigger = required_str(args, "trigger")?;
            if !["once", "daily", "logon"].contains(&trigger) {
                return Err("'trigger' must be once, daily or logon.".into());
            }
            if trigger != "logon" {
                required_str(args, "at")?;
            }
            powershell_json(
                "$action = New-ScheduledTaskAction -Execute ([string]$a.executable) -Argument ([string]$a.actionArguments); $trigger = switch ($a.trigger) { 'once' { New-ScheduledTaskTrigger -Once -At ([datetime]::Parse([string]$a.at)) } 'daily' { New-ScheduledTaskTrigger -Daily -At ([datetime]::Parse([string]$a.at)) } 'logon' { New-ScheduledTaskTrigger -AtLogOn } }; $task = Register-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -Action $action -Trigger $trigger -Force:([bool]$a.overwrite) -ErrorAction Stop; [pscustomobject]@{ name=$task.TaskName; taskPath=$task.TaskPath; state=[string]$task.State; operation='register' }",
                &data,
                60000,
            )
        }
        "unregister" => powershell_json(
            "Unregister-ScheduledTask -TaskName ([string]$a.name) -TaskPath ([string]$a.taskPath) -Confirm:$false -ErrorAction Stop; [pscustomobject]@{ name=$a.name; taskPath=$a.taskPath; operation='unregister'; removed=$true }",
            &data,
            60000,
        ),
        _ => unreachable!(),
    }
}

fn eventlog_follow(args: &Value) -> Result<Value, String> {
    required_str(args, "logName")?;
    let max = optional_u64(args, "maxEvents", 20, 100)?;
    if max == 0 {
        return Err("'maxEvents' must be at least 1.".into());
    }
    if args.get("afterRecordId").is_some() && required_u64(args, "afterRecordId")? > i64::MAX as u64
    {
        return Err("'afterRecordId' is too large.".into());
    }
    let mut data = args.clone();
    data["maxEvents"] = json!(max);
    powershell_json(
        "$rows = @(); try { if ($null -ne $a.afterRecordId) { $xpath = '*[System[EventRecordID > {0}]]' -f [long]$a.afterRecordId; $rows = @(Get-WinEvent -LogName ([string]$a.logName) -FilterXPath $xpath -Oldest -MaxEvents ([int]$a.maxEvents) -ErrorAction Stop) } else { $rows = @(Get-WinEvent -LogName ([string]$a.logName) -MaxEvents ([int]$a.maxEvents) -ErrorAction Stop | Sort-Object RecordId) } } catch { if ($_.FullyQualifiedErrorId -notlike 'NoMatchingEventsFound*') { throw } }; $events = @($rows | ForEach-Object { [pscustomobject]@{ recordId=$_.RecordId; id=$_.Id; provider=$_.ProviderName; level=$_.LevelDisplayName; time=$_.TimeCreated.ToString('o'); message=([string]$_.Message).Substring(0,[Math]::Min(2000,([string]$_.Message).Length)) } }); $next = if ($rows.Count -gt 0) { [long]$rows[-1].RecordId } else { $a.afterRecordId }; [pscustomobject]@{ logName=$a.logName; events=$events; nextRecordId=$next }",
        &data,
        30000,
    )
}

fn modules(args: &Value) -> Result<Value, String> {
    let pid = required_u64(args, "pid")?;
    if pid > u32::MAX as u64 {
        return Err("'pid' is too large.".into());
    }
    powershell_json(
        "$p = Get-Process -Id ([int]$a.pid) -ErrorAction Stop; [pscustomobject]@{ pid = [int]$a.pid; modules = @($p.Modules | ForEach-Object { [pscustomobject]@{ name=$_.ModuleName; path=$_.FileName; baseAddress=('0x{0:X}' -f $_.BaseAddress.ToInt64()); sizeBytes=$_.ModuleMemorySize } }) }",
        args,
        30000,
    )
}

fn port_owner(args: &Value) -> Result<Value, String> {
    let port = required_u64(args, "port")?;
    if port == 0 || port > 65535 {
        return Err("'port' must be between 1 and 65535.".into());
    }
    let protocol = optional_str(args, "protocol")?
        .unwrap_or("tcp")
        .to_lowercase();
    if protocol != "tcp" && protocol != "udp" {
        return Err("'protocol' must be tcp or udp.".into());
    }
    let data = json!({"port":port,"protocol":protocol});
    powershell_json(
        "$rows = if ($a.protocol -eq 'tcp') { Get-NetTCPConnection -LocalPort ([int]$a.port) -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{ localAddress=$_.LocalAddress; localPort=$_.LocalPort; remoteAddress=$_.RemoteAddress; remotePort=$_.RemotePort; state=[string]$_.State; pid=$_.OwningProcess } } } else { Get-NetUDPEndpoint -LocalPort ([int]$a.port) -ErrorAction SilentlyContinue | ForEach-Object { [pscustomobject]@{ localAddress=$_.LocalAddress; localPort=$_.LocalPort; pid=$_.OwningProcess } } }; [pscustomobject]@{ protocol=$a.protocol; port=[int]$a.port; endpoints=@($rows) }",
        &data,
        30000,
    )
}

fn service_get(args: &Value) -> Result<Value, String> {
    powershell_json(
        r#"$name = [string]$a.name; $escaped = $name.Replace("'", "''"); $svc = Get-CimInstance Win32_Service -Filter ("Name = '{0}'" -f $escaped) -ErrorAction Stop; if (-not $svc) { throw 'Service not found.' }; [pscustomobject]@{ name=$svc.Name; displayName=$svc.DisplayName; state=$svc.State; startMode=$svc.StartMode; pid=$svc.ProcessId; pathName=$svc.PathName }"#,
        args,
        30000,
    )
}

fn service_control(args: &Value) -> Result<Value, String> {
    let action = required_str(args, "action")?;
    if !["start", "stop", "restart"].contains(&action) {
        return Err("'action' must be start, stop or restart.".into());
    }
    powershell_json(
        "$name = [string]$a.name; switch ($a.action) { 'start' { Start-Service -Name $name -ErrorAction Stop } 'stop' { Stop-Service -Name $name -ErrorAction Stop } 'restart' { Restart-Service -Name $name -ErrorAction Stop } }; $svc = Get-Service -Name $name -ErrorAction Stop; [pscustomobject]@{ name=$svc.Name; status=[string]$svc.Status; action=$a.action }",
        args,
        60000,
    )
}

fn eventlog_query(args: &Value) -> Result<Value, String> {
    let max = optional_u64(args, "maxEvents", 20, 100)?;
    if max == 0 {
        return Err("'maxEvents' must be at least 1.".into());
    }
    let data = json!({"logName":required_str(args, "logName")?,"maxEvents":max});
    powershell_json(
        "$rows = Get-WinEvent -LogName ([string]$a.logName) -MaxEvents ([int]$a.maxEvents) -ErrorAction Stop | ForEach-Object { [pscustomobject]@{ id=$_.Id; provider=$_.ProviderName; level=$_.LevelDisplayName; time=$_.TimeCreated.ToString('o'); message=([string]$_.Message).Substring(0,[Math]::Min(2000,([string]$_.Message).Length)) } }; [pscustomobject]@{ logName=$a.logName; events=@($rows) }",
        &data,
        30000,
    )
}

fn registry_read(args: &Value) -> Result<Value, String> {
    powershell_json(
        "$item = Get-ItemProperty -LiteralPath ([string]$a.path) -Name ([string]$a.property) -ErrorAction Stop; $entry = $item.PSObject.Properties[[string]$a.property]; if (-not $entry) { throw 'Registry property not found.' }; [pscustomobject]@{ path=$a.path; property=$a.property; value=$entry.Value }",
        args,
        30000,
    )
}

fn environment_get(args: &Value) -> Result<Value, String> {
    let name = required_str(args, "name")?;
    if name.is_empty() {
        return Err("'name' must not be empty.".into());
    }
    let scope = optional_str(args, "scope")?
        .unwrap_or("process")
        .to_lowercase();
    if !["process", "user", "machine"].contains(&scope.as_str()) {
        return Err("'scope' must be process, user or machine.".into());
    }
    if scope == "process" {
        return Ok(json!({"name":name,"scope":scope,"value":std::env::var(name).ok()}));
    }
    let data = json!({"name":name,"scope":scope});
    powershell_json(
        "$scope = [EnvironmentVariableTarget]::Parse([EnvironmentVariableTarget], [string]$a.scope, $true); [pscustomobject]@{ name=$a.name; scope=$a.scope; value=[Environment]::GetEnvironmentVariable([string]$a.name, $scope) }",
        &data,
        15000,
    )
}

fn powershell_run(args: &Value) -> Result<Value, String> {
    let script = required_str(args, "script")?;
    let timeout = optional_u64(args, "timeoutMs", 30000, 300000)?;
    let limit = optional_u64(args, "maxOutputBytes", 1024 * 1024, 4 * 1024 * 1024)? as usize;
    powershell(script, None, timeout, limit)
}

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "process_modules" => modules(args),
        "port_owner" => port_owner(args),
        "service_get" => service_get(args),
        "service_control" => service_control(args),
        "eventlog_query" => eventlog_query(args),
        "registry_read" => registry_read(args),
        "environment_get" => environment_get(args),
        "powershell_run" => powershell_run(args),
        "file_signature" => file_signature(args),
        "scheduled_task" => scheduled_task(args),
        "eventlog_follow" => eventlog_follow(args),
        "acl_get" => acl_get(args),
        _ => unreachable!(),
    }
}
