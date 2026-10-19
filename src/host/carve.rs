use super::{io_error, optional_bool, optional_u64, required_str, string_array};
use flate2::bufread::{GzDecoder, ZlibDecoder};
use serde_json::{Value, json};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const CHUNK: usize = 8 * 1024 * 1024;
const OVERLAP: usize = 8;
const MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_INFLATE_BYTES: u64 = 64 * 1024 * 1024;
const ELF_TYPES: [&str; 5] = ["none", "rel", "exec", "dyn", "core"];
const TYPES: [&str; 7] = ["pe", "elf", "zip", "png", "dds", "gzip", "zlib"];

struct Found {
    offset: u64,
    kind: &'static str,
    size: Option<u64>,
    details: Value,
}

/// Bounded random access to the scanned file for header validation.
struct Source {
    file: File,
    len: u64,
}

impl Source {
    fn read(&mut self, offset: u64, length: usize) -> Option<Vec<u8>> {
        let end = offset.checked_add(length as u64)?;
        if end > self.len {
            return None;
        }
        let mut buffer = vec![0; length];
        self.file.seek(SeekFrom::Start(offset)).ok()?;
        self.file.read_exact(&mut buffer).ok()?;
        Some(buffer)
    }
}

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn u32be(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}

fn pe(src: &mut Source, at: u64) -> Option<Found> {
    let dos = src.read(at, 64)?;
    let lfanew = u32le(&dos, 0x3c) as u64;
    if !(0x40..=0x10000).contains(&lfanew) {
        return None;
    }
    let nt = src.read(at + lfanew, 24)?;
    if &nt[..4] != b"PE\0\0" {
        return None;
    }
    let machine = u16le(&nt, 4);
    let sections = u16le(&nt, 6) as usize;
    let optional_size = u16le(&nt, 20) as usize;
    let characteristics = u16le(&nt, 22);
    if sections == 0 || sections > 96 || optional_size < 2 {
        return None;
    }
    let optional = src.read(at + lfanew + 24, optional_size)?;
    let bits = match u16le(&optional, 0) {
        0x10b => 32,
        0x20b => 64,
        _ => return None,
    };
    let table = src.read(at + lfanew + 24 + optional_size as u64, sections * 40)?;
    let mut end = u64::from(if optional.len() >= 64 {
        u32le(&optional, 60)
    } else {
        0
    });
    for s in 0..sections {
        let raw_size = u32le(&table, s * 40 + 16) as u64;
        let raw_pointer = u32le(&table, s * 40 + 20) as u64;
        if raw_size > 0 {
            end = end.max(raw_pointer + raw_size);
        }
    }
    // The certificate table is addressed by file offset and sits after the sections.
    let security = if bits == 64 { 144 } else { 128 };
    if optional.len() >= security + 8 {
        let (offset, size) = (
            u32le(&optional, security) as u64,
            u32le(&optional, security + 4) as u64,
        );
        if offset > 0 && size > 0 {
            end = end.max(offset + size);
        }
    }
    let machine_name = match machine {
        0x14c => "i386",
        0x8664 => "amd64",
        0xaa64 => "arm64",
        0x1c0 | 0x1c4 => "arm",
        _ => "other",
    };
    Some(Found {
        offset: at,
        kind: "pe",
        size: Some(end).filter(|&e| at + e <= src.len),
        details: json!({"machine":machine_name,"machineId":format!("0x{machine:x}"),"bits":bits,
            "dll":characteristics & 0x2000 != 0,"sections":sections}),
    })
}

fn elf(src: &mut Source, at: u64) -> Option<Found> {
    let ident = src.read(at, 16)?;
    let (class, data, version) = (ident[4], ident[5], ident[6]);
    if !matches!(class, 1 | 2) || !matches!(data, 1 | 2) || version != 1 {
        return None;
    }
    let little = data == 1;
    let header = src.read(at, if class == 2 { 64 } else { 52 })?;
    let r16 = |b: &[u8], o: usize| {
        let v = [b[o], b[o + 1]];
        if little {
            u16::from_le_bytes(v)
        } else {
            u16::from_be_bytes(v)
        }
    };
    let r32 = |b: &[u8], o: usize| {
        let v = b[o..o + 4].try_into().unwrap();
        if little {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }
    };
    let r64 = |b: &[u8], o: usize| {
        let v = b[o..o + 8].try_into().unwrap();
        if little {
            u64::from_le_bytes(v)
        } else {
            u64::from_be_bytes(v)
        }
    };
    let (kind, machine) = (r16(&header, 16), r16(&header, 18));
    if r32(&header, 20) != 1 || kind > 4 {
        return None;
    }
    let (phoff, shoff, phentsize, phnum, shentsize, shnum) = if class == 2 {
        (
            r64(&header, 32),
            r64(&header, 40),
            r16(&header, 54),
            r16(&header, 56),
            r16(&header, 58),
            r16(&header, 60),
        )
    } else {
        (
            u64::from(r32(&header, 28)),
            u64::from(r32(&header, 32)),
            r16(&header, 42),
            r16(&header, 44),
            r16(&header, 46),
            r16(&header, 48),
        )
    };
    let mut end = header.len() as u64;
    if shnum > 0 {
        end = end.max(shoff.checked_add(u64::from(shnum) * u64::from(shentsize))?);
    }
    if phnum > 0 && phnum <= 512 && phentsize >= if class == 2 { 56 } else { 32 } {
        let table = src.read(at + phoff, phnum as usize * phentsize as usize)?;
        for p in 0..phnum as usize {
            let base = p * phentsize as usize;
            let (offset, size) = if class == 2 {
                (r64(&table, base + 8), r64(&table, base + 32))
            } else {
                (
                    u64::from(r32(&table, base + 4)),
                    u64::from(r32(&table, base + 16)),
                )
            };
            end = end.max(offset.saturating_add(size));
        }
    }
    Some(Found {
        offset: at,
        kind: "elf",
        size: Some(end).filter(|&e| at + e <= src.len),
        details: json!({"bits":if class == 2 {64} else {32},"endian":if little {"little"} else {"big"},
            "type":ELF_TYPES[kind as usize],"machineId":machine}),
    })
}

/// ZIP archives are found from their end-of-central-directory record and walked back
/// to the first local header, so the archive's start offset is reported.
fn zip(src: &mut Source, eocd: u64) -> Option<Found> {
    let record = src.read(eocd, 22)?;
    let entries = u16le(&record, 10);
    let (cd_size, cd_offset) = (u32le(&record, 12) as u64, u32le(&record, 16) as u64);
    let comment = u16le(&record, 20) as u64;
    if u16le(&record, 4) != 0 || u16le(&record, 6) != 0 || eocd + 22 + comment > src.len {
        return None;
    }
    let start = eocd.checked_sub(cd_size)?.checked_sub(cd_offset)?;
    if entries > 0 {
        let first = src.read(start, 4)?;
        let central = src.read(eocd - cd_size, 4)?;
        if &first != b"PK\x03\x04" || &central != b"PK\x01\x02" {
            return None;
        }
    } else if cd_size != 0 {
        return None;
    }
    Some(Found {
        offset: start,
        kind: "zip",
        size: Some(eocd + 22 + comment - start),
        details: json!({"entries":entries}),
    })
}

fn png(src: &mut Source, at: u64) -> Option<Found> {
    let header = src.read(at + 8, 25)?;
    if u32be(&header, 0) != 13 || &header[4..8] != b"IHDR" {
        return None;
    }
    let (width, height) = (u32be(&header, 8), u32be(&header, 12));
    let mut position = at + 8;
    for _ in 0..100_000 {
        let chunk = src.read(position, 8)?;
        let length = u32be(&chunk, 0) as u64;
        if length > 0x7fff_ffff || !chunk[4..8].iter().all(u8::is_ascii_alphabetic) {
            return None;
        }
        position += 12 + length;
        if &chunk[4..8] == b"IEND" {
            return (position <= src.len).then(|| Found {
                offset: at,
                kind: "png",
                size: Some(position - at),
                details: json!({"width":width,"height":height,"bitDepth":header[16],"colorType":header[17]}),
            });
        }
    }
    None
}

fn dds(src: &mut Source, at: u64) -> Option<Found> {
    let header = src.read(at, 128)?;
    if u32le(&header, 4) != 124 || u32le(&header, 76) != 32 {
        return None;
    }
    let (height, width) = (u32le(&header, 12), u32le(&header, 16));
    let mips = u32le(&header, 28).max(1);
    let format_flags = u32le(&header, 80);
    let four_cc = &header[84..88];
    let bit_count = u32le(&header, 88);
    if width == 0 || height == 0 || width > 65536 || height > 65536 || mips > 32 {
        return None;
    }
    let fourcc = (format_flags & 4 != 0).then(|| String::from_utf8_lossy(four_cc).into_owned());
    // Payload size for common block-compressed and uncompressed 2D textures.
    let block = match four_cc {
        b"DXT1" | b"ATI1" | b"BC4U" | b"BC4S" => Some(8),
        b"DXT2" | b"DXT3" | b"DXT4" | b"DXT5" | b"ATI2" | b"BC5U" | b"BC5S" => Some(16),
        _ => None,
    };
    let payload = (0..mips).try_fold(0u64, |total, level| {
        let (w, h) = (
            (width >> level).max(1) as u64,
            (height >> level).max(1) as u64,
        );
        let bytes = if format_flags & 4 != 0 {
            block? * w.div_ceil(4) * h.div_ceil(4)
        } else if format_flags & 0x40 != 0 && bit_count.is_multiple_of(8) && bit_count > 0 {
            w * h * u64::from(bit_count / 8)
        } else {
            return None;
        };
        Some(total + bytes)
    });
    let size = payload.map(|p| p + 128).filter(|&s| at + s <= src.len);
    Some(Found {
        offset: at,
        kind: "dds",
        size,
        details: json!({"width":width,"height":height,"mipCount":mips,"fourCC":fourcc,
            "rgbBitCount":(format_flags & 0x40 != 0).then_some(bit_count),
            "sizeKnown":size.is_some()}),
    })
}

/// Decompresses a gzip member or zlib stream in place. A stream counts only when it
/// ends cleanly with a valid checksum, which rules out chance header matches.
fn inflate(src: &mut Source, at: u64, gzip: bool) -> Option<Found> {
    let mut file = src.file.try_clone().ok()?;
    file.seek(SeekFrom::Start(at)).ok()?;
    let reader = BufReader::with_capacity(64 * 1024, file);
    let mut sink = [0u8; 64 * 1024];
    let mut output = 0u64;
    macro_rules! drain {
        ($decoder:expr) => {{
            let mut decoder = $decoder;
            loop {
                match decoder.read(&mut sink) {
                    Ok(0) => break,
                    Ok(n) => {
                        output += n as u64;
                        if output > MAX_INFLATE_BYTES {
                            return Some(Found {
                                offset: at,
                                kind: if gzip { "gzip" } else { "zlib" },
                                size: None,
                                details: json!({"decompressedBytes":null,"note":"Output exceeds 64 MiB; size not measured."}),
                            });
                        }
                    }
                    Err(_) => return None,
                }
            }
            decoder.into_inner().stream_position().ok()?
        }};
    }
    let end = if gzip {
        drain!(GzDecoder::new(reader))
    } else {
        drain!(ZlibDecoder::new(reader))
    };
    if output == 0 {
        return None;
    }
    Some(Found {
        offset: at,
        kind: if gzip { "gzip" } else { "zlib" },
        size: Some(end - at),
        details: json!({"decompressedBytes":output}),
    })
}

fn candidate(
    src: &mut Source,
    window: &[u8],
    i: usize,
    at: u64,
    wanted: &[bool; 7],
) -> Option<Found> {
    let rest = &window[i..];
    match rest[0] {
        b'M' if wanted[0] && rest.starts_with(b"MZ") => pe(src, at),
        0x7f if wanted[1] && rest.starts_with(b"\x7fELF") => elf(src, at),
        b'P' if wanted[2] && rest.starts_with(b"PK\x05\x06") => zip(src, at),
        0x89 if wanted[3] && rest.starts_with(b"\x89PNG\r\n\x1a\n") => png(src, at),
        b'D' if wanted[4] && rest.starts_with(b"DDS ") => dds(src, at),
        0x1f if wanted[5]
            && rest.len() >= 4
            && rest[1] == 0x8b
            && rest[2] == 8
            && rest[3] & 0xe0 == 0 =>
        {
            inflate(src, at, true)
        }
        0x78 if wanted[6] && rest.len() >= 2 && matches!(rest[1], 0x01 | 0x5e | 0x9c | 0xda) => {
            inflate(src, at, false)
        }
        _ => None,
    }
}

pub fn carve(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let file = File::open(path).map_err(|e| io_error("Cannot open file", e))?;
    let len = file
        .metadata()
        .map_err(|e| io_error("Cannot inspect file", e))?
        .len();
    let start = optional_u64(args, "offset", 0, len)?;
    let max_scan = optional_u64(args, "maxScanBytes", MAX_SCAN_BYTES, MAX_SCAN_BYTES)?;
    let limit = optional_u64(args, "limit", 100, 10_000)? as usize;
    if limit == 0 {
        return Err("'limit' must be at least 1.".into());
    }
    let skip_contained = optional_bool(args, "skipContained", true)?;
    let requested = string_array(args, "types", false)?;
    let mut wanted = [requested.is_empty(); 7];
    for kind in &requested {
        let index = TYPES
            .iter()
            .position(|t| t.eq_ignore_ascii_case(kind))
            .ok_or_else(|| format!("Unknown type '{kind}'; use {}.", TYPES.join(", ")))?;
        wanted[index] = true;
    }
    let scan_end = len.min(start.saturating_add(max_scan));
    let mut scanner = file
        .try_clone()
        .map_err(|e| io_error("Cannot open file", e))?;
    let mut src = Source { file, len };
    let mut found: Vec<Found> = Vec::new();
    let mut covered_until = 0u64;
    let mut position = start;
    let mut next = None;
    let mut window = Vec::with_capacity(CHUNK + OVERLAP);
    'scan: while position < scan_end {
        let length = (scan_end - position).min(CHUNK as u64) as usize;
        let read = (len - position).min((length + OVERLAP) as u64) as usize;
        window.resize(read, 0);
        scanner
            .seek(SeekFrom::Start(position))
            .and_then(|_| scanner.read_exact(&mut window))
            .map_err(|e| io_error("Cannot read file", e))?;
        for i in 0..length {
            let at = position + i as u64;
            if skip_contained && at < covered_until {
                continue;
            }
            let Some(hit) = candidate(&mut src, &window, i, at, &wanted) else {
                continue;
            };
            if found.len() == limit {
                next = Some(at);
                break 'scan;
            }
            // Only leaf formats hide what they contain (for example a PNG's zlib stream);
            // executables and archives often embed other files worth reporting.
            if let Some(size) = hit.size
                && matches!(hit.kind, "png" | "dds" | "gzip" | "zlib")
            {
                covered_until = covered_until.max(hit.offset + size);
            }
            found.push(hit);
        }
        position += length as u64;
    }
    found.sort_by_key(|f| f.offset);
    let scanned = next.unwrap_or(position.min(scan_end)) - start;
    Ok(json!({
        "path":path.to_string_lossy(),"fileSize":len,"startOffset":start,"scannedBytes":scanned,
        "matches":found.iter().map(|f| json!({"offset":f.offset,"type":f.kind,"sizeBytes":f.size,
            "end":f.size.map(|s| f.offset + s),"details":f.details})).collect::<Vec<_>>(),
        "limitReached":next.is_some(),
        "nextOffset":next.or((position < len && scan_end < len).then_some(scan_end))
    }))
}
