use super::optional_u64;
use serde_json::{Value, json};
use std::thread;
use std::time::{Duration, Instant};
use sysinfo::{Disks, Networks, System};

pub fn sample(args: &Value) -> Result<Value, String> {
    let sample_ms = optional_u64(args, "sampleMs", 500, 10000)?;
    if sample_ms < 200 {
        return Err("'sampleMs' must be at least 200.".into());
    }
    let mut system = System::new_all();
    system.refresh_cpu_usage();
    system.refresh_memory();
    let mut networks = Networks::new_with_refreshed_list();
    let start = Instant::now();
    thread::sleep(Duration::from_millis(sample_ms));
    system.refresh_cpu_usage();
    system.refresh_memory();
    networks.refresh(true);
    let elapsed = start.elapsed().as_secs_f64();
    let mut network_values: Vec<Value> = networks
        .iter()
        .map(|(name, data)| {
            json!({
                "name":name,
                "receivedBytes":data.received(),
                "transmittedBytes":data.transmitted(),
                "receivedBytesPerSecond":data.received() as f64 / elapsed,
                "transmittedBytesPerSecond":data.transmitted() as f64 / elapsed
            })
        })
        .collect();
    network_values.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    let disks = Disks::new_with_refreshed_list();
    let disk_values: Vec<Value> = disks
        .list()
        .iter()
        .map(|disk| {
            json!({
                "name":disk.name().to_string_lossy(),
                "mountPoint":disk.mount_point().to_string_lossy(),
                "totalBytes":disk.total_space(),
                "availableBytes":disk.available_space()
            })
        })
        .collect();
    Ok(json!({
        "sampleMs":start.elapsed().as_millis(),
        "cpuPercent":system.global_cpu_usage(),
        "totalMemoryBytes":system.total_memory(),
        "availableMemoryBytes":system.available_memory(),
        "networks":network_values,
        "disks":disk_values
    }))
}
