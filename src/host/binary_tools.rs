use super::{
    MAX_COLLECTION_ENTRIES, io_error, optional_bool, optional_str, optional_u64, required_str,
    string_array,
};
use goblin::pe::PE;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_STRING_CHARS: usize = 256;
const MAX_PE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_GRAPH_BYTES: u64 = 512 * 1024 * 1024;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "binary_strings" => strings(args),
        "binary_entropy" => entropy(args),
        "pe_dependencies" => dependencies(args),
        _ => unreachable!(),
    }
}

fn open_binary(args: &Value) -> Result<(&Path, File, u64, u64), String> {
    let path = Path::new(required_str(args, "path")?);
    let file = File::open(path).map_err(|e| io_error("Cannot open binary file", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| io_error("Cannot inspect binary file", e))?;
    if !metadata.is_file() {
        return Err("'path' must be a regular file.".into());
    }
    let offset = optional_u64(args, "offset", 0, metadata.len())?;
    Ok((path, file, metadata.len(), offset))
}

struct StringRun {
    start: u64,
    end: u64,
    length: u64,
    text: String,
    matched: bool,
    filter_position: usize,
}

struct StringResults {
    offset: u64,
    minimum: u64,
    limit: usize,
    filter: Vec<char>,
    filter_prefix: Vec<usize>,
    matches: BTreeMap<u64, (u8, Value)>,
}

impl StringResults {
    fn finish(&mut self, run: StringRun, encoding: u8) {
        if run.start < self.offset || run.length < self.minimum || !run.matched {
            return;
        }
        if self
            .matches
            .get(&run.start)
            .is_some_and(|(rank, _)| *rank <= encoding)
        {
            return;
        }
        let value = json!({"offset":run.start,"encoding":encoding_name(encoding),
            "text":run.text,"length":run.length,"byteLength":run.end-run.start,
            "truncated":run.length>MAX_STRING_CHARS as u64});
        self.matches.insert(run.start, (encoding, value));
        if self.matches.len() > self.limit + 1 {
            self.matches.pop_last();
        }
    }
}

struct StringScanner {
    encoding: u8,
    parity: u64,
    run: Option<StringRun>,
    high_surrogate: Option<(u16, u64)>,
}

impl StringScanner {
    fn finish(&mut self, results: &mut StringResults) {
        if let Some(run) = self.run.take() {
            results.finish(run, self.encoding);
        }
    }

    fn character(&mut self, ch: Option<char>, start: u64, end: u64, results: &mut StringResults) {
        let Some(ch) = ch.filter(|c| !c.is_control() && (!c.is_whitespace() || *c == ' ')) else {
            self.finish(results);
            return;
        };
        let run = self.run.get_or_insert_with(|| StringRun {
            start,
            end,
            length: 0,
            text: String::new(),
            matched: results.filter.is_empty(),
            filter_position: 0,
        });
        run.end = end;
        if run.length < MAX_STRING_CHARS as u64 {
            run.text.push(ch);
        }
        run.length += 1;
        if !run.matched {
            while run.filter_position > 0 && results.filter[run.filter_position] != ch {
                run.filter_position = results.filter_prefix[run.filter_position - 1];
            }
            if results.filter[run.filter_position] == ch {
                run.filter_position += 1;
                run.matched = run.filter_position == results.filter.len();
            }
        }
    }

    fn word(&mut self, unit: u16, offset: u64, results: &mut StringResults) {
        if let Some((high, start)) = self.high_surrogate.take() {
            if (0xdc00..=0xdfff).contains(&unit) {
                let scalar = 0x10000 + (((high as u32 - 0xd800) << 10) | (unit as u32 - 0xdc00));
                self.character(char::from_u32(scalar), start, offset + 2, results);
                return;
            }
            self.finish(results);
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.high_surrogate = Some((unit, offset));
        } else {
            self.character(char::from_u32(unit as u32), offset, offset + 2, results);
        }
    }

    fn pending_before(&self, offset: u64) -> bool {
        self.run.as_ref().is_some_and(|r| r.start <= offset)
            || self
                .high_surrogate
                .is_some_and(|(_, start)| start <= offset)
    }
}

fn encoding_name(encoding: u8) -> &'static str {
    match encoding {
        0 => "ascii",
        1 => "utf16le",
        _ => "utf16be",
    }
}

fn strings(args: &Value) -> Result<Value, String> {
    let encoding = optional_str(args, "encoding")?.unwrap_or("all");
    if !["ascii", "utf16le", "utf16be", "all"].contains(&encoding) {
        return Err("'encoding' must be ascii, utf16le, utf16be, or all.".into());
    }
    let minimum = optional_u64(args, "minLength", 4, MAX_STRING_CHARS as u64)?;
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    if minimum == 0 || limit == 0 {
        return Err("'minLength' and 'limit' must be positive.".into());
    }
    let filter: Vec<char> = optional_str(args, "filter")?
        .unwrap_or("")
        .chars()
        .collect();
    if filter.len() > MAX_STRING_CHARS {
        return Err("'filter' must contain at most 256 characters.".into());
    }
    let mut filter_prefix = vec![0; filter.len()];
    for i in 1..filter.len() {
        let mut j = filter_prefix[i - 1];
        while j > 0 && filter[i] != filter[j] {
            j = filter_prefix[j - 1];
        }
        if filter[i] == filter[j] {
            j += 1;
        }
        filter_prefix[i] = j;
    }
    let (path, mut file, size, offset) = open_binary(args)?;
    // Look behind the cursor so a continuation never reports the tail of an earlier run.
    let start = offset.saturating_sub(4);
    file.seek(SeekFrom::Start(start))
        .map_err(|e| io_error("Cannot seek binary file", e))?;
    let mut reader = file.take(size - start);
    let mut results = StringResults {
        offset,
        minimum,
        limit,
        filter,
        filter_prefix,
        matches: BTreeMap::new(),
    };
    let mut scanners = Vec::new();
    for rank in 0..3 {
        if encoding == "all" || encoding == encoding_name(rank) {
            for parity in 0..if rank == 0 { 1 } else { 2 } {
                scanners.push(StringScanner {
                    encoding: rank,
                    parity,
                    run: None,
                    high_surrogate: None,
                });
            }
        }
    }
    let mut buffer = [0; 65536];
    let mut position = start;
    let mut previous = None;
    'scan: loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|e| io_error("Cannot read binary file", e))?;
        if count == 0 {
            break;
        }
        for &byte in &buffer[..count] {
            for scanner in &mut scanners {
                if scanner.encoding == 0 {
                    scanner.character(
                        (0x20..=0x7e).contains(&byte).then_some(byte as char),
                        position,
                        position + 1,
                        &mut results,
                    );
                } else if let Some(previous) = previous {
                    let word_start = position - 1;
                    if word_start % 2 == scanner.parity {
                        let pair = [previous, byte];
                        let unit = if scanner.encoding == 1 {
                            u16::from_le_bytes(pair)
                        } else {
                            u16::from_be_bytes(pair)
                        };
                        scanner.word(unit, word_start, &mut results);
                    }
                }
            }
            previous = Some(byte);
            position += 1;
            if results.matches.len() > limit {
                let last = *results.matches.last_key_value().unwrap().0;
                if !scanners.iter().any(|scanner| scanner.pending_before(last)) {
                    break 'scan;
                }
            }
        }
    }
    for scanner in &mut scanners {
        scanner.finish(&mut results);
    }
    let next_offset = results.matches.keys().nth(limit).copied();
    let matches: Vec<Value> = results
        .matches
        .into_values()
        .take(limit)
        .map(|(_, value)| value)
        .collect();
    Ok(
        json!({"path":path,"sizeBytes":size,"offset":offset,"strings":matches,
        "nextOffset":next_offset,"limitReached":next_offset.is_some(),"maxStringChars":MAX_STRING_CHARS,
        "sameOffsetPreference":["ascii","utf16le","utf16be"]}),
    )
}

fn shannon_entropy(bytes: &[u8]) -> f64 {
    let mut counts = [0u64; 256];
    for &byte in bytes {
        counts[byte as usize] += 1;
    }
    counts
        .iter()
        .filter(|&&count| count > 0)
        .map(|&count| {
            let probability = count as f64 / bytes.len() as f64;
            -probability * probability.log2()
        })
        .sum()
}

fn entropy(args: &Value) -> Result<Value, String> {
    let block_size = optional_u64(args, "blockSize", 65536, 1024 * 1024)? as usize;
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    if block_size == 0 || limit == 0 {
        return Err("'blockSize' and 'limit' must be positive.".into());
    }
    let (path, mut file, size, offset) = open_binary(args)?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| io_error("Cannot seek binary file", e))?;
    let mut buffer = vec![0; block_size];
    let mut position = offset;
    let mut blocks = Vec::new();
    while position < size && blocks.len() < limit {
        let count = (size - position).min(block_size as u64) as usize;
        file.read_exact(&mut buffer[..count])
            .map_err(|e| io_error("Cannot read entropy block", e))?;
        blocks.push(json!({"offset":position,"sizeBytes":count,"entropyBitsPerByte":shannon_entropy(&buffer[..count])}));
        position += count as u64;
    }
    Ok(
        json!({"path":path,"sizeBytes":size,"offset":offset,"blockSize":block_size,"blocks":blocks,
        "nextOffset":(position<size).then_some(position),"limitReached":position<size}),
    )
}

struct PeImage {
    machine: u16,
    imports: Vec<String>,
    import_count: usize,
}

fn read_pe(path: &Path, remaining_bytes: &mut u64, limit: usize) -> Result<PeImage, String> {
    let file = File::open(path).map_err(|e| io_error("Cannot open PE file", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| io_error("Cannot inspect PE file", e))?;
    if !metadata.is_file() || metadata.len() > MAX_PE_BYTES {
        return Err("PE input must be a regular file of at most 256 MiB.".into());
    }
    if metadata.len() > *remaining_bytes {
        return Err("PE dependency graph exceeds the 512 MiB total input budget.".into());
    }
    let max_read = (*remaining_bytes).min(MAX_PE_BYTES);
    let mut bytes = Vec::new();
    file.take(max_read + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_error("Cannot read PE file", e))?;
    if bytes.len() as u64 > max_read {
        return Err("PE input exceeded its byte budget while reading.".into());
    }
    *remaining_bytes -= bytes.len() as u64;
    let pe = PE::parse(&bytes).map_err(|e| io_error("Invalid PE image", e))?;
    let mut imports = BTreeSet::new();
    for name in &pe.libraries {
        if name.len() > 260 {
            return Err("PE import name exceeds 260 bytes.".into());
        }
        imports.insert(name.to_ascii_lowercase());
        if imports.len() > limit + 1 {
            imports.pop_last();
        }
    }
    Ok(PeImage {
        machine: pe.header.coff_header.machine,
        imports: imports.into_iter().collect(),
        import_count: pe.libraries.len(),
    })
}

#[derive(Default)]
struct SearchDirectories {
    indexes: BTreeMap<PathBuf, BTreeMap<String, Vec<PathBuf>>>,
    entries: usize,
}

impl SearchDirectories {
    fn candidates(&mut self, roots: &[PathBuf], name: &str) -> Result<Vec<PathBuf>, String> {
        let mut candidates = BTreeSet::new();
        for root in roots {
            if !self.indexes.contains_key(root) {
                let mut files: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
                for entry in fs::read_dir(root)
                    .map_err(|e| io_error("Cannot read dependency search directory", e))?
                {
                    let entry =
                        entry.map_err(|e| io_error("Cannot read dependency directory entry", e))?;
                    self.entries += 1;
                    if self.entries > MAX_COLLECTION_ENTRIES {
                        return Err(
                            "Dependency search directories exceed the 100000-entry scan limit."
                                .into(),
                        );
                    }
                    if entry
                        .file_type()
                        .map_err(|e| io_error("Cannot inspect dependency candidate", e))?
                        .is_dir()
                    {
                        continue;
                    }
                    files
                        .entry(entry.file_name().to_string_lossy().to_ascii_lowercase())
                        .or_default()
                        .push(entry.path());
                }
                self.indexes.insert(root.clone(), files);
            }
            if let Some(paths) = self.indexes[root].get(name) {
                for path in paths {
                    let candidate = fs::canonicalize(path)
                        .map_err(|e| io_error("Cannot resolve dependency candidate", e))?;
                    if candidate.is_file() {
                        candidates.insert(candidate);
                    }
                    if candidates.len() > 64 {
                        return Err("Import has more than 64 candidate files.".into());
                    }
                }
            }
        }
        Ok(candidates.into_iter().collect())
    }
}

fn has_path(graph: &BTreeMap<PathBuf, Vec<PathBuf>>, start: &Path, end: &Path) -> bool {
    let mut pending = vec![start.to_path_buf()];
    let mut seen = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if path == end {
            return true;
        }
        if seen.insert(path.clone())
            && let Some(next) = graph.get(&path)
        {
            pending.extend(next.iter().cloned());
        }
    }
    false
}

fn dependencies(args: &Value) -> Result<Value, String> {
    let path = fs::canonicalize(required_str(args, "path")?)
        .map_err(|e| io_error("Cannot resolve PE path", e))?;
    let roots = string_array(args, "searchPaths", true)?;
    if roots.len() > 64 {
        return Err("'searchPaths' may contain at most 64 directories.".into());
    }
    let mut search_paths = BTreeSet::new();
    for root in roots {
        let root = fs::canonicalize(root)
            .map_err(|e| io_error("Cannot resolve dependency search directory", e))?;
        if !root.is_dir() {
            return Err("'searchPaths' must contain existing directories.".into());
        }
        search_paths.insert(root);
    }
    let recursive = optional_bool(args, "recursive", true)?;
    let limit = optional_u64(args, "limit", 200, 1000)? as usize;
    if limit == 0 {
        return Err("'limit' must be positive.".into());
    }
    let mut remaining_bytes = MAX_GRAPH_BYTES;
    let root_image = read_pe(&path, &mut remaining_bytes, limit)?;
    let mut images = BTreeMap::from([(path.clone(), root_image)]);
    let mut pending = VecDeque::from([path.clone()]);
    let mut visited = BTreeSet::new();
    let mut directories = SearchDirectories::default();
    let mut dependencies = Vec::new();
    let mut contracts = Vec::new();
    let mut graph: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut limit_reached = false;
    'graph: while let Some(source) = pending.pop_front() {
        if !visited.insert(source.clone()) {
            continue;
        }
        let image = &images[&source];
        let machine = image.machine;
        let imports = image.imports.clone();
        let mut roots = search_paths.clone();
        if let Some(parent) = source.parent() {
            roots.insert(parent.to_path_buf());
        }
        let roots: Vec<_> = roots.into_iter().collect();
        for name in imports {
            if dependencies.len() + contracts.len() >= limit {
                limit_reached = true;
                break 'graph;
            }
            if name.starts_with("api-ms-") || name.starts_with("ext-ms-") {
                contracts.push(json!({"from":source,"name":name,"status":"unresolved"}));
                continue;
            }
            let mut edge = json!({"from":source,"name":name,"status":"missing","candidates":[],"resolvedPath":null,"cycle":false});
            if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
                edge["status"] = json!("invalidImportName");
                dependencies.push(edge);
                continue;
            }
            let candidates = directories.candidates(&roots, &name)?;
            edge["candidates"] = json!(candidates);
            if candidates.len() > 1 {
                edge["status"] = json!("ambiguous");
            } else if let Some(target) = candidates.first() {
                if !images.contains_key(target) {
                    match read_pe(target, &mut remaining_bytes, limit) {
                        Ok(image) => {
                            images.insert(target.clone(), image);
                        }
                        Err(error) => {
                            edge["status"] = json!("inspectionError");
                            edge["error"] = json!(error);
                            dependencies.push(edge);
                            continue;
                        }
                    }
                }
                let target_machine = images[target].machine;
                edge["machine"] = json!(format!("0x{target_machine:04x}"));
                edge["resolvedPath"] = json!(target);
                if target_machine != machine {
                    edge["status"] = json!("architectureMismatch");
                    edge["expectedMachine"] = json!(format!("0x{machine:04x}"));
                } else {
                    edge["status"] = json!("resolved");
                    graph
                        .entry(source.clone())
                        .or_default()
                        .push(target.clone());
                    if recursive && !visited.contains(target) {
                        pending.push_back(target.clone());
                    }
                }
            }
            dependencies.push(edge);
        }
    }
    for edge in &mut dependencies {
        if edge["status"] == "resolved" {
            let from = Path::new(edge["from"].as_str().unwrap());
            let to = Path::new(edge["resolvedPath"].as_str().unwrap());
            edge["cycle"] = json!(has_path(&graph, to, from));
        }
    }
    let nodes: Vec<_> = images
        .iter()
        .map(|(path, image)| {
            json!({"path":path,"machine":format!("0x{:04x}",image.machine),
        "importCount":image.import_count,"importsTraversed":visited.contains(path)})
        })
        .collect();
    Ok(
        json!({"path":path,"searchPaths":search_paths,"recursive":recursive,"nodes":nodes,
        "dependencies":dependencies,"apiSetContracts":contracts,"limitReached":limit_reached,
        "bytesRead":MAX_GRAPH_BYTES-remaining_bytes,
        "note":"Static import-directory candidates only. Searches each image's parent and explicit searchPaths. Ambiguous candidates are not selected. API-set contracts, delay imports, manifests, exports/forwarders and runtime loader resolution are not resolved."}),
    )
}
