use super::{
    MAX_COLLECTION_ENTRIES, io_error, optional_bool, optional_u64, required_str, string_array,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "directory_size" => size(args),
        "file_duplicates" => duplicates(args),
        "directory_sync" => sync(args),
        _ => unreachable!(),
    }
}

struct Entry {
    relative: PathBuf,
    directory: bool,
    bytes: u64,
    modified: Option<SystemTime>,
}

fn path_key(path: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        Ok(PathBuf::from(
            path.to_str()
                .ok_or("Paths must be valid Unicode.")?
                .to_lowercase(),
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(path.to_path_buf())
    }
}

fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn checked_path(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err("Directory paths must not be empty.".into());
    }
    let absolute = std::path::absolute(path).map_err(|e| io_error("Cannot resolve path", e))?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) => {
                normalized.push(component);
                continue;
            }
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => continue,
            _ => normalized.push(component),
        }
        match fs::symlink_metadata(&normalized) {
            Ok(metadata) if is_link(&metadata) => {
                return Err(format!(
                    "Symlinks and reparse points are not supported: '{}'.",
                    normalized.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("Cannot inspect path", error)),
        }
    }
    let mut existing = normalized.as_path();
    let mut missing = Vec::new();
    while !existing
        .try_exists()
        .map_err(|e| io_error("Cannot inspect path", e))?
    {
        missing.push(
            existing
                .file_name()
                .ok_or("Cannot resolve directory path.")?,
        );
        existing = existing
            .parent()
            .ok_or("Cannot resolve directory parent.")?;
    }
    let mut resolved = existing
        .canonicalize()
        .map_err(|e| io_error("Cannot resolve path", e))?;
    for name in missing.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}

fn root(args: &Value, key: &str, allow_missing: bool) -> Result<PathBuf, String> {
    let path = checked_path(Path::new(required_str(args, key)?))?;
    if !path.is_dir() && (!allow_missing || path.exists()) {
        return Err(format!(
            "'{key}' must name a directory{}.",
            if allow_missing { "" } else { " that exists" }
        ));
    }
    Ok(path)
}

fn limit(args: &Value, default: u64) -> Result<usize, String> {
    let value = optional_u64(args, "limit", default, 1000)?;
    if value == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    Ok(value as usize)
}

fn exclusions(args: &Value) -> Result<Vec<PathBuf>, String> {
    string_array(args, "exclude", false)?
        .iter()
        .map(|name| {
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
                return Err("'exclude' must contain basenames, not paths.".into());
            }
            path_key(Path::new(name))
        })
        .collect()
}

fn scan(root: &Path, exclude: &[PathBuf]) -> Result<BTreeMap<PathBuf, Entry>, String> {
    let mut entries = BTreeMap::new();
    let mut walker = WalkDir::new(root).follow_links(false).into_iter();
    while let Some(entry) = walker.next() {
        let entry = entry.map_err(|e| io_error("Cannot traverse directory", e))?;
        if entry.depth() == 0 {
            continue;
        }
        if exclude.contains(&path_key(Path::new(entry.file_name()))?) {
            if entry.file_type().is_dir() {
                walker.skip_current_dir();
            }
            continue;
        }
        if entries.len() == MAX_COLLECTION_ENTRIES {
            return Err(format!(
                "Directory scan exceeds {MAX_COLLECTION_ENTRIES} entries."
            ));
        }
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|e| io_error("Cannot inspect entry", e))?;
        if is_link(&metadata) || !(metadata.is_file() || metadata.is_dir()) {
            return Err(format!(
                "Only regular files and directories are supported; symlinks and reparse points are rejected: '{}'.",
                entry.path().display()
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|e| e.to_string())?
            .to_path_buf();
        let key = path_key(&relative)?;
        let data = Entry {
            relative,
            directory: metadata.is_dir(),
            bytes: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
            modified: metadata.modified().ok(),
        };
        if entries.insert(key, data).is_some() {
            return Err("Directory contains ambiguous case-insensitive paths.".into());
        }
    }
    Ok(entries)
}

fn display_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn size(args: &Value) -> Result<Value, String> {
    let root = root(args, "root", false)?;
    let limit = limit(args, 20)?;
    let entries = scan(&root, &exclusions(args)?)?;
    let mut directories = BTreeMap::<PathBuf, u64>::new();
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    for entry in entries.values() {
        if entry.directory {
            directories.entry(entry.relative.clone()).or_default();
            continue;
        }
        total_bytes = total_bytes
            .checked_add(entry.bytes)
            .ok_or("Total byte count overflow.")?;
        files.push((entry.relative.clone(), entry.bytes));
        let mut parent = entry.relative.parent();
        while let Some(path) = parent.filter(|path| !path.as_os_str().is_empty()) {
            *directories.entry(path.to_path_buf()).or_default() += entry.bytes;
            parent = path.parent();
        }
    }
    let file_count = files.len();
    let directory_count = directories.len();
    let mut directories: Vec<_> = directories.into_iter().collect();
    let rank = |left: &(PathBuf, u64), right: &(PathBuf, u64)| {
        right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0))
    };
    files.sort_by(rank);
    directories.sort_by(rank);
    let summarize = |items: Vec<(PathBuf, u64)>| {
        items
            .into_iter()
            .take(limit)
            .map(|(path, bytes)| json!({"path":display_relative(&path),"sizeBytes":bytes}))
            .collect::<Vec<_>>()
    };
    Ok(
        json!({"root":root,"totalBytes":total_bytes,"fileCount":file_count,"directoryCount":directory_count,"largestFiles":summarize(files),"largestDirectories":summarize(directories),"limitReached":file_count>limit || directory_count>limit}),
    )
}

fn file_hash(root: &Path, entry: &Entry) -> Result<String, String> {
    verify_entry(root, entry)?;
    let (hash, bytes) = super::files::hash_file(&root.join(&entry.relative), "sha256")?;
    if bytes != entry.bytes {
        return Err("File changed during directory scan; retry on a stable tree.".into());
    }
    verify_entry(root, entry)?;
    Ok(hash)
}

fn duplicates(args: &Value) -> Result<Value, String> {
    let root = root(args, "root", false)?;
    let limit = limit(args, 100)?;
    let entries = scan(&root, &exclusions(args)?)?;
    let mut sizes = BTreeMap::<u64, Vec<&Entry>>::new();
    for entry in entries.values().filter(|entry| !entry.directory) {
        sizes.entry(entry.bytes).or_default().push(entry);
    }
    let mut duplicate_groups = Vec::new();
    let mut duplicate_bytes = 0u64;
    for (bytes, candidates) in sizes.into_iter().filter(|(_, entries)| entries.len() > 1) {
        let mut hashes = BTreeMap::<String, Vec<&Entry>>::new();
        for entry in candidates {
            hashes
                .entry(file_hash(&root, entry)?)
                .or_default()
                .push(entry);
        }
        for (hash, files) in hashes.into_iter().filter(|(_, files)| files.len() > 1) {
            let wasted = bytes
                .checked_mul((files.len() - 1) as u64)
                .ok_or("Duplicate byte count overflow.")?;
            duplicate_bytes = duplicate_bytes
                .checked_add(wasted)
                .ok_or("Duplicate byte count overflow.")?;
            duplicate_groups.push((wasted, bytes, hash, files));
        }
    }
    duplicate_groups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.2.cmp(&right.2)));
    let group_count = duplicate_groups.len();
    let mut path_budget = 1000usize;
    let mut paths_truncated = false;
    let mut groups = Vec::new();
    for (wasted, bytes, hash, files) in duplicate_groups.into_iter().take(limit) {
        if path_budget == 0 {
            break;
        }
        let paths: Vec<_> = files
            .iter()
            .take(path_budget)
            .map(|entry| display_relative(&entry.relative))
            .collect();
        path_budget -= paths.len();
        let truncated = paths.len() < files.len();
        paths_truncated |= truncated;
        groups.push(json!({"sha256":hash,"sizeBytes":bytes,"fileCount":files.len(),"duplicateBytes":wasted,"paths":paths,"pathsTruncated":truncated}));
    }
    Ok(
        json!({"root":root,"groupCount":group_count,"duplicateBytes":duplicate_bytes,"limitReached":groups.len()<group_count || paths_truncated,"groups":groups}),
    )
}

fn verify_entry(root: &Path, entry: &Entry) -> Result<(), String> {
    let path = root.join(&entry.relative);
    checked_path(&path)?;
    let metadata = fs::symlink_metadata(&path).map_err(|e| io_error("Cannot recheck entry", e))?;
    if is_link(&metadata)
        || metadata.is_dir() != entry.directory
        || (!entry.directory
            && (!metadata.is_file()
                || metadata.len() != entry.bytes
                || metadata.modified().ok() != entry.modified))
    {
        return Err(format!("Entry changed since scan: '{}'.", path.display()));
    }
    Ok(())
}

struct Change {
    action: &'static str,
    relative: PathBuf,
    source: Option<PathBuf>,
    bytes: u64,
}

fn sync(args: &Value) -> Result<Value, String> {
    let source = root(args, "source", false)?;
    let destination = root(args, "destination", true)?;
    let dry_run = optional_bool(args, "dryRun", true)?;
    let delete = optional_bool(args, "delete", false)?;
    let limit = limit(args, 1000)?;
    if source.parent().is_none() || destination.parent().is_none() {
        return Err("Synchronizing filesystem roots is not supported.".into());
    }
    let source_key = path_key(&source)?;
    let destination_key = path_key(&destination)?;
    if source_key.starts_with(&destination_key) || destination_key.starts_with(&source_key) {
        return Err("Source and destination must not overlap.".into());
    }
    let source_entries = scan(&source, &[])?;
    let destination_entries = if destination.exists() {
        scan(&destination, &[])?
    } else {
        BTreeMap::new()
    };
    let mut changes = Vec::new();
    if !destination.exists() {
        changes.push(Change {
            action: "create_directory",
            relative: PathBuf::new(),
            source: None,
            bytes: 0,
        });
    }
    let mut unchanged_files = 0;
    for (key, entry) in &source_entries {
        let existing = destination_entries.get(key);
        if let Some(existing) = existing {
            if existing.directory != entry.directory {
                return Err(format!(
                    "File/directory type conflict at '{}'; resolve it before syncing.",
                    entry.relative.display()
                ));
            }
            if entry.directory {
                continue;
            }
            if entry.bytes == existing.bytes
                && file_hash(&source, entry)? == file_hash(&destination, existing)?
            {
                unchanged_files += 1;
                continue;
            }
        }
        changes.push(Change {
            action: if entry.directory {
                "create_directory"
            } else if existing.is_some() {
                "update_file"
            } else {
                "copy_file"
            },
            relative: existing.unwrap_or(entry).relative.clone(),
            source: Some(entry.relative.clone()),
            bytes: entry.bytes,
        });
    }
    if delete {
        let mut extras: Vec<_> = destination_entries
            .iter()
            .filter(|(key, _)| !source_entries.contains_key(*key))
            .map(|(_, entry)| entry)
            .collect();
        extras.sort_by_key(|entry| std::cmp::Reverse(entry.relative.components().count()));
        for entry in extras {
            changes.push(Change {
                action: if entry.directory {
                    "delete_directory"
                } else {
                    "delete_file"
                },
                relative: entry.relative.clone(),
                source: None,
                bytes: entry.bytes,
            });
        }
    }
    let bytes_to_copy = changes
        .iter()
        .filter(|change| change.action == "copy_file" || change.action == "update_file")
        .try_fold(0u64, |total, change| {
            total
                .checked_add(change.bytes)
                .ok_or("Copy byte count overflow.")
        })?;
    // Plan both trees before writes so traversal errors or type conflicts cannot leave a partial sync.
    // shortcut: trees are not locked; use filesystem snapshots when concurrent writers must be supported.
    if !dry_run {
        for change in &changes {
            checked_path(&source)?;
            checked_path(&destination)?;
            let target = destination.join(&change.relative);
            checked_path(&target)?;
            if let Some(existing) = destination_entries.get(&path_key(&change.relative)?) {
                verify_entry(&destination, existing)?;
            } else if target.exists() {
                return Err(format!(
                    "Destination appeared after scan: '{}'.",
                    target.display()
                ));
            }
            match change.action {
                "create_directory" => {
                    fs::create_dir_all(&target)
                        .map_err(|e| io_error("Cannot create destination directory", e))?;
                }
                "copy_file" | "update_file" => {
                    let relative = change.source.as_ref().unwrap();
                    verify_entry(&source, &source_entries[&path_key(relative)?])?;
                    let mut file = File::open(source.join(relative))
                        .map_err(|e| io_error("Cannot open source", e))?;
                    super::files::persist_reader(
                        &target,
                        &mut file,
                        change.action == "update_file",
                    )?;
                }
                "delete_file" => {
                    fs::remove_file(&target).map_err(|e| io_error("Cannot delete extra file", e))?
                }
                "delete_directory" => fs::remove_dir(&target)
                    .map_err(|e| io_error("Cannot delete extra directory", e))?,
                _ => unreachable!(),
            }
        }
    }
    let copied_files = changes
        .iter()
        .filter(|change| change.action == "copy_file" || change.action == "update_file")
        .count();
    let count = |action| {
        changes
            .iter()
            .filter(|change| change.action == action)
            .count()
    };
    let plan: Vec<_> = changes.iter().take(limit).map(|change| json!({"action":change.action,"path":display_relative(&change.relative),"sizeBytes":change.bytes})).collect();
    Ok(
        json!({"source":source,"destination":destination,"dryRun":dry_run,"delete":delete,"copiedFiles":copied_files,"unchangedFiles":unchanged_files,"createdDirectories":count("create_directory"),"deletedFiles":count("delete_file"),"deletedDirectories":count("delete_directory"),"bytesToCopy":bytes_to_copy,"changeCount":changes.len(),"changes":plan,"limitReached":changes.len()>limit}),
    )
}
