use super::{io_error, optional_u64, required_str};
use serde_json::{Map, Value, json};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

const MAX_METADATA_BYTES: usize = 32 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 64 * 1024;

struct Dump {
    file: File,
    size: u64,
    remaining: usize,
}

impl Dump {
    fn range(&self, rva: u64, length: u64) -> Result<(), String> {
        if rva.checked_add(length).is_none_or(|end| end > self.size) {
            return Err(format!(
                "Minidump range at file offset 0x{rva:x}, length {length}, exceeds file size {}.",
                self.size
            ));
        }
        Ok(())
    }

    fn read(&mut self, rva: u64, length: usize) -> Result<Vec<u8>, String> {
        self.range(rva, length as u64)?;
        self.remaining = self
            .remaining
            .checked_sub(length)
            .ok_or("Minidump metadata exceeds the 32 MiB inspection budget.")?;
        self.file
            .seek(SeekFrom::Start(rva))
            .map_err(|e| io_error("Cannot seek minidump", e))?;
        let mut bytes = vec![0; length];
        self.file
            .read_exact(&mut bytes)
            .map_err(|e| io_error("Cannot read minidump", e))?;
        Ok(bytes)
    }

    fn string(&mut self, rva: u32, decode: bool) -> Result<Option<String>, String> {
        let length = number::<4>(&self.read(rva as u64, 4)?, 0)? as usize;
        if length > MAX_STRING_BYTES || !length.is_multiple_of(2) {
            return Err(
                "Minidump string must have an even UTF-16 byte length of at most 65536.".into(),
            );
        }
        self.range(rva as u64 + 4, length as u64)?;
        if !decode {
            return Ok(None);
        }
        let bytes = self.read(rva as u64 + 4, length)?;
        let words: Vec<_> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| u16::from_le_bytes(*v))
            .collect();
        String::from_utf16(&words)
            .map(Some)
            .map_err(|e| io_error("Invalid minidump UTF-16 string", e))
    }

    fn descriptor(&self, bytes: &[u8], offset: usize) -> Result<(u64, usize), String> {
        let length = number::<4>(bytes, offset)? as usize;
        let rva = number::<4>(bytes, offset + 4)?;
        self.range(rva, length as u64)?;
        Ok((rva, length))
    }

    fn context(
        &mut self,
        bytes: &[u8],
        offset: usize,
        architecture: Option<u16>,
    ) -> Result<Value, String> {
        let (rva, length) = self.descriptor(bytes, offset)?;
        if length == 0 {
            return Ok(Value::Null);
        }
        if length > 64 * 1024 {
            return Err("Minidump context exceeds 64 KiB.".into());
        }
        let Some(architecture @ (0 | 9 | 12)) = architecture else {
            return Ok(json!({"sizeBytes":length,"supported":false,"architectureId":architecture}));
        };
        // Only the prefix containing scalar registers is read; extended state can be much larger.
        let bytes = self.read(rva, length.min(272))?;
        let flags = number::<4>(&bytes, if architecture == 9 { 48 } else { 0 })? as u32;
        let expected_arch = match architecture {
            0 => 0x10000,
            9 => 0x100000,
            _ => 0x400000,
        };
        if flags & 0x00ff0000 != expected_arch {
            return Err("Minidump context flags do not match SystemInfo architecture.".into());
        }
        let mut registers = Map::new();
        let mut add = |name: &str, offset: usize, width: usize| -> Result<(), String> {
            let value = if width == 8 {
                number::<8>(&bytes, offset)?
            } else {
                number::<4>(&bytes, offset)?
            };
            registers.insert(name.into(), json!(format!("0x{value:x}")));
            Ok(())
        };
        match architecture {
            9 => {
                if flags & 1 != 0 {
                    for (name, at, width) in [("rsp", 152, 8), ("rip", 248, 8), ("eflags", 68, 4)] {
                        add(name, at, width)?;
                    }
                }
                if flags & 2 != 0 {
                    for (name, at) in [
                        ("rax", 120),
                        ("rcx", 128),
                        ("rdx", 136),
                        ("rbx", 144),
                        ("rbp", 160),
                        ("rsi", 168),
                        ("rdi", 176),
                        ("r8", 184),
                        ("r9", 192),
                        ("r10", 200),
                        ("r11", 208),
                        ("r12", 216),
                        ("r13", 224),
                        ("r14", 232),
                        ("r15", 240),
                    ] {
                        add(name, at, 8)?;
                    }
                }
            }
            0 => {
                if flags & 1 != 0 {
                    for (name, at) in [("ebp", 180), ("eip", 184), ("eflags", 192), ("esp", 196)] {
                        add(name, at, 4)?;
                    }
                }
                if flags & 2 != 0 {
                    for (name, at) in [
                        ("edi", 156),
                        ("esi", 160),
                        ("ebx", 164),
                        ("edx", 168),
                        ("ecx", 172),
                        ("eax", 176),
                    ] {
                        add(name, at, 4)?;
                    }
                }
            }
            12 => {
                if flags & 1 != 0 {
                    for (name, at, width) in [
                        ("cpsr", 4, 4),
                        ("fp", 240, 8),
                        ("lr", 248, 8),
                        ("sp", 256, 8),
                        ("pc", 264, 8),
                    ] {
                        add(name, at, width)?;
                    }
                }
                if flags & 2 != 0 {
                    for index in 0..29 {
                        add(&format!("x{index}"), 8 + index * 8, 8)?;
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(
            json!({"sizeBytes":length,"architecture":architecture_name(architecture),"flags":format!("0x{flags:x}"),"supported":true,"registers":registers}),
        )
    }

    fn records(
        &mut self,
        rva: u64,
        size: usize,
        record_size: usize,
    ) -> Result<(usize, Vec<u8>), String> {
        if size < 4 {
            return Err("Truncated minidump list header.".into());
        }
        let count = number::<4>(&self.read(rva, 4)?, 0)? as usize;
        if count > super::MAX_COLLECTION_ENTRIES {
            return Err("Minidump list exceeds 100000 entries.".into());
        }
        let length = count
            .checked_mul(record_size)
            .and_then(|n| n.checked_add(4))
            .ok_or("Minidump list size overflow.")?;
        if length > size {
            return Err("Minidump list count exceeds its stream size.".into());
        }
        Ok((count, self.read(rva + 4, length - 4)?))
    }
}

fn number<const N: usize>(bytes: &[u8], at: usize) -> Result<u64, String> {
    let end = at.checked_add(N).ok_or("Minidump field offset overflow.")?;
    let slice = bytes
        .get(at..end)
        .ok_or("Truncated minidump structure or context.")?;
    let mut value = [0; 8];
    value[..N].copy_from_slice(slice);
    Ok(u64::from_le_bytes(value))
}

fn architecture_name(value: u16) -> &'static str {
    match value {
        0 => "x86",
        5 => "arm",
        6 => "ia64",
        9 => "amd64",
        12 => "arm64",
        _ => "unknown",
    }
}

fn stream_name(value: u32) -> &'static str {
    match value {
        0 => "UnusedStream",
        3 => "ThreadListStream",
        4 => "ModuleListStream",
        5 => "MemoryListStream",
        6 => "ExceptionStream",
        7 => "SystemInfoStream",
        8 => "ThreadExListStream",
        9 => "Memory64ListStream",
        10 => "CommentStreamA",
        11 => "CommentStreamW",
        12 => "HandleDataStream",
        13 => "FunctionTableStream",
        14 => "UnloadedModuleListStream",
        15 => "MiscInfoStream",
        16 => "MemoryInfoListStream",
        17 => "ThreadInfoListStream",
        18 => "HandleOperationListStream",
        19 => "TokenStream",
        24 => "ThreadNamesStream",
        _ => "UnknownStream",
    }
}

pub fn inspect(args: &Value) -> Result<Value, String> {
    let path = required_str(args, "path")?;
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    if limit == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    let file = File::open(path).map_err(|e| io_error("Cannot open minidump", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| io_error("Cannot inspect minidump file", e))?;
    if !metadata.is_file() {
        return Err("'path' must be a regular file.".into());
    }
    let mut dump = Dump {
        file,
        size: metadata.len(),
        remaining: MAX_METADATA_BYTES,
    };
    let header = dump.read(0, 32)?;
    if &header[..4] != b"MDMP" {
        return Err("Not a little-endian minidump (MDMP signature required).".into());
    }
    let version = number::<4>(&header, 4)?;
    if version & 0xffff != 0xa793 {
        return Err("Unsupported minidump version.".into());
    }
    let stream_count = number::<4>(&header, 8)? as usize;
    if stream_count > 4096 {
        return Err("Minidump has more than 4096 streams.".into());
    }
    let directory = dump.read(number::<4>(&header, 12)?, stream_count * 12)?;
    let mut streams = Vec::new();
    let mut supported = std::collections::BTreeMap::new();
    for (index, entry) in directory.as_chunks::<12>().0.iter().enumerate() {
        let kind = number::<4>(entry, 0)? as u32;
        let (rva, size) = dump.descriptor(entry, 4)?;
        if matches!(kind, 3 | 4 | 6 | 7) && supported.insert(kind, (rva, size)).is_some() {
            return Err(format!("Duplicate minidump {}.", stream_name(kind)));
        }
        if index < limit {
            streams.push(
                json!({"type":kind,"name":stream_name(kind),"fileOffset":rva,"sizeBytes":size}),
            );
        }
    }
    let mut result = json!({"path":path,"fileSize":dump.size,"version":format!("0x{version:x}"),
        "flags":format!("0x{:x}",number::<8>(&header,24)?),"timeDateStamp":number::<4>(&header,20)?,
        "streamCount":stream_count,"streams":streams,"streamsTruncated":stream_count>limit,
        "systemInfo":null,"threadCount":null,"threads":[],"threadsTruncated":false,
        "moduleCount":null,"modules":[],"modulesTruncated":false,"exception":null});
    let mut architecture = None;
    if let Some(&(rva, size)) = supported.get(&7) {
        if size < 56 {
            return Err("Truncated minidump SystemInfoStream.".into());
        }
        let bytes = dump.read(rva, 56)?;
        let arch = number::<2>(&bytes, 0)? as u16;
        architecture = Some(arch);
        let csd = number::<4>(&bytes, 24)? as u32;
        result["systemInfo"] = json!({"architectureId":arch,"architecture":architecture_name(arch),
            "processorCount":bytes[6],"majorVersion":number::<4>(&bytes,8)?,"minorVersion":number::<4>(&bytes,12)?,
            "buildNumber":number::<4>(&bytes,16)?,"platformId":number::<4>(&bytes,20)?,
            "servicePack":if csd==0 {None} else {dump.string(csd,true)?}});
    }
    if let Some(&(rva, size)) = supported.get(&3) {
        let (count, bytes) = dump.records(rva, size, 48)?;
        let mut threads = Vec::new();
        for (index, entry) in bytes.as_chunks::<48>().0.iter().enumerate() {
            dump.descriptor(entry, 32)?;
            dump.descriptor(entry, 40)?;
            if index < limit {
                threads.push(json!({"threadId":number::<4>(entry,0)?,"suspendCount":number::<4>(entry,4)?,
                    "priorityClass":number::<4>(entry,8)?,"priority":number::<4>(entry,12)?,
                    "teb":format!("0x{:x}",number::<8>(entry,16)?),
                    "stackBase":format!("0x{:x}",number::<8>(entry,24)?),"stackSizeBytes":number::<4>(entry,32)?,
                    "context":dump.context(entry,40,architecture)?}));
            }
        }
        result["threadCount"] = json!(count);
        result["threads"] = json!(threads);
        result["threadsTruncated"] = json!(count > limit);
    }
    if let Some(&(rva, size)) = supported.get(&4) {
        let (count, bytes) = dump.records(rva, size, 108)?;
        let mut modules = Vec::new();
        for (index, entry) in bytes.as_chunks::<108>().0.iter().enumerate() {
            dump.descriptor(entry, 76)?;
            dump.descriptor(entry, 84)?;
            let name = dump.string(number::<4>(entry, 20)? as u32, index < limit)?;
            let base = number::<8>(entry, 0)?;
            let image_size = number::<4>(entry, 8)?;
            base.checked_add(image_size)
                .ok_or("Minidump module address range overflow.")?;
            if index < limit {
                let high = number::<4>(entry, 32)?;
                let low = number::<4>(entry, 36)?;
                modules.push(json!({"name":name,"baseAddress":format!("0x{base:x}"),"sizeBytes":image_size,
                    "checksum":format!("0x{:x}",number::<4>(entry,12)?),"timeDateStamp":number::<4>(entry,16)?,
                    "fileVersion":if number::<4>(entry,24)?==0xfeef04bd {Some(format!("{}.{}.{}.{}",high>>16,high&0xffff,low>>16,low&0xffff))} else {None}}));
            }
        }
        result["moduleCount"] = json!(count);
        result["modules"] = json!(modules);
        result["modulesTruncated"] = json!(count > limit);
    }
    if let Some(&(rva, size)) = supported.get(&6) {
        if size < 168 {
            return Err("Truncated minidump ExceptionStream.".into());
        }
        let bytes = dump.read(rva, 168)?;
        let count = number::<4>(&bytes, 32)? as usize;
        if count > 15 {
            return Err("Minidump exception has more than 15 parameters.".into());
        }
        let parameters: Result<Vec<_>, String> = (0..count)
            .map(|i| Ok(format!("0x{:x}", number::<8>(&bytes, 40 + i * 8)?)))
            .collect();
        result["exception"] = json!({"threadId":number::<4>(&bytes,0)?,
            "code":format!("0x{:x}",number::<4>(&bytes,8)?),"flags":format!("0x{:x}",number::<4>(&bytes,12)?),
            "recordAddress":format!("0x{:x}",number::<8>(&bytes,16)?),"address":format!("0x{:x}",number::<8>(&bytes,24)?),
            "parameters":parameters?,"context":dump.context(&bytes,160,architecture)?});
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn scalar_context_layouts_preserve_x86_and_arm64_registers() {
        for (architecture, size, flags, at, value, register) in [
            (0, 204, 0x10003u32, 184, 0x76543210u64, "eip"),
            (12, 272, 0x400003, 264, 0x123456789abcdef0, "pc"),
        ] {
            let mut bytes = vec![0; size];
            bytes[..4].copy_from_slice(&flags.to_le_bytes());
            let width = if architecture == 0 { 4 } else { 8 };
            bytes[at..at + width].copy_from_slice(&value.to_le_bytes()[..width]);
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(&bytes).unwrap();
            let mut dump = Dump {
                file,
                size: size as u64,
                remaining: MAX_METADATA_BYTES,
            };
            let mut descriptor = [0; 8];
            descriptor[..4].copy_from_slice(&(size as u32).to_le_bytes());
            let result = dump.context(&descriptor, 0, Some(architecture)).unwrap();
            assert_eq!(result["registers"][register], format!("0x{value:x}"));
        }
    }

    #[test]
    fn malformed_ranges_and_truncated_contexts_return_errors() {
        assert!(number::<8>(&[0; 7], 0).is_err());
        assert!(number::<4>(&[0; 8], usize::MAX).is_err());
        let mut file = tempfile::tempfile().unwrap();
        let mut bytes = [0; 52];
        bytes[48..52].copy_from_slice(&0x100001u32.to_le_bytes());
        file.write_all(&bytes).unwrap();
        let mut dump = Dump {
            file,
            size: 52,
            remaining: MAX_METADATA_BYTES,
        };
        assert!(dump.range(u64::MAX, 2).is_err());
        assert!(
            dump.context(&[52, 0, 0, 0, 0, 0, 0, 0], 0, Some(9))
                .is_err()
        );
    }
}
