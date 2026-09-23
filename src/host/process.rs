use super::{
    io_error, optional_bool, optional_str, optional_u64, required_str, required_u64, string_array,
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

const DEFAULT_CAPTURE_BYTES: usize = 1024 * 1024;

#[derive(Default)]
struct Capture {
    data: Vec<u8>,
    truncated: bool,
    start_offset: u64,
    total_bytes: u64,
}

struct Managed {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Arc<Mutex<Capture>>,
    stderr: Arc<Mutex<Capture>>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    exit_code: Option<i32>,
}

static MANAGED: OnceLock<Mutex<HashMap<u32, Managed>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<u32, Managed>> {
    MANAGED.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_registry() -> Result<std::sync::MutexGuard<'static, HashMap<u32, Managed>>, String> {
    registry()
        .lock()
        .map_err(|_| "Process registry lock poisoned.".into())
}

fn capture_reader(
    mut reader: impl Read + Send + 'static,
    limit: usize,
) -> (Arc<Mutex<Capture>>, JoinHandle<()>) {
    let capture = Arc::new(Mutex::new(Capture::default()));
    let shared = Arc::clone(&capture);
    let handle = thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            let Ok(mut output) = shared.lock() else {
                break;
            };
            output.total_bytes += size as u64;
            output.data.extend_from_slice(&chunk[..size]);
            if output.data.len() > limit {
                let excess = output.data.len() - limit;
                output.data.drain(..excess);
                output.start_offset += excess as u64;
                output.truncated = true;
            }
        }
    });
    (capture, handle)
}

fn attach(mut child: Child, limit: usize) -> Result<Managed, String> {
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or("Child stdout was not piped.")?;
    let stderr = child.stderr.take().ok_or("Child stderr was not piped.")?;
    let (stdout, stdout_thread) = capture_reader(stdout, limit);
    let (stderr, stderr_thread) = capture_reader(stderr, limit);
    Ok(Managed {
        child,
        stdin,
        stdout,
        stderr,
        stdout_thread: Some(stdout_thread),
        stderr_thread: Some(stderr_thread),
        exit_code: None,
    })
}

fn finish_readers(managed: &mut Managed) {
    if let Some(handle) = managed.stdout_thread.take() {
        let _ = handle.join();
    }
    if let Some(handle) = managed.stderr_thread.take() {
        let _ = handle.join();
    }
}

fn output(
    managed: &Managed,
    stdout_offset: Option<u64>,
    stderr_offset: Option<u64>,
) -> Result<Value, String> {
    let stdout = managed.stdout.lock().map_err(|_| "stdout lock poisoned.")?;
    let stderr = managed.stderr.lock().map_err(|_| "stderr lock poisoned.")?;
    let stdout_from = stdout_offset.unwrap_or(stdout.start_offset);
    let stderr_from = stderr_offset.unwrap_or(stderr.start_offset);
    if stdout_from > stdout.total_bytes || stderr_from > stderr.total_bytes {
        return Err("Output offset is beyond the captured stream.".into());
    }
    let stdout_start = stdout_from.max(stdout.start_offset);
    let stderr_start = stderr_from.max(stderr.start_offset);
    Ok(json!({
        "stdout":String::from_utf8_lossy(&stdout.data[(stdout_start-stdout.start_offset) as usize..]),
        "stderr":String::from_utf8_lossy(&stderr.data[(stderr_start-stderr.start_offset) as usize..]),
        "stdoutTruncated":stdout.truncated,
        "stderrTruncated":stderr.truncated,
        "stdoutStartOffset":stdout_start,
        "stderrStartOffset":stderr_start,
        "nextStdoutOffset":stdout.total_bytes,
        "nextStderrOffset":stderr.total_bytes,
        "missedStdoutBytes":stdout_start.saturating_sub(stdout_from),
        "missedStderrBytes":stderr_start.saturating_sub(stderr_from)
    }))
}

fn update_exit(managed: &mut Managed) -> Result<Option<i32>, String> {
    if managed.exit_code.is_some() {
        return Ok(managed.exit_code);
    }
    match managed
        .child
        .try_wait()
        .map_err(|e| io_error("Cannot query child", e))?
    {
        Some(status) => {
            managed.exit_code = Some(status.code().unwrap_or(-1));
            finish_readers(managed);
            Ok(managed.exit_code)
        }
        None => Ok(None),
    }
}

fn command(args: &Value) -> Result<Command, String> {
    let executable = required_str(args, "executable")?;
    if executable.is_empty() {
        return Err("'executable' must not be empty.".into());
    }
    let arguments = string_array(args, "arguments", false)?;
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = optional_str(args, "cwd")? {
        command.current_dir(cwd);
    }
    if let Some(env) = args.get("env") {
        let map = env
            .as_object()
            .ok_or("'env' must be an object of strings.")?;
        for (key, value) in map {
            command.env(
                key,
                value.as_str().ok_or("'env' must contain only strings.")?,
            );
        }
    }
    Ok(command)
}

pub(super) fn run_capture(
    mut command: Command,
    timeout_ms: u64,
    limit: usize,
) -> Result<Value, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|e| io_error("Cannot start process", e))?;
    let pid = child.id();
    let mut managed = attach(child, limit)?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    loop {
        if update_exit(&mut managed)?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            managed
                .child
                .kill()
                .map_err(|e| io_error("Cannot stop timed-out process", e))?;
            let status: ExitStatus = managed
                .child
                .wait()
                .map_err(|e| io_error("Cannot wait for timed-out process", e))?;
            managed.exit_code = Some(status.code().unwrap_or(-1));
            finish_readers(&mut managed);
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let mut result = output(&managed, None, None)?;
    result["pid"] = json!(pid);
    result["exitCode"] = json!(managed.exit_code);
    result["timedOut"] = json!(timed_out);
    Ok(result)
}

fn start(args: &Value) -> Result<Value, String> {
    let mut entries = lock_registry()?;
    if entries.len() >= 64 {
        return Err("At most 64 managed processes are supported per server session.".into());
    }
    let mut command = command(args)?;
    command.stdin(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|e| io_error("Cannot start process", e))?;
    let pid = child.id();
    let managed = attach(child, DEFAULT_CAPTURE_BYTES)?;
    entries.insert(pid, managed);
    Ok(json!({"pid":pid,"captureLimitBytes":DEFAULT_CAPTURE_BYTES}))
}

fn run(args: &Value) -> Result<Value, String> {
    let timeout = optional_u64(args, "timeoutMs", 30000, 300000)?;
    let limit = optional_u64(
        args,
        "maxOutputBytes",
        DEFAULT_CAPTURE_BYTES as u64,
        4 * 1024 * 1024,
    )? as usize;
    run_capture(command(args)?, timeout, limit)
}

fn pid_arg(args: &Value) -> Result<u32, String> {
    let raw = required_u64(args, "pid")?;
    u32::try_from(raw).map_err(|_| "'pid' is too large.".into())
}

fn process_refresh() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_memory()
        .with_cpu()
        .with_disk_usage()
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
}

fn snapshot() -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh());
    system
}

fn info_json(pid: Pid, process: &sysinfo::Process) -> Value {
    let io = process.disk_usage();
    json!({
        "pid":pid.as_u32(),
        "name":process.name().to_string_lossy(),
        "exe":process.exe().map(|p| p.to_string_lossy().to_string()),
        "commandLine":process.cmd().iter().map(|v| v.to_string_lossy().to_string()).collect::<Vec<_>>(),
        "parentPid":process.parent().map(Pid::as_u32),
        "startTimeUnix":process.start_time(),
        "cpuPercent":process.cpu_usage(),
        "memoryBytes":process.memory(),
        "virtualMemoryBytes":process.virtual_memory(),
        "readBytes":io.total_read_bytes,
        "writtenBytes":io.total_written_bytes
    })
}

fn get(args: &Value) -> Result<Value, String> {
    let pid = Pid::from_u32(pid_arg(args)?);
    let sample_ms = optional_u64(args, "sampleMs", 200, 5000)?.max(100);
    let mut system = snapshot();
    if system.process(pid).is_none() {
        return Err("Process not found or inaccessible.".into());
    }
    thread::sleep(Duration::from_millis(sample_ms));
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, process_refresh());
    let process = system
        .process(pid)
        .ok_or("Process exited while sampling.")?;
    let mut result = info_json(pid, process);
    result["cpuSampleMs"] = json!(sample_ms);
    Ok(result)
}

fn list(args: &Value) -> Result<Value, String> {
    let system = snapshot();
    let filter = optional_str(args, "name")?.map(str::to_lowercase);
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    let mut processes: Vec<Value> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            let name = process.name().to_string_lossy();
            if filter
                .as_ref()
                .is_some_and(|value| !name.to_lowercase().contains(value))
            {
                return None;
            }
            Some(info_json(*pid, process))
        })
        .collect();
    processes.sort_by_key(|value| value["pid"].as_u64());
    let total = processes.len();
    processes.truncate(limit);
    Ok(json!({"processes":processes,"totalMatched":total,"limitReached":total>limit}))
}

fn tree(args: &Value) -> Result<Value, String> {
    let root = pid_arg(args)?;
    let system = snapshot();
    if system.process(Pid::from_u32(root)).is_none() {
        return Err("Process not found or inaccessible.".into());
    }
    let mut found = HashSet::from([root]);
    loop {
        let old = found.len();
        for (pid, process) in system.processes() {
            if process
                .parent()
                .is_some_and(|parent| found.contains(&parent.as_u32()))
            {
                found.insert(pid.as_u32());
            }
        }
        if found.len() == old {
            break;
        }
    }
    let mut processes: Vec<Value> = found
        .into_iter()
        .filter_map(|pid| {
            system
                .process(Pid::from_u32(pid))
                .map(|process| info_json(Pid::from_u32(pid), process))
        })
        .collect();
    processes.sort_by_key(|value| value["pid"].as_u64());
    Ok(json!({"rootPid":root,"processes":processes}))
}

fn wait(args: &Value) -> Result<Value, String> {
    let pid = pid_arg(args)?;
    let timeout = optional_u64(args, "timeoutMs", 30000, 300000)?;
    let deadline = Instant::now() + Duration::from_millis(timeout);
    loop {
        {
            let mut entries = lock_registry()?;
            if let Some(child) = entries.get_mut(&pid) {
                if let Some(code) = update_exit(child)? {
                    return Ok(json!({"pid":pid,"exited":true,"exitCode":code,"timedOut":false}));
                }
            } else {
                let mut system = System::new();
                system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
                if system.process(Pid::from_u32(pid)).is_none() {
                    return Ok(json!({"pid":pid,"exited":true,"exitCode":null,"timedOut":false}));
                }
            }
        }
        if Instant::now() >= deadline {
            return Ok(json!({"pid":pid,"exited":false,"exitCode":null,"timedOut":true}));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn managed_output(args: &Value) -> Result<Value, String> {
    let pid = pid_arg(args)?;
    let release = optional_bool(args, "release", false)?;
    let stdout_offset = args
        .get("stdoutOffset")
        .map(|_| required_u64(args, "stdoutOffset"))
        .transpose()?;
    let stderr_offset = args
        .get("stderrOffset")
        .map(|_| required_u64(args, "stderrOffset"))
        .transpose()?;
    let mut entries = lock_registry()?;
    let managed = entries
        .get_mut(&pid)
        .ok_or("PID was not started by this server session.")?;
    let exit = update_exit(managed)?;
    let mut result = output(managed, stdout_offset, stderr_offset)?;
    result["pid"] = json!(pid);
    result["exited"] = json!(exit.is_some());
    result["exitCode"] = json!(exit);
    if release {
        if exit.is_none() {
            return Err("Cannot release a running process capture.".into());
        }
        entries.remove(&pid);
    }
    Ok(result)
}

fn managed_input(args: &Value) -> Result<Value, String> {
    let pid = pid_arg(args)?;
    let close = optional_bool(args, "close", false)?;
    let encoding = optional_str(args, "encoding")?.unwrap_or("utf8");
    let raw = optional_str(args, "data")?.unwrap_or("");
    let bytes = match encoding {
        "utf8" => raw.as_bytes().to_vec(),
        "hex" => crate::bytes::parse_hex(raw)?,
        _ => return Err("'encoding' must be utf8 or hex.".into()),
    };
    let mut entries = lock_registry()?;
    let managed = entries
        .get_mut(&pid)
        .ok_or("PID was not started by this server session.")?;
    let stdin = managed
        .stdin
        .as_mut()
        .ok_or("Process stdin is already closed.")?;
    stdin
        .write_all(&bytes)
        .map_err(|e| io_error("Cannot write child stdin", e))?;
    stdin
        .flush()
        .map_err(|e| io_error("Cannot flush child stdin", e))?;
    if close {
        managed.stdin.take();
    }
    Ok(json!({"pid":pid,"bytesWritten":bytes.len(),"closed":close}))
}

fn stop(args: &Value) -> Result<Value, String> {
    let pid = pid_arg(args)?;
    if pid == std::process::id() {
        return Err("Refusing to stop the MCP server itself.".into());
    }
    let include_tree = optional_bool(args, "tree", false)?;
    let force = optional_bool(args, "force", true)?;
    let system = snapshot();
    let target = system
        .process(Pid::from_u32(pid))
        .ok_or("Process not found or inaccessible.")?;
    if let Some(expected) = args.get("expectedStartTimeUnix") {
        let expected = expected
            .as_u64()
            .ok_or("'expectedStartTimeUnix' must be a nonnegative integer.")?;
        if target.start_time() != expected {
            return Err("Process start time does not match; PID may have been reused.".into());
        }
    }
    if include_tree {
        let mut ancestors = HashSet::new();
        let mut current = Some(Pid::from_u32(std::process::id()));
        while let Some(value) = current {
            if !ancestors.insert(value) {
                break;
            }
            if value.as_u32() == pid {
                return Err("Refusing to stop a tree containing the MCP server.".into());
            }
            current = system.process(value).and_then(sysinfo::Process::parent);
        }
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("taskkill.exe");
        command.arg("/PID").arg(pid.to_string());
        if include_tree {
            command.arg("/T");
        }
        if force {
            command.arg("/F");
        }
        let result = run_capture(command, 15000, 65536)?;
        if result["exitCode"] != 0 {
            return Err(format!("taskkill failed: {}", result["stderr"]));
        }
        Ok(json!({"pid":pid,"tree":include_tree,"force":force,"stopped":true}))
    }
    #[cfg(not(windows))]
    {
        if include_tree {
            return Err("Tree termination is only supported on Windows.".into());
        }
        let success = target.kill();
        Ok(json!({"pid":pid,"tree":false,"force":force,"stopped":success}))
    }
}

fn system_info() -> Value {
    let system = System::new_all();
    json!({
        "osName":System::name(),
        "osVersion":System::os_version(),
        "kernelVersion":System::kernel_version(),
        "hostName":System::host_name(),
        "cpuCount":system.cpus().len(),
        "totalMemoryBytes":system.total_memory(),
        "availableMemoryBytes":system.available_memory()
    })
}

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "process_start" => start(args),
        "process_run" => run(args),
        "process_get" => get(args),
        "process_list" => list(args),
        "process_tree" => tree(args),
        "process_wait" => wait(args),
        "process_output" => managed_output(args),
        "process_input" => managed_input(args),
        "process_stop" => stop(args),
        "system_info" => Ok(system_info()),
        _ => unreachable!(),
    }
}
