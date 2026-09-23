use super::{io_error, required_str};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

fn architecture(path: &Path) -> Option<&'static str> {
    let mut file = File::open(path).ok()?;
    let mut header = [0u8; 64];
    file.read_exact(&mut header).ok()?;
    if header.starts_with(b"MZ") {
        let pe_offset = u32::from_le_bytes(header[0x3c..0x40].try_into().ok()?) as u64;
        file.seek(SeekFrom::Start(pe_offset)).ok()?;
        let mut pe = [0u8; 6];
        file.read_exact(&mut pe).ok()?;
        if &pe[..4] != b"PE\0\0" {
            return None;
        }
        return match u16::from_le_bytes([pe[4], pe[5]]) {
            0x8664 => Some("x86_64"),
            0x14c => Some("x86"),
            0xaa64 => Some("aarch64"),
            _ => Some("unknown-pe"),
        };
    }
    if header.starts_with(b"\x7fELF") {
        let machine = match header[5] {
            1 => u16::from_le_bytes([header[18], header[19]]),
            2 => u16::from_be_bytes([header[18], header[19]]),
            _ => return Some("unknown-elf"),
        };
        return match machine {
            0x03 => Some("x86"),
            0x3e => Some("x86_64"),
            0x28 => Some("arm"),
            0xb7 => Some("aarch64"),
            0xf3 => Some("riscv"),
            _ => Some("unknown-elf"),
        };
    }
    let magic = &header[..4];
    if magic == [0xcf, 0xfa, 0xed, 0xfe] || magic == [0xce, 0xfa, 0xed, 0xfe] {
        return match u32::from_le_bytes(header[4..8].try_into().ok()?) {
            0x0100_0007 => Some("x86_64"),
            0x0100_000c => Some("aarch64"),
            _ => Some("unknown-mach-o"),
        };
    }
    None
}

fn candidates(name: &str) -> Vec<PathBuf> {
    let path = Path::new(name);
    let has_directory = path.is_absolute() || name.contains('/') || name.contains('\\');
    let dirs = if has_directory {
        vec![PathBuf::new()]
    } else {
        std::env::var_os("PATH")
            .map(|value| std::env::split_paths(&value).collect())
            .unwrap_or_default()
    };
    let suffixes = if cfg!(windows) && path.extension().is_none() {
        let mut values = vec![String::new()];
        values.extend(
            std::env::var("PATHEXT")
                .unwrap_or_else(|_| ".EXE;.COM;.BAT;.CMD".into())
                .split(';')
                .map(str::to_owned),
        );
        values
    } else {
        vec![String::new()]
    };
    dirs.into_iter()
        .flat_map(|dir| {
            suffixes.iter().map(move |suffix| {
                let mut candidate = dir.join(name).into_os_string();
                candidate.push(suffix);
                PathBuf::from(candidate)
            })
        })
        .collect()
}

pub fn executable_resolve(args: &Value) -> Result<Value, String> {
    let name = required_str(args, "name")?;
    if name.is_empty() {
        return Err("'name' must not be empty.".into());
    }
    let path = candidates(name)
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| format!("Executable '{name}' was not found."))?;
    let path = path
        .canonicalize()
        .map_err(|e| io_error("Cannot resolve executable", e))?;
    let metadata = fs::metadata(&path).map_err(|e| io_error("Cannot inspect executable", e))?;
    #[cfg(windows)]
    let version = super::windows::file_version(&path).ok().flatten();
    #[cfg(not(windows))]
    let version: Option<String> = None;
    Ok(
        json!({"name":name,"path":path.to_string_lossy(),"sizeBytes":metadata.len(),"architecture":architecture(&path),"fileVersion":version}),
    )
}
