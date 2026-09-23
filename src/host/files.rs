use super::{io_error, optional_bool, optional_str, optional_u64, required_str, required_u64};
use regex::RegexBuilder;
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "file_find" => find(args),
        "text_search" => search(args),
        "file_stat" => stat(args),
        "file_hash" => hash(args),
        "file_read_range" => read_range(args),
        "file_write_atomic" => write_atomic(args),
        "file_patch" => patch(args),
        "file_copy_move" => copy_move(args),
        _ => unreachable!(),
    }
}

fn modified_ms(metadata: &fs::Metadata) -> Option<u128> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
}

fn stat(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let meta = fs::symlink_metadata(path).map_err(|e| io_error("Cannot stat path", e))?;
    Ok(json!({
        "path":path.to_string_lossy(),
        "kind":if meta.file_type().is_symlink() {"symlink"} else if meta.is_dir() {"directory"} else if meta.is_file() {"file"} else {"other"},
        "sizeBytes":meta.len(),
        "readonly":meta.permissions().readonly(),
        "modifiedUnixMs":modified_ms(&meta)
    }))
}

fn wildcard(pattern: &str) -> Result<regex::Regex, String> {
    let mut source = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => source.push_str(".*"),
            '?' => source.push('.'),
            _ => source.push_str(&regex::escape(&ch.to_string())),
        }
    }
    source.push('$');
    RegexBuilder::new(&source)
        .case_insensitive(cfg!(windows))
        .build()
        .map_err(|e| io_error("Invalid wildcard", e))
}

fn find(args: &Value) -> Result<Value, String> {
    let root = Path::new(required_str(args, "root")?);
    if !root.is_dir() {
        return Err("'root' must be an existing directory.".into());
    }
    let pattern = wildcard(optional_str(args, "pattern")?.unwrap_or("*"))?;
    let recursive = optional_bool(args, "recursive", true)?;
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    if limit == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    let mut entries = Vec::new();
    let walker = WalkDir::new(root).max_depth(if recursive { usize::MAX } else { 1 });
    for entry in walker.into_iter().skip(1) {
        let entry = entry.map_err(|e| io_error("Directory traversal failed", e))?;
        if !pattern.is_match(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|e| io_error("Cannot stat entry", e))?;
        entries.push(json!({"path":entry.path().to_string_lossy(),"isDirectory":meta.is_dir(),"sizeBytes":meta.len()}));
        if entries.len() >= limit {
            break;
        }
    }
    Ok(json!({"entries":entries,"limitReached":entries.len() >= limit}))
}

fn search(args: &Value) -> Result<Value, String> {
    let root = Path::new(required_str(args, "root")?);
    if !root.exists() {
        return Err("'root' does not exist.".into());
    }
    let needle = required_str(args, "pattern")?;
    if needle.is_empty() {
        return Err("'pattern' must not be empty.".into());
    }
    let use_regex = optional_bool(args, "regex", false)?;
    let case_sensitive = optional_bool(args, "caseSensitive", true)?;
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    if limit == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    let expression = if use_regex {
        needle.to_owned()
    } else {
        regex::escape(needle)
    };
    let pattern = RegexBuilder::new(&expression)
        .case_insensitive(!case_sensitive)
        .size_limit(1 << 20)
        .build()
        .map_err(|e| io_error("Invalid search pattern", e))?;
    let mut matches = Vec::new();
    let mut skipped = 0usize;
    let walker = WalkDir::new(root);
    for entry in walker {
        let entry = entry.map_err(|e| io_error("Directory traversal failed", e))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let meta = entry
            .metadata()
            .map_err(|e| io_error("Cannot stat entry", e))?;
        if meta.len() > 16 * 1024 * 1024 {
            skipped += 1;
            continue;
        }
        let bytes = fs::read(entry.path()).map_err(|e| io_error("Cannot read file", e))?;
        let Ok(content) = std::str::from_utf8(&bytes) else {
            skipped += 1;
            continue;
        };
        for (line_index, line) in content.lines().enumerate() {
            if let Some(hit) = pattern.find(line) {
                matches.push(json!({"path":entry.path().to_string_lossy(),"line":line_index+1,"column":line[..hit.start()].chars().count()+1,"text":line.chars().take(500).collect::<String>()}));
                if matches.len() >= limit {
                    return Ok(
                        json!({"matches":matches,"limitReached":true,"skippedFiles":skipped}),
                    );
                }
            }
        }
    }
    Ok(json!({"matches":matches,"limitReached":false,"skippedFiles":skipped}))
}

fn digest_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 15) as usize] as char);
    }
    text
}

fn hash(args: &Value) -> Result<Value, String> {
    let path = required_str(args, "path")?;
    let algorithm = optional_str(args, "algorithm")?.unwrap_or("sha256");
    let mut file = File::open(path).map_err(|e| io_error("Cannot open file", e))?;
    let mut buffer = [0u8; 65536];
    let mut count = 0u64;
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    if algorithm != "sha256" && algorithm != "sha512" {
        return Err("'algorithm' must be sha256 or sha512.".into());
    }
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|e| io_error("Cannot read file", e))?;
        if length == 0 {
            break;
        }
        count += length as u64;
        if algorithm == "sha256" {
            sha256.update(&buffer[..length]);
        } else {
            sha512.update(&buffer[..length]);
        }
    }
    let digest = if algorithm == "sha256" {
        digest_hex(&sha256.finalize())
    } else {
        digest_hex(&sha512.finalize())
    };
    Ok(json!({"algorithm":algorithm,"digest":digest,"bytesHashed":count}))
}

fn read_range(args: &Value) -> Result<Value, String> {
    let path = required_str(args, "path")?;
    let offset = required_u64(args, "offset")?;
    let length = required_u64(args, "length")?;
    if length > 1024 * 1024 {
        return Err("'length' must be at most 1 MiB.".into());
    }
    let encoding = optional_str(args, "encoding")?.unwrap_or("utf8");
    let mut file = File::open(path).map_err(|e| io_error("Cannot open file", e))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| io_error("Cannot seek file", e))?;
    let mut data = vec![0u8; length as usize];
    let count = file
        .read(&mut data)
        .map_err(|e| io_error("Cannot read file", e))?;
    data.truncate(count);
    let output = match encoding {
        "utf8" => {
            String::from_utf8(data).map_err(|_| "Selected bytes are not valid UTF-8.".to_owned())?
        }
        "hex" => digest_hex(&data),
        _ => return Err("'encoding' must be utf8 or hex.".into()),
    };
    Ok(json!({"data":output,"bytesRead":count,"encoding":encoding,"offset":offset}))
}

fn persist_bytes(path: &Path, content: &[u8], overwrite: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        return Err("Parent directory does not exist.".into());
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| io_error("Cannot create temporary file", e))?;
    temp.write_all(content)
        .map_err(|e| io_error("Cannot write temporary file", e))?;
    temp.flush()
        .map_err(|e| io_error("Cannot flush temporary file", e))?;
    let persist = if overwrite {
        temp.persist(path)
    } else {
        temp.persist_noclobber(path)
    };
    persist.map_err(|e| io_error("Cannot persist file", e.error))?;
    Ok(())
}

fn write_atomic(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let content = required_str(args, "content")?;
    let overwrite = optional_bool(args, "overwrite", false)?;
    persist_bytes(path, content.as_bytes(), overwrite)?;
    Ok(json!({"path":path.to_string_lossy(),"bytesWritten":content.len()}))
}

fn patch(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let mode = optional_str(args, "mode")?.unwrap_or("text");
    let find = required_str(args, "find")?;
    let replace = required_str(args, "replace")?;
    let (find, replace) = match mode {
        "text" => (find.as_bytes().to_vec(), replace.as_bytes().to_vec()),
        "hex" => (
            crate::bytes::parse_hex(find)?,
            crate::bytes::parse_hex(replace)?,
        ),
        _ => return Err("'mode' must be text or hex.".into()),
    };
    if find.is_empty() {
        return Err("'find' must not be empty.".into());
    }
    let expected_matches = optional_u64(args, "expectedMatches", 1, 10000)? as usize;
    if expected_matches == 0 {
        return Err("'expectedMatches' must be at least 1.".into());
    }
    let metadata = fs::metadata(path).map_err(|e| io_error("Cannot inspect file", e))?;
    if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
        return Err("Patch target must be a file of at most 16 MiB.".into());
    }
    let original = fs::read(path).map_err(|e| io_error("Cannot read file", e))?;
    let original_hash = digest_hex(&Sha256::digest(&original));
    if let Some(expected) = optional_str(args, "expectedSha256")?
        && !original_hash.eq_ignore_ascii_case(expected)
    {
        return Err("File SHA-256 does not match expectedSha256.".into());
    }
    let mut result = Vec::with_capacity(original.len());
    let mut cursor = 0;
    let mut matches = 0;
    while cursor + find.len() <= original.len() {
        if original[cursor..].starts_with(&find) {
            result.extend_from_slice(&replace);
            cursor += find.len();
            matches += 1;
        } else {
            result.push(original[cursor]);
            cursor += 1;
        }
        if result.len() > 16 * 1024 * 1024 {
            return Err("Patched file would exceed 16 MiB.".into());
        }
    }
    result.extend_from_slice(&original[cursor..]);
    if result.len() > 16 * 1024 * 1024 {
        return Err("Patched file would exceed 16 MiB.".into());
    }
    if matches != expected_matches {
        return Err(format!(
            "Expected {expected_matches} matches, found {matches}."
        ));
    }
    persist_bytes(path, &result, true)?;
    Ok(
        json!({"path":path.to_string_lossy(),"matches":matches,"bytesWritten":result.len(),"oldSha256":original_hash,"newSha256":digest_hex(&Sha256::digest(&result))}),
    )
}

fn copy_directory(source: &Path, destination: &Path) -> Result<u64, String> {
    let mut bytes = 0;
    for entry in WalkDir::new(source) {
        let entry = entry.map_err(|e| io_error("Directory traversal failed", e))?;
        let relative = entry
            .path()
            .strip_prefix(source)
            .map_err(|e| e.to_string())?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| io_error("Cannot create destination directory", e))?;
        } else if entry.file_type().is_file() {
            bytes +=
                fs::copy(entry.path(), &target).map_err(|e| io_error("Cannot copy file", e))?;
        } else {
            return Err("Copying symlinks is not supported.".into());
        }
    }
    Ok(bytes)
}

fn copy_move(args: &Value) -> Result<Value, String> {
    let source = PathBuf::from(required_str(args, "source")?);
    let destination = PathBuf::from(required_str(args, "destination")?);
    let operation = required_str(args, "operation")?;
    let overwrite = optional_bool(args, "overwrite", false)?;
    if operation != "copy" && operation != "move" {
        return Err("'operation' must be copy or move.".into());
    }
    if fs::symlink_metadata(&source)
        .map_err(|e| io_error("Cannot inspect source", e))?
        .file_type()
        .is_symlink()
    {
        return Err("Copying or moving symlinks is not supported.".into());
    }
    let source_abs = source
        .canonicalize()
        .map_err(|e| io_error("Cannot resolve source", e))?;
    let destination_parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_abs = destination_parent
        .canonicalize()
        .map_err(|e| io_error("Cannot resolve destination parent", e))?;
    let destination_abs = parent_abs.join(
        destination
            .file_name()
            .ok_or("Destination must name a file or directory.")?,
    );
    if destination_abs == source_abs || destination_abs.starts_with(&source_abs) {
        return Err("Destination must not equal or be inside source.".into());
    }
    let destination_exists = fs::symlink_metadata(&destination).is_ok();
    if destination_exists && !overwrite {
        return Err("Destination already exists.".into());
    }
    if destination_exists && destination.canonicalize().ok().as_deref() == Some(&source_abs) {
        return Err("Source and destination refer to the same path.".into());
    }
    if operation == "move" && !destination_exists && fs::rename(&source, &destination).is_ok() {
        return Ok(
            json!({"source":source.to_string_lossy(),"destination":destination.to_string_lossy(),"operation":operation,"bytesCopied":0}),
        );
    }
    let staging = tempfile::Builder::new()
        .prefix(".everymcp-")
        .tempdir_in(&parent_abs)
        .map_err(|e| io_error("Cannot create staging directory", e))?;
    let payload = staging.path().join("payload");
    let bytes = if source.is_dir() {
        copy_directory(&source, &payload)?
    } else {
        fs::copy(&source, &payload).map_err(|e| io_error("Cannot stage source", e))?
    };
    let backup = staging.path().join("backup");
    if destination_exists {
        fs::rename(&destination, &backup)
            .map_err(|e| io_error("Cannot stage old destination", e))?;
    }
    if let Err(error) = fs::rename(&payload, &destination) {
        if destination_exists && let Err(restore) = fs::rename(&backup, &destination) {
            let recovery = staging.keep();
            return Err(format!(
                "Cannot install replacement: {error}; recovery of old destination failed: {restore}. Old destination retained at '{}'.",
                recovery.display()
            ));
        }
        return Err(io_error("Cannot install destination", error));
    }
    if operation == "move" {
        if source.is_dir() {
            fs::remove_dir_all(&source)
        } else {
            fs::remove_file(&source)
        }
        .map_err(|e| io_error("Destination created, but cannot remove source", e))?;
    }
    Ok(
        json!({"source":source.to_string_lossy(),"destination":destination.to_string_lossy(),"operation":operation,"bytesCopied":bytes}),
    )
}
