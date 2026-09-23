use super::{data, files, optional_bool, optional_str, optional_u64, required_str, workspace};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "artifact_manifest" => create(args),
        "manifest_diff" => diff(args),
        _ => Err(format!("Unknown artifact tool '{name}'.")),
    }
}

fn digest(entries: &Value) -> Result<String, String> {
    let encoded = serde_json::to_vec(&json!({"formatVersion":1,"entries":entries}))
        .map_err(|e| format!("Cannot encode manifest: {e}"))?;
    Ok(files::digest_hex(&Sha256::digest(&encoded)))
}

fn snapshot(root: &Path, excluded: Option<&Path>, limit: u64) -> Result<Value, String> {
    let source = files::manifest(&json!({"root":root,"hashFiles":true,"limit":limit}))?;
    if source["limitReached"] == true {
        return Err(format!(
            "Directory exceeds manifest limit of {limit} entries."
        ));
    }
    let excluded = excluded
        .and_then(|path| path.strip_prefix(root).ok())
        .map(|path| path.to_string_lossy().replace('\\', "/"));
    let mut entries = Vec::new();
    let mut file_count = 0;
    let mut total_bytes = 0u64;
    for entry in source["entries"]
        .as_array()
        .ok_or("Directory manifest omitted entries.")?
    {
        let relative = entry["path"]
            .as_str()
            .ok_or("Directory manifest entry omitted path.")?;
        if excluded.as_deref() == Some(relative) {
            continue;
        }
        let kind = entry["kind"]
            .as_str()
            .ok_or("Directory manifest entry omitted kind.")?;
        let size = entry["sizeBytes"]
            .as_u64()
            .ok_or("Directory manifest entry omitted size.")?;
        if kind == "file" {
            file_count += 1;
            total_bytes += size;
        }
        entries
            .push(json!({"path":relative,"kind":kind,"sizeBytes":size,"sha256":entry["sha256"]}));
    }
    let digest = digest(&json!(entries))?;
    Ok(
        json!({"formatVersion":1,"root":root,"entries":entries,"fileCount":file_count,"totalBytes":total_bytes,"manifestSha256":digest}),
    )
}

fn create(args: &Value) -> Result<Value, String> {
    let root = workspace::directory(args)?;
    let output_path = optional_str(args, "outputPath")?
        .map(std::path::PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                Ok(path)
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(path))
                    .map_err(|e| format!("Cannot resolve output path: {e}"))
            }
        })
        .transpose()?;
    let include_entries = optional_bool(args, "includeEntries", false)?;
    let overwrite = optional_bool(args, "overwrite", false)?;
    let limit = optional_u64(args, "limit", 10000, 10000)?;
    if limit == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    let manifest = snapshot(&root, output_path.as_deref(), limit)?;
    if let Some(output) = output_path.as_deref() {
        let content = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("Cannot encode manifest: {e}"))?;
        files::execute(
            "file_write_atomic",
            &json!({"path":output,"content":content,"overwrite":overwrite}),
        )?;
    }
    let mut response = manifest;
    if !include_entries {
        response.as_object_mut().unwrap().remove("entries");
    }
    response["outputPath"] = json!(output_path);
    Ok(response)
}

fn loaded(path: &Path) -> Result<Value, String> {
    if path.is_dir() {
        return snapshot(path, None, 10000);
    }
    let (_, value) = data::load(path, Some("json"))?;
    if value["formatVersion"] != 1 || !value["entries"].is_array() {
        return Err("Manifest file must contain formatVersion 1 and an entries array.".into());
    }
    let expected = value["manifestSha256"]
        .as_str()
        .ok_or("Manifest file omitted manifestSha256.")?;
    if digest(&value["entries"])? != expected {
        return Err("Manifest integrity hash does not match its entries.".into());
    }
    Ok(value)
}

fn indexed(snapshot: &Value) -> Result<BTreeMap<String, Value>, String> {
    let mut map = BTreeMap::new();
    for entry in snapshot["entries"]
        .as_array()
        .ok_or("Manifest omitted entries.")?
    {
        let path = entry["path"]
            .as_str()
            .ok_or("Manifest entry omitted path.")?;
        if entry["kind"].as_str().is_none() || entry["sizeBytes"].as_u64().is_none() {
            return Err("Manifest entry omitted kind or sizeBytes.".into());
        }
        if map.insert(path.to_owned(), entry.clone()).is_some() {
            return Err(format!("Duplicate manifest path '{path}'."));
        }
    }
    Ok(map)
}

fn diff(args: &Value) -> Result<Value, String> {
    let left_path = Path::new(required_str(args, "left")?);
    let right_path = Path::new(required_str(args, "right")?);
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    let left = indexed(&loaded(left_path)?)?;
    let right = indexed(&loaded(right_path)?)?;
    let mut changes = Vec::new();
    let mut total = 0;
    for (path, before) in &left {
        let change = match right.get(path) {
            None => Some(json!({"path":path,"kind":"removed","before":before})),
            Some(after)
                if before["kind"] != after["kind"]
                    || before["sizeBytes"] != after["sizeBytes"]
                    || before["sha256"] != after["sha256"] =>
            {
                Some(json!({"path":path,"kind":"modified","before":before,"after":after}))
            }
            _ => None,
        };
        if let Some(change) = change {
            total += 1;
            if changes.len() < limit {
                changes.push(change);
            }
        }
    }
    for (path, after) in &right {
        if !left.contains_key(path) {
            total += 1;
            if changes.len() < limit {
                changes.push(json!({"path":path,"kind":"added","after":after}));
            }
        }
    }
    changes.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    Ok(json!({"equal":total==0,"totalChanges":total,"limitReached":total>limit,"changes":changes}))
}
