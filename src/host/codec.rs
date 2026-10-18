use super::{io_error, optional_bool, optional_str, optional_u64, required_str};
use crate::hashing::{self, Algorithm, CaseFold};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

const MAX_INPUT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STEP_BYTES: usize = 64 * 1024 * 1024;
const MAX_STEPS: usize = 32;
const MAX_TARGETS: usize = 100_000;
const MAX_WORDLIST_BYTES: u64 = 1024 * 1024 * 1024;
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "data_transform" => transform(args),
        "hash_dictionary_match" => dictionary_match(args),
        _ => unreachable!(),
    }
}

fn base64_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let word = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            result.push(if i <= chunk.len() {
                BASE64[(word >> (18 - 6 * i) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    result
}

fn radix_decode(text: &[u8], alphabet: &[u8], bits: u32, name: &str) -> Result<Vec<u8>, String> {
    let mut result = Vec::with_capacity(text.len() * bits as usize / 8);
    let mut buffer = 0u32;
    let mut held = 0u32;
    let mut padding = false;
    for &byte in text {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            padding = true;
            continue;
        }
        // Accept the URL-safe base64 alphabet and lowercase base32 as well.
        let byte = match (bits, byte) {
            (6, b'-') => b'+',
            (6, b'_') => b'/',
            (5, b'a'..=b'z') => byte.to_ascii_uppercase(),
            _ => byte,
        };
        let index = alphabet
            .iter()
            .position(|&c| c == byte)
            .filter(|_| !padding)
            .ok_or_else(|| format!("Invalid {name} input."))?;
        buffer = (buffer << bits) | index as u32;
        held += bits;
        if held >= 8 {
            held -= 8;
            result.push((buffer >> held) as u8);
            buffer &= (1 << held) - 1;
        }
    }
    if buffer != 0 {
        return Err(format!("Invalid {name} input: nonzero trailing bits."));
    }
    Ok(result)
}

fn base32_encode(data: &[u8]) -> String {
    let mut result = String::with_capacity(data.len().div_ceil(5) * 8);
    for chunk in data.chunks(5) {
        let mut block = [0u8; 5];
        block[..chunk.len()].copy_from_slice(chunk);
        let word = block.iter().fold(0u64, |w, &b| (w << 8) | b as u64);
        let symbols = (chunk.len() * 8).div_ceil(5);
        for i in 0..8 {
            result.push(if i < symbols {
                BASE32[(word >> (35 - 5 * i) & 31) as usize] as char
            } else {
                '='
            });
        }
    }
    result
}

fn bounded_read(reader: impl Read, what: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    reader
        .take(MAX_STEP_BYTES as u64 + 1)
        .read_to_end(&mut output)
        .map_err(|e| io_error(&format!("{what} failed"), e))?;
    if output.len() > MAX_STEP_BYTES {
        return Err(format!("{what} output exceeds 64 MiB."));
    }
    Ok(output)
}

fn byte_amount(step: &Value, key: &str, max: u64) -> Result<u64, String> {
    let value = super::required_u64(step, key)?;
    if value > max {
        return Err(format!("'{key}' must be at most {max}."));
    }
    Ok(value)
}

fn apply(step: &Value, data: Vec<u8>) -> Result<Vec<u8>, String> {
    use flate2::Compression;
    use flate2::read::{
        DeflateDecoder, DeflateEncoder, GzDecoder, GzEncoder, ZlibDecoder, ZlibEncoder,
    };

    let op = required_str(step, "op")?;
    let text = |data: &[u8]| String::from_utf8_lossy(data).into_owned();
    Ok(match op {
        "xor" => {
            let key = crate::bytes::parse_hex(required_str(step, "key")?)?;
            if key.is_empty() || key.len() > 4096 {
                return Err("xor 'key' must contain 1 to 4096 hex bytes.".into());
            }
            data.iter()
                .zip(key.iter().cycle())
                .map(|(b, k)| b ^ k)
                .collect()
        }
        "add" | "sub" => {
            let amount = byte_amount(step, "value", 255)? as u8;
            data.iter()
                .map(|&b| {
                    if op == "add" {
                        b.wrapping_add(amount)
                    } else {
                        b.wrapping_sub(amount)
                    }
                })
                .collect()
        }
        "rol" | "ror" => {
            let bits = byte_amount(step, "bits", 7)? as u32;
            data.iter()
                .map(|&b| {
                    if op == "rol" {
                        b.rotate_left(bits)
                    } else {
                        b.rotate_right(bits)
                    }
                })
                .collect()
        }
        "not" => data.iter().map(|b| !b).collect(),
        "reverse" => data.into_iter().rev().collect(),
        "slice" => {
            let offset = byte_amount(step, "offset", data.len() as u64)? as usize;
            let length = optional_u64(
                step,
                "length",
                (data.len() - offset) as u64,
                (data.len() - offset) as u64,
            )? as usize;
            data[offset..offset + length].to_vec()
        }
        "hex_encode" => crate::bytes::format_hex(&data)
            .replace(' ', "")
            .into_bytes(),
        "hex_decode" => crate::bytes::parse_hex(&text(&data))?,
        "base64_encode" => base64_encode(&data).into_bytes(),
        "base64_decode" => radix_decode(&data, BASE64, 6, "base64")?,
        "base32_encode" => base32_encode(&data).into_bytes(),
        "base32_decode" => radix_decode(&data, BASE32, 5, "base32")?,
        "zlib_decompress" => bounded_read(ZlibDecoder::new(&data[..]), "zlib decompression")?,
        "deflate_decompress" => {
            bounded_read(DeflateDecoder::new(&data[..]), "deflate decompression")?
        }
        "gzip_decompress" => bounded_read(GzDecoder::new(&data[..]), "gzip decompression")?,
        "zlib_compress" => bounded_read(
            ZlibEncoder::new(&data[..], Compression::default()),
            "zlib compression",
        )?,
        "deflate_compress" => bounded_read(
            DeflateEncoder::new(&data[..], Compression::default()),
            "deflate compression",
        )?,
        "gzip_compress" => bounded_read(
            GzEncoder::new(&data[..], Compression::default()),
            "gzip compression",
        )?,
        "lz4_block_compress" => lz4_flex::block::compress(&data),
        "lz4_block_decompress" => {
            let size = byte_amount(step, "size", MAX_STEP_BYTES as u64)? as usize;
            lz4_flex::block::decompress(&data, size)
                .map_err(|e| io_error("LZ4 block decompression failed", e))?
        }
        "lz4_frame_compress" => {
            let mut encoder = lz4_flex::frame::FrameEncoder::new(Vec::new());
            encoder
                .write_all(&data)
                .map_err(|e| io_error("LZ4 compression failed", e))?;
            encoder
                .finish()
                .map_err(|e| io_error("LZ4 compression failed", e))?
        }
        "lz4_frame_decompress" => bounded_read(
            lz4_flex::frame::FrameDecoder::new(&data[..]),
            "LZ4 frame decompression",
        )?,
        _ => return Err(format!("Unsupported transform op '{op}'.")),
    })
}

fn read_input(args: &Value) -> Result<(Vec<u8>, Value), String> {
    match (optional_str(args, "data")?, optional_str(args, "path")?) {
        (Some(data), None) => {
            let encoding = optional_str(args, "inputEncoding")?.unwrap_or("hex");
            let bytes = match encoding {
                "hex" => crate::bytes::parse_hex(data)?,
                "utf8" => data.as_bytes().to_vec(),
                "base64" => radix_decode(data.as_bytes(), BASE64, 6, "base64")?,
                _ => return Err("'inputEncoding' must be hex, utf8 or base64.".into()),
            };
            if bytes.len() as u64 > MAX_INPUT_BYTES {
                return Err("Input exceeds 16 MiB.".into());
            }
            Ok((bytes, json!({"kind":"data","encoding":encoding})))
        }
        (None, Some(path)) => {
            let mut file = File::open(path).map_err(|e| io_error("Cannot open input", e))?;
            let size = file
                .metadata()
                .map_err(|e| io_error("Cannot inspect input", e))?
                .len();
            let offset = optional_u64(args, "offset", 0, size)?;
            let length = optional_u64(args, "length", size - offset, size - offset)?;
            if length > MAX_INPUT_BYTES {
                return Err("Input range exceeds 16 MiB; pass 'length'.".into());
            }
            file.seek(SeekFrom::Start(offset))
                .map_err(|e| io_error("Cannot seek input", e))?;
            let mut bytes = vec![0; length as usize];
            file.read_exact(&mut bytes)
                .map_err(|e| io_error("Cannot read input", e))?;
            Ok((
                bytes,
                json!({"kind":"file","path":path,"offset":offset,"length":length}),
            ))
        }
        _ => Err("Supply exactly one of 'data' or 'path'.".into()),
    }
}

fn transform(args: &Value) -> Result<Value, String> {
    let (mut data, source) = read_input(args)?;
    let steps = args
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("'steps' must be an array of step objects.")?;
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err("'steps' must contain 1 to 32 steps.".into());
    }
    let output_encoding = optional_str(args, "outputEncoding")?.unwrap_or("hex");
    if !["hex", "utf8", "base64"].contains(&output_encoding) {
        return Err("'outputEncoding' must be hex, utf8 or base64.".into());
    }
    let max_output = optional_u64(args, "maxOutputBytes", 65536, 4 * 1024 * 1024)? as usize;
    let output_path = optional_str(args, "outputPath")?;
    let overwrite = optional_bool(args, "overwrite", false)?;
    let input_size = data.len();
    let mut report = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        if !step.is_object() {
            return Err(format!("steps[{index}] must be an object."));
        }
        data = apply(step, data).map_err(|e| format!("steps[{index}]: {e}"))?;
        report.push(json!({"op":step["op"],"sizeBytes":data.len()}));
    }
    if let Some(path) = output_path {
        super::files::persist_bytes(Path::new(path), &data, overwrite)?;
    }
    let shown = &data[..data.len().min(max_output)];
    let output = match output_encoding {
        "hex" => json!(crate::bytes::format_hex(shown)),
        "base64" => json!(base64_encode(shown)),
        _ => json!(String::from_utf8_lossy(shown)),
    };
    Ok(
        json!({"source":source,"inputSizeBytes":input_size,"steps":report,"sizeBytes":data.len(),
        "outputEncoding":output_encoding,"output":output,"truncated":shown.len()<data.len(),
        "sha256":super::files::digest_hex(&data),"outputPath":output_path}),
    )
}

fn strings(args: &Value, key: &str, max: usize) -> Result<Vec<String>, String> {
    let Some(value) = args.get(key) else {
        return Ok(Vec::new());
    };
    let array = value
        .as_array()
        .ok_or_else(|| format!("'{key}' must be an array of strings."))?;
    if array.len() > max {
        return Err(format!("'{key}' may contain at most {max} strings."));
    }
    array
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("'{key}' must contain only strings."))
        })
        .collect()
}

fn dictionary_match(args: &Value) -> Result<Value, String> {
    let algorithm = Algorithm::parse(required_str(args, "algorithm")?)?;
    let bits = algorithm.bits();
    let seed = crate::ops::hash_seed(args, algorithm)?;
    let fold = CaseFold::parse(optional_str(args, "caseFold")?)?;
    let encoding = optional_str(args, "encoding")?.unwrap_or("utf8");
    if !["utf8", "utf16le"].contains(&encoding) {
        return Err("'encoding' must be utf8 or utf16le.".into());
    }
    let raw_targets = strings(args, "targets", MAX_TARGETS)?;
    if raw_targets.is_empty() {
        return Err("'targets' must contain at least one hash.".into());
    }
    let mut targets: HashMap<u64, Vec<Value>> = HashMap::new();
    for raw in &raw_targets {
        let value = crate::input::parse_big_integer(raw, "targets")?;
        let value = num_traits::ToPrimitive::to_i128(&value).ok_or("Target hash is too large.")?;
        let value = crate::integer::unsigned(value, bits)
            .map_err(|_| format!("Target '{raw}' does not fit in {bits} bits."))?;
        targets.entry(value as u64).or_default();
    }
    let words = strings(args, "words", MAX_TARGETS)?;
    let path = optional_str(args, "path")?;
    if words.is_empty() && path.is_none() {
        return Err("Supply 'words', 'path' or both.".into());
    }
    let mut prefixes = strings(args, "prefixes", 256)?;
    let mut suffixes = strings(args, "suffixes", 256)?;
    if prefixes.is_empty() {
        prefixes.push(String::new());
    }
    if suffixes.is_empty() {
        suffixes.push(String::new());
    }
    let max_candidates = optional_u64(args, "maxCandidates", 10_000_000, 200_000_000)?;
    let limit = optional_u64(args, "limit", 1000, MAX_TARGETS as u64)? as usize;
    if limit == 0 || max_candidates == 0 {
        return Err("'limit' and 'maxCandidates' must be positive.".into());
    }
    let mut tried = 0u64;
    let mut words_read = 0u64;
    let mut exhausted = false;
    let mut matches = 0usize;
    let mut candidate = Vec::new();
    let mut visit = |word: &str| -> bool {
        words_read += 1;
        for prefix in &prefixes {
            for suffix in &suffixes {
                if tried == max_candidates {
                    exhausted = true;
                    return false;
                }
                tried += 1;
                let text = format!("{prefix}{word}{suffix}");
                candidate.clear();
                match encoding {
                    "utf16le" => candidate.extend(text.encode_utf16().flat_map(u16::to_le_bytes)),
                    _ => candidate.extend_from_slice(text.as_bytes()),
                }
                fold.apply(&mut candidate);
                if let Some(found) = targets.get_mut(&algorithm.hash(&candidate, seed))
                    && found.len() < 16
                    && !found.iter().any(|v| v == &json!(text))
                {
                    found.push(json!(text));
                    matches += 1;
                }
            }
        }
        true
    };
    let mut completed = words.iter().all(|word| visit(word));
    if completed && let Some(path) = path {
        let file = File::open(path).map_err(|e| io_error("Cannot open wordlist", e))?;
        if file
            .metadata()
            .map_err(|e| io_error("Cannot inspect wordlist", e))?
            .len()
            > MAX_WORDLIST_BYTES
        {
            return Err("Wordlist exceeds 1 GiB.".into());
        }
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        loop {
            line.clear();
            if reader
                .read_until(b'\n', &mut line)
                .map_err(|e| io_error("Cannot read wordlist", e))?
                == 0
            {
                break;
            }
            let word = String::from_utf8_lossy(&line);
            let word = word.trim_end_matches(['\r', '\n']);
            if word.is_empty() {
                continue;
            }
            if !visit(word) {
                completed = false;
                break;
            }
        }
    }
    let mut found: Vec<Value> = targets
        .iter()
        .filter(|(_, words)| !words.is_empty())
        .map(|(hash, words)| {
            let mut entry = hashing::describe(*hash, bits);
            entry["candidates"] = json!(words);
            entry
        })
        .collect();
    found.sort_by(|a, b| a["hex"].as_str().cmp(&b["hex"].as_str()));
    let mut unmatched: Vec<u64> = targets
        .iter()
        .filter(|(_, words)| words.is_empty())
        .map(|(hash, _)| *hash)
        .collect();
    unmatched.sort_unstable();
    let digits = (bits / 4) as usize;
    let unmatched_count = unmatched.len();
    Ok(
        json!({"algorithm":required_str(args, "algorithm")?,"bits":bits,"seed":seed.to_string(),
        "caseFold":optional_str(args, "caseFold")?.unwrap_or("none"),"encoding":encoding,
        "targetCount":targets.len(),"matchedCount":found.len(),"candidateCount":matches,
        "matches":found.into_iter().take(limit).collect::<Vec<_>>(),
        "unmatchedCount":unmatched_count,
        "unmatched":unmatched.iter().take(limit).map(|h| format!("0x{h:0digits$X}")).collect::<Vec<_>>(),
        "wordsRead":words_read,"candidatesTried":tried,"complete":completed && !exhausted,
        "maxCandidatesReached":exhausted}),
    )
}
