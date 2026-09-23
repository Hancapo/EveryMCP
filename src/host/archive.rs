use super::{io_error, optional_bool, required_str, string_array};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "archive_create" => create(args),
        "archive_extract" => extract(args),
        _ => unreachable!(),
    }
}

fn create(args: &Value) -> Result<Value, String> {
    let archive_path = Path::new(required_str(args, "archive")?);
    let paths = string_array(args, "paths", true)?;
    if paths.is_empty() {
        return Err("'paths' must not be empty.".into());
    }
    let overwrite = optional_bool(args, "overwrite", false)?;
    if archive_path.exists() && !overwrite {
        return Err("Archive already exists.".into());
    }
    let mut entries: Vec<(PathBuf, String, bool)> = Vec::new();
    let mut names = HashSet::new();
    let archive_absolute = archive_path.canonicalize().ok();
    for raw in &paths {
        let path = Path::new(raw);
        let basename = path
            .file_name()
            .ok_or("Each source must have a basename.")?
            .to_string_lossy()
            .to_string();
        if path.is_file() {
            if !names.insert(basename.clone()) {
                return Err(format!("Duplicate archive path '{basename}'."));
            }
            entries.push((path.to_owned(), basename, false));
        } else if path.is_dir() {
            for item in WalkDir::new(path) {
                let item = item.map_err(|e| io_error("Cannot traverse source", e))?;
                if item.file_type().is_symlink() {
                    return Err("Archive sources may not contain symlinks.".into());
                }
                if item.file_type().is_file()
                    && archive_absolute.as_deref() == item.path().canonicalize().ok().as_deref()
                {
                    return Err("Archive cannot include itself.".into());
                }
                let relative = item.path().strip_prefix(path).map_err(|e| e.to_string())?;
                let name = if relative.as_os_str().is_empty() {
                    basename.clone()
                } else {
                    format!(
                        "{basename}/{}",
                        relative.to_string_lossy().replace('\\', "/")
                    )
                };
                if !names.insert(name.clone()) {
                    return Err(format!("Duplicate archive path '{name}'."));
                }
                entries.push((item.path().to_owned(), name, item.file_type().is_dir()));
            }
        } else {
            return Err(format!("Source '{raw}' is not a file or directory."));
        }
    }
    if entries.len() > 10000 {
        return Err("Archive supports at most 10000 entries.".into());
    }
    let parent = archive_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| io_error("Cannot create archive temporary file", e))?;
    let mut zip = ZipWriter::new(temp);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut total = 0u64;
    let mut file_count = 0;
    for (path, name, is_dir) in &entries {
        if *is_dir {
            zip.add_directory(format!("{name}/"), options)
                .map_err(|e| io_error("Cannot add archive directory", e))?;
            continue;
        }
        zip.start_file(name, options)
            .map_err(|e| io_error("Cannot add archive entry", e))?;
        let mut source = File::open(path).map_err(|e| io_error("Cannot open source", e))?;
        total += io::copy(&mut source, &mut zip)
            .map_err(|e| io_error("Cannot write archive entry", e))?;
        file_count += 1;
    }
    let temp = zip
        .finish()
        .map_err(|e| io_error("Cannot finish archive", e))?;
    let persisted = if overwrite {
        temp.persist(archive_path)
    } else {
        temp.persist_noclobber(archive_path)
    };
    persisted.map_err(|e| io_error("Cannot persist archive", e.error))?;
    Ok(json!({"archive":archive_path.to_string_lossy(),"fileCount":file_count,"inputBytes":total}))
}

fn safe_target(destination: &Path, relative: &Path) -> Result<PathBuf, String> {
    let mut target = destination.to_owned();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => {
                target.push(part);
                if target.exists()
                    && fs::symlink_metadata(&target)
                        .map_err(|e| io_error("Cannot inspect output", e))?
                        .file_type()
                        .is_symlink()
                {
                    return Err("Archive output crosses a symlink.".into());
                }
            }
            _ => return Err("Archive entry has an unsafe path.".into()),
        }
    }
    Ok(target)
}

fn extract(args: &Value) -> Result<Value, String> {
    let archive_path = Path::new(required_str(args, "archive")?);
    let destination = Path::new(required_str(args, "destination")?);
    let overwrite = optional_bool(args, "overwrite", false)?;
    if destination.exists()
        && fs::symlink_metadata(destination)
            .map_err(|e| io_error("Cannot inspect destination", e))?
            .file_type()
            .is_symlink()
    {
        return Err("Destination may not be a symlink.".into());
    }
    let archive_file = File::open(archive_path).map_err(|e| io_error("Cannot open archive", e))?;
    let mut zip = ZipArchive::new(archive_file).map_err(|e| io_error("Invalid ZIP archive", e))?;
    if zip.len() > 10000 {
        return Err("Archive has more than 10000 entries.".into());
    }
    let mut total = 0u64;
    let mut output_paths = Vec::new();
    let mut seen_paths = HashSet::new();
    for index in 0..zip.len() {
        let entry = zip
            .by_index(index)
            .map_err(|e| io_error("Invalid ZIP entry", e))?;
        let relative = entry
            .enclosed_name()
            .ok_or("Archive entry has an unsafe path.")?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("ZIP symlinks are not supported.".into());
        }
        total = total
            .checked_add(entry.size())
            .ok_or("Archive size overflow.")?;
        if total > 1024 * 1024 * 1024 {
            return Err("Archive expands beyond 1 GiB.".into());
        }
        let output = safe_target(destination, &relative)?;
        if !seen_paths.insert(output.clone()) {
            return Err("Archive contains duplicate output paths.".into());
        }
        if output.exists() && !entry.is_dir() && !overwrite {
            return Err(format!("Output '{}' already exists.", output.display()));
        }
        output_paths.push(output);
    }
    fs::create_dir_all(destination).map_err(|e| io_error("Cannot create destination", e))?;
    let mut count = 0usize;
    for (index, output) in output_paths.iter().enumerate() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| io_error("Invalid ZIP entry", e))?;
        if entry.is_dir() {
            fs::create_dir_all(output)
                .map_err(|e| io_error("Cannot create output directory", e))?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| io_error("Cannot create output directory", e))?;
            }
            let mut file = if overwrite {
                File::create(output)
            } else {
                File::create_new(output)
            }
            .map_err(|e| io_error("Cannot create output file", e))?;
            let copied =
                io::copy(&mut entry, &mut file).map_err(|e| io_error("Cannot extract entry", e))?;
            if copied != entry.size() {
                return Err("Extracted size differs from ZIP metadata.".into());
            }
            count += 1;
        }
    }
    Ok(json!({"destination":destination.to_string_lossy(),"fileCount":count,"outputBytes":total}))
}
