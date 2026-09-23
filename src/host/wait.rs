use super::{optional_u64, required_str, required_u64};
use serde_json::{Value, json};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessesToUpdate, System};

pub fn execute(args: &Value) -> Result<Value, String> {
    let kind = required_str(args, "kind")?;
    let timeout_ms = optional_u64(args, "timeoutMs", 30000, 300000)?;
    let interval_ms = optional_u64(args, "intervalMs", 200, 5000)?;
    if interval_ms < 50 {
        return Err("'intervalMs' must be at least 50.".into());
    }
    match kind {
        "file_exists" => {
            required_str(args, "path")?;
        }
        "process_exit" => {
            required_u64(args, "pid")?;
        }
        "tcp" => {
            required_str(args, "host")?;
            validate_port(args)?;
        }
        "http" => {
            let url = required_str(args, "url")?;
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err("'url' must use http or https.".into());
            }
        }
        _ => return Err("'kind' must be file_exists, process_exit, tcp or http.".into()),
    }
    let start = Instant::now();
    let deadline = start + Duration::from_millis(timeout_ms);
    let mut attempts = 0;
    loop {
        attempts += 1;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let probe_timeout = remaining
            .min(Duration::from_secs(5))
            .max(Duration::from_millis(1));
        let (ready, detail) = match kind {
            "file_exists" => (Path::new(required_str(args, "path")?).exists(), Value::Null),
            "process_exit" => {
                let pid =
                    u32::try_from(required_u64(args, "pid")?).map_err(|_| "'pid' is too large.")?;
                let mut system = System::new();
                system.refresh_processes(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
                (system.process(Pid::from_u32(pid)).is_none(), Value::Null)
            }
            "tcp" => {
                let host = required_str(args, "host")?;
                let port = validate_port(args)?;
                match super::network::connect_tcp(host, port, probe_timeout) {
                    Ok(_) => (true, Value::Null),
                    Err(error) => (false, json!({"error":error})),
                }
            }
            "http" => {
                let expected = optional_u64(args, "expectedStatus", 200, 599)?;
                match super::network::request(
                    &json!({"url":required_str(args,"url")?,"timeoutMs":probe_timeout.as_millis() as u64,"maxBodyBytes":0}),
                ) {
                    Ok(response) => {
                        let status = response["status"].as_u64().unwrap_or(0);
                        (status == expected, json!({"status":status}))
                    }
                    Err(error) => (false, json!({"error":error})),
                }
            }
            _ => unreachable!(),
        };
        if ready || Instant::now() >= deadline {
            return Ok(
                json!({"kind":kind,"ready":ready,"attempts":attempts,"elapsedMs":start.elapsed().as_millis(),"detail":detail}),
            );
        }
        thread::sleep(
            Duration::from_millis(interval_ms)
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn validate_port(args: &Value) -> Result<u16, String> {
    let port = required_u64(args, "port")?;
    if port == 0 || port > 65535 {
        return Err("'port' must be between 1 and 65535.".into());
    }
    Ok(port as u16)
}
