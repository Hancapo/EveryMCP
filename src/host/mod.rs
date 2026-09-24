mod archive;
mod binary_inspect;
mod data;
mod dependencies;
mod file_extra;
mod files;
mod hardware;
mod inspect;
mod locks;
mod manifest;
mod network;
mod performance;
mod process;
mod reports;
mod shell;
mod wait;
mod windows;
mod workspace;

use serde_json::Value;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "process_start" | "process_run" | "process_get" | "process_list" | "process_tree"
        | "process_wait" | "process_output" | "process_input" | "process_stop" | "system_info" => {
            process::execute(name, args)
        }
        "wait_for" => wait::execute(args),
        "http_request" => network::request(args),
        "http_download_file" => network::download_file(args),
        "network_probe" => network::probe(args),
        "dns_query" => network::dns_query(args),
        "executable_resolve" => inspect::executable_resolve(args),
        "performance_sample" => performance::sample(args),
        "file_find" | "text_search" | "file_stat" | "file_hash" | "file_read_range"
        | "file_write_atomic" | "file_copy_move" | "file_patch" | "directory_manifest" => {
            files::execute(name, args)
        }
        "archive_create" | "archive_extract" => archive::execute(name, args),
        "archive_inspect" | "archive_extract_selected" => archive::execute(name, args),
        "hardware_inventory" => hardware::inventory(),
        "file_tail"
        | "file_diff"
        | "file_trash"
        | "binary_pattern_search"
        | "image_inspect"
        | "text_transcode" => file_extra::execute(name, args),
        "pe_inspect" => binary_inspect::pe_inspect(args),
        "file_lock_holders" => locks::holders(args),
        "workspace_scan"
        | "repo_status_batch"
        | "command_pipeline"
        | "environment_snapshot"
        | "file_watch" => workspace::execute(name, args),
        "structured_data_query" | "structured_data_diff" => data::execute(name, args),
        "dependency_inventory" => dependencies::execute(args),
        "artifact_manifest" | "manifest_diff" => manifest::execute(name, args),
        "diagnostics_parse" | "test_results_parse" => reports::execute(name, args),
        "file_open" => shell::open(args),
        "process_modules" | "port_owner" | "service_get" | "service_control" | "eventlog_query"
        | "registry_read" | "environment_get" | "powershell_run" | "file_signature"
        | "scheduled_task" | "eventlog_follow" | "acl_get" => windows::execute(name, args),
        _ => Err(format!("Unknown host tool '{name}'.")),
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("'{key}' must be a string."))
}

fn optional_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("'{key}' must be a string.")),
    }
}

fn required_u64(args: &Value, key: &str) -> Result<u64, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("'{key}' must be a nonnegative integer."))
}

fn optional_u64(args: &Value, key: &str, default: u64, max: u64) -> Result<u64, String> {
    let value = match args.get(key) {
        None => default,
        Some(_) => required_u64(args, key)?,
    };
    if value > max {
        return Err(format!("'{key}' must be at most {max}."));
    }
    Ok(value)
}

fn optional_bool(args: &Value, key: &str, default: bool) -> Result<bool, String> {
    match args.get(key) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("'{key}' must be a boolean.")),
    }
}

fn string_array(args: &Value, key: &str, required: bool) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return if required {
            Err(format!("'{key}' is required."))
        } else {
            Ok(Vec::new())
        };
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("'{key}' must be an array of strings."))?;
    if array.len() > 1024 {
        return Err(format!("'{key}' may contain at most 1024 strings."));
    }
    array
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("'{key}' must contain only strings."))
        })
        .collect()
}

fn io_error(context: &str, error: impl std::fmt::Display) -> String {
    format!("{context}: {error}")
}
