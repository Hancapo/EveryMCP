use super::{io_error, optional_bool, optional_str, optional_u64, required_str};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "file_tail" => tail(args),
        "file_diff" => diff(args),
        "file_trash" => trash_file(args),
        "binary_pattern_search" => pattern_search(args),
        "image_inspect" => image_inspect(args),
        "text_transcode" => transcode(args),
        _ => unreachable!(),
    }
}

fn decode_text(bytes: &[u8]) -> Result<(&'static str, String), String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        let body = &bytes[2..];
        if !body.len().is_multiple_of(2) {
            return Err("UTF-16 input has an odd byte count.".into());
        }
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        return Ok((
            if little { "utf16le" } else { "utf16be" },
            String::from_utf16(&units).map_err(|_| "Invalid UTF-16 text.".to_owned())?,
        ));
    }
    let body = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    Ok((
        "utf8",
        String::from_utf8(body.to_vec()).map_err(|_| "Invalid UTF-8 text.".to_owned())?,
    ))
}

fn tail(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let lines = optional_u64(args, "lines", 100, 1000)? as usize;
    let max_bytes = optional_u64(args, "maxBytes", 65536, 1048576)?;
    if lines == 0 || max_bytes == 0 {
        return Err("'lines' and 'maxBytes' must be positive.".into());
    }
    let mut file = File::open(path).map_err(|e| io_error("Cannot open log", e))?;
    let size = file
        .metadata()
        .map_err(|e| io_error("Cannot inspect log", e))?
        .len();
    let cursor = args
        .get("cursor")
        .map(|_| optional_u64(args, "cursor", 0, u64::MAX))
        .transpose()?;
    let mut start = cursor.unwrap_or(size.saturating_sub(max_bytes));
    let rotated = start > size;
    if rotated {
        start = 0;
    }
    let truncated = size.saturating_sub(start) > max_bytes;
    if truncated {
        start = size - max_bytes;
    }
    let mut bom = [0u8; 3];
    let read = file
        .read(&mut bom)
        .map_err(|e| io_error("Cannot read log", e))?;
    let utf16 = read >= 2 && (bom[..2] == [0xff, 0xfe] || bom[..2] == [0xfe, 0xff]);
    if utf16 && start > 2 && start % 2 != 0 {
        start += 1;
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|e| io_error("Cannot seek log", e))?;
    let mut bytes = Vec::new();
    file.take(max_bytes)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("Cannot read log", e))?;
    let (_, text) = if utf16 && start > 0 {
        let mut data = bom[..2].to_vec();
        data.extend_from_slice(&bytes[..bytes.len() / 2 * 2]);
        decode_text(&data)?
    } else if start > 0 {
        let first = (0..bytes.len().min(4))
            .find(|&i| std::str::from_utf8(&bytes[i..]).is_ok())
            .ok_or("Cannot find UTF-8 boundary.")?;
        decode_text(&bytes[first..])?
    } else {
        decode_text(&bytes)?
    };
    let text = if (truncated || cursor.is_none() && start > 0) && start > 0 {
        text.split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or("")
            .to_owned()
    } else {
        text
    };
    let rows: Vec<&str> = text.lines().collect();
    let output = rows[rows.len().saturating_sub(lines)..].join("\n");
    Ok(
        json!({"path":path.to_string_lossy(),"text":output,"nextCursor":size,"startOffset":start,
        "truncated":truncated,"rotated":rotated,"lineCount":rows.len().min(lines)}),
    )
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let meta = fs::metadata(path).map_err(|e| io_error("Cannot inspect file", e))?;
    if !meta.is_file() || meta.len() > max {
        return Err(format!("Input must be a file of at most {max} bytes."));
    }
    fs::read(path).map_err(|e| io_error("Cannot read file", e))
}

fn diff(args: &Value) -> Result<Value, String> {
    let left = Path::new(required_str(args, "leftPath")?);
    let right = Path::new(required_str(args, "rightPath")?);
    let mode = optional_str(args, "mode")?.unwrap_or("text");
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    if limit == 0 {
        return Err("'limit' must be positive.".into());
    }
    let a = read_bounded(left, 8 * 1024 * 1024)?;
    let b = read_bounded(right, 8 * 1024 * 1024)?;
    let mut changes = Vec::new();
    let mut change_count = 0usize;
    let preview = |line: Option<&&str>| {
        line.map(|value| {
            let text: String = value.chars().take(512).collect();
            if value.chars().count() > 512 {
                format!("{text}…")
            } else {
                text
            }
        })
    };
    match mode {
        "text" => {
            let a = std::str::from_utf8(&a)
                .map_err(|_| "Left file is not UTF-8; use mode=binary.".to_owned())?;
            let b = std::str::from_utf8(&b)
                .map_err(|_| "Right file is not UTF-8; use mode=binary.".to_owned())?;
            let al: Vec<_> = a.split('\n').collect();
            let bl: Vec<_> = b.split('\n').collect();
            for index in 0..al.len().max(bl.len()) {
                if al.get(index) != bl.get(index) {
                    change_count += 1;
                    if changes.len() < limit {
                        changes.push(json!({"line":index+1,"left":preview(al.get(index)),"right":preview(bl.get(index))}));
                    }
                }
            }
        }
        "binary" => {
            for index in 0..a.len().max(b.len()) {
                if a.get(index) != b.get(index) {
                    change_count += 1;
                    if changes.len() < limit {
                        changes.push(
                            json!({"offset":index,"left":a.get(index).map(|v|format!("{v:02x}")),
                        "right":b.get(index).map(|v|format!("{v:02x}"))}),
                        );
                    }
                }
            }
        }
        _ => return Err("'mode' must be text or binary.".into()),
    }
    Ok(
        json!({"mode":mode,"equal":a==b,"changeCount":change_count,"changes":changes,
        "truncated":change_count>limit,"leftSha256":super::files::digest_hex(&Sha256::digest(&a)),
        "rightSha256":super::files::digest_hex(&Sha256::digest(&b))}),
    )
}

fn trash_file(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let metadata = fs::symlink_metadata(path).map_err(|e| io_error("Cannot inspect path", e))?;
    let permanent = optional_bool(args, "permanent", false)?;
    if permanent {
        if metadata.file_type().is_symlink() {
            return Err("Permanent deletion of symlinks is not supported.".into());
        }
        if metadata.is_dir() {
            fs::remove_dir_all(path)
                .map_err(|e| io_error("Cannot permanently delete directory", e))?;
        } else if metadata.is_file() {
            fs::remove_file(path).map_err(|e| io_error("Cannot permanently delete file", e))?;
        } else {
            return Err("Unsupported path type for permanent deletion.".into());
        }
    } else {
        trash::delete(path).map_err(|e| io_error("Cannot move path to Trash", e))?;
    }
    Ok(json!({"path":path.to_string_lossy(),"trashed":!permanent,"permanentlyDeleted":permanent}))
}

fn parse_pattern(raw: &str) -> Result<Vec<Option<u8>>, String> {
    let compact = raw.split_whitespace().collect::<String>();
    if compact.is_empty() || compact.len() % 2 != 0 || compact.len() > 512 {
        return Err("Pattern must contain 1-256 hex bytes or ?? wildcards.".into());
    }
    compact
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            if pair == b"??" {
                Ok(None)
            } else {
                let s = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(s, 16)
                    .map(Some)
                    .map_err(|_| "Invalid hex pattern.".to_owned())
            }
        })
        .collect()
}

fn pattern_search(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let pattern = parse_pattern(required_str(args, "pattern")?)?;
    if pattern.iter().all(Option::is_none) {
        return Err("Pattern must include at least one fixed byte.".into());
    }
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    let start = optional_u64(args, "startOffset", 0, u64::MAX)?;
    if limit == 0 {
        return Err("'limit' must be positive.".into());
    }
    let mut file = File::open(path).map_err(|e| io_error("Cannot open file", e))?;
    file.seek(SeekFrom::Start(start))
        .map_err(|e| io_error("Cannot seek file", e))?;
    let mut matches = Vec::new();
    let mut chunk = vec![0u8; 65536];
    let mut carry = Vec::new();
    let mut position = start;
    loop {
        let n = file
            .read(&mut chunk)
            .map_err(|e| io_error("Cannot scan file", e))?;
        if n == 0 {
            break;
        }
        let base = position.saturating_sub(carry.len() as u64);
        carry.extend_from_slice(&chunk[..n]);
        if carry.len() >= pattern.len() {
            for i in 0..=carry.len() - pattern.len() {
                if pattern
                    .iter()
                    .enumerate()
                    .all(|(j, byte)| byte.is_none_or(|v| carry[i + j] == v))
                {
                    matches.push(base + i as u64);
                    if matches.len() >= limit {
                        return Ok(
                            json!({"path":path.to_string_lossy(),"offsets":matches,"limitReached":true}),
                        );
                    }
                }
            }
        }
        let keep = pattern.len() - 1;
        let drain = carry.len().saturating_sub(keep);
        carry.drain(..drain);
        position += n as u64;
    }
    Ok(json!({"path":path.to_string_lossy(),"offsets":matches,"limitReached":false}))
}

fn image_inspect(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let size = imagesize::size(path).map_err(|e| io_error("Cannot inspect image dimensions", e))?;
    let mut file = File::open(path).map_err(|e| io_error("Cannot open image", e))?;
    let mut head = [0u8; 32];
    let n = file
        .read(&mut head)
        .map_err(|e| io_error("Cannot read image", e))?;
    let h = &head[..n];
    let (format, alpha): (&str, Option<bool>) = if h.starts_with(b"\x89PNG\r\n\x1a\n") {
        (
            "png",
            h.get(25).and_then(|c| match c {
                4 | 6 => Some(true),
                0 | 2 => Some(false),
                _ => None,
            }),
        )
    } else if h.starts_with(b"\xff\xd8\xff") {
        ("jpeg", Some(false))
    } else if h.starts_with(b"GIF8") {
        ("gif", None)
    } else if h.starts_with(b"BM") {
        ("bmp", None)
    } else if h.starts_with(b"RIFF") && h.get(8..12) == Some(b"WEBP") {
        ("webp", None)
    } else if h.starts_with(b"II*\0") || h.starts_with(b"MM\0*") {
        ("tiff", None)
    } else {
        ("unknown", None)
    };
    let exif = exif::Reader::new()
        .read_from_container(&mut std::io::BufReader::new(
            File::open(path).map_err(|e| io_error("Cannot open image", e))?,
        ))
        .ok();
    let mut metadata = Vec::new();
    if let Some(exif) = exif {
        for field in exif.fields().take(40) {
            metadata.push(json!({"tag":format!("{:?}",field.tag),"value":field.display_value().with_unit(&exif).to_string().chars().take(256).collect::<String>()}));
        }
    }
    Ok(
        json!({"path":path.to_string_lossy(),"format":format,"width":size.width,"height":size.height,
        "hasAlpha":alpha,"exif":metadata}),
    )
}

fn transcode(args: &Value) -> Result<Value, String> {
    let source = Path::new(required_str(args, "source")?);
    let dest = Path::new(required_str(args, "destination")?);
    let encoding = required_str(args, "encoding")?;
    let endings = optional_str(args, "lineEndings")?.unwrap_or("preserve");
    let overwrite = optional_bool(args, "overwrite", false)?;
    let bom = optional_bool(args, "bom", encoding != "utf8")?;
    if !["utf8", "utf16le", "utf16be"].contains(&encoding) {
        return Err("'encoding' must be utf8, utf16le or utf16be.".into());
    }
    if !["preserve", "lf", "crlf"].contains(&endings) {
        return Err("'lineEndings' must be preserve, lf or crlf.".into());
    }
    if source == dest && !overwrite {
        return Err("In-place conversion requires overwrite=true.".into());
    }
    if dest.exists() && !overwrite {
        return Err("Destination already exists.".into());
    }
    let input = read_bounded(source, 32 * 1024 * 1024)?;
    let (input_encoding, mut text) = decode_text(&input)?;
    if endings != "preserve" {
        text = text.replace("\r\n", "\n").replace('\r', "\n");
        if endings == "crlf" {
            text = text.replace('\n', "\r\n");
        }
    }
    let mut output = Vec::new();
    match encoding {
        "utf8" => {
            if bom {
                output.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            }
            output.extend_from_slice(text.as_bytes());
        }
        "utf16le" | "utf16be" => {
            if bom {
                output.extend_from_slice(if encoding == "utf16le" {
                    &[0xff, 0xfe]
                } else {
                    &[0xfe, 0xff]
                });
            }
            for code in text.encode_utf16() {
                output.extend_from_slice(&if encoding == "utf16le" {
                    code.to_le_bytes()
                } else {
                    code.to_be_bytes()
                });
            }
        }
        _ => unreachable!(),
    }
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| io_error("Cannot create temporary file", e))?;
    temp.write_all(&output)
        .map_err(|e| io_error("Cannot write output", e))?;
    let result = if overwrite {
        temp.persist(dest)
    } else {
        temp.persist_noclobber(dest)
    };
    result.map_err(|e| io_error("Cannot persist output", e.error))?;
    Ok(
        json!({"source":source.to_string_lossy(),"destination":dest.to_string_lossy(),
        "inputEncoding":input_encoding,"outputEncoding":encoding,"bytesWritten":output.len()}),
    )
}
