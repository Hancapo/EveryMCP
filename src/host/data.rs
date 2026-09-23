use super::{io_error, optional_str, optional_u64, required_str};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::path::Path;

const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_RESULT_BYTES: usize = 1024 * 1024;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "structured_data_query" => query(args),
        "structured_data_diff" => diff(args),
        _ => Err(format!("Unknown data tool '{name}'.")),
    }
}

pub(super) fn read_text(path: &Path) -> Result<String, String> {
    let metadata = std::fs::metadata(path).map_err(|e| io_error("Cannot inspect input file", e))?;
    if !metadata.is_file() {
        return Err("Input path must be a file.".into());
    }
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err("Input file exceeds 8 MiB.".into());
    }
    std::fs::read_to_string(path).map_err(|e| io_error("Cannot read UTF-8 input file", e))
}

pub(super) fn load(path: &Path, format: Option<&str>) -> Result<(String, Value), String> {
    let format = match format {
        Some(value) => value.to_ascii_lowercase(),
        None => match path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "json" => "json",
            "toml" => "toml",
            "yaml" | "yml" => "yaml",
            "xml" | "csproj" | "props" => "xml",
            "csv" => "csv",
            _ => return Err("Cannot infer format; pass 'format'.".into()),
        }
        .to_owned(),
    };
    let source = read_text(path)?;
    let value = match format.as_str() {
        "json" => serde_json::from_str(&source).map_err(|e| io_error("Invalid JSON", e))?,
        "toml" => toml::from_str(&source).map_err(|e| io_error("Invalid TOML", e))?,
        "yaml" => serde_yaml_ng::from_str(&source).map_err(|e| io_error("Invalid YAML", e))?,
        "xml" => parse_xml(&source)?,
        "csv" => parse_csv(&source)?,
        _ => return Err("'format' must be json, toml, yaml, xml or csv.".into()),
    };
    Ok((format, value))
}

fn parse_xml(source: &str) -> Result<Value, String> {
    let document = roxmltree::Document::parse(source).map_err(|e| io_error("Invalid XML", e))?;
    fn element(node: roxmltree::Node<'_, '_>, depth: usize) -> Result<Value, String> {
        if depth > 128 {
            return Err("XML nesting exceeds 128 levels.".into());
        }
        let mut object = Map::new();
        for attr in node.attributes() {
            object.insert(format!("@{}", attr.name()), json!(attr.value()));
        }
        let mut child_names = BTreeSet::new();
        for child in node.children().filter(|child| child.is_element()) {
            let name = child.tag_name().name().to_owned();
            let value = element(child, depth + 1)?;
            if child_names.contains(&name) {
                let current = object.get_mut(&name).ok_or("XML child grouping failed.")?;
                if let Some(array) = current.as_array_mut() {
                    array.push(value);
                } else {
                    *current = json!([current.clone(), value]);
                }
            } else {
                object.insert(name.clone(), value);
                child_names.insert(name);
            }
        }
        let text = node
            .children()
            .filter(|child| child.is_text())
            .filter_map(|child| child.text())
            .collect::<String>();
        let text = text.trim();
        if object.is_empty() {
            return Ok(json!(text));
        }
        if !text.is_empty() {
            object.insert("#text".into(), json!(text));
        }
        Ok(Value::Object(object))
    }
    let root = document.root_element();
    Ok(json!({root.tag_name().name():element(root, 0)?}))
}

fn parse_csv(source: &str) -> Result<Value, String> {
    let mut reader = csv::ReaderBuilder::new().from_reader(source.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| io_error("Invalid CSV header", e))?
        .clone();
    if headers.len() > 256 {
        return Err("CSV has more than 256 columns.".into());
    }
    let mut unique = BTreeSet::new();
    if headers.iter().any(|name| !unique.insert(name)) {
        return Err("CSV headers must be unique.".into());
    }
    let mut rows = Vec::new();
    for record in reader.records() {
        if rows.len() >= 10000 {
            return Err("CSV has more than 10000 rows.".into());
        }
        let record = record.map_err(|e| io_error("Invalid CSV row", e))?;
        let row: Map<String, Value> = headers
            .iter()
            .zip(record.iter())
            .map(|(key, value)| (key.to_owned(), json!(value)))
            .collect();
        rows.push(Value::Object(row));
    }
    Ok(Value::Array(rows))
}

fn selected<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value, String> {
    if pointer.is_empty() {
        return Ok(value);
    }
    if !pointer.starts_with('/') {
        return Err("'pointer' must be empty or start with '/'.".into());
    }
    value
        .pointer(pointer)
        .ok_or_else(|| format!("Pointer '{pointer}' was not found."))
}

fn bounded(value: Value) -> Result<Value, String> {
    if serde_json::to_vec(&value)
        .map_err(|e| io_error("Cannot encode result", e))?
        .len()
        > MAX_RESULT_BYTES
    {
        return Err("Result exceeds 1 MiB; select a narrower pointer.".into());
    }
    Ok(value)
}

fn query(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let (format, document) = load(path, optional_str(args, "format")?)?;
    let pointer = optional_str(args, "pointer")?.unwrap_or("");
    bounded(
        json!({"path":path,"format":format,"pointer":pointer,"value":selected(&document,pointer)?}),
    )
}

fn escaped(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn collect_changes(
    left: &Value,
    right: &Value,
    path: &str,
    depth: usize,
    changes: &mut Vec<Value>,
    total: &mut usize,
    limit: usize,
) -> Result<(), String> {
    if depth > 128 {
        return Err("Document nesting exceeds 128 levels.".into());
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let child_path = format!("{path}/{}", escaped(key));
                match (a.get(key), b.get(key)) {
                    (Some(av), Some(bv)) => {
                        collect_changes(av, bv, &child_path, depth + 1, changes, total, limit)?
                    }
                    (Some(av), None) => push_change(
                        changes,
                        total,
                        limit,
                        json!({"path":child_path,"kind":"removed","left":av}),
                    ),
                    (None, Some(bv)) => push_change(
                        changes,
                        total,
                        limit,
                        json!({"path":child_path,"kind":"added","right":bv}),
                    ),
                    _ => {}
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            for index in 0..a.len().max(b.len()) {
                let child_path = format!("{path}/{index}");
                match (a.get(index), b.get(index)) {
                    (Some(av), Some(bv)) => {
                        collect_changes(av, bv, &child_path, depth + 1, changes, total, limit)?
                    }
                    (Some(av), None) => push_change(
                        changes,
                        total,
                        limit,
                        json!({"path":child_path,"kind":"removed","left":av}),
                    ),
                    (None, Some(bv)) => push_change(
                        changes,
                        total,
                        limit,
                        json!({"path":child_path,"kind":"added","right":bv}),
                    ),
                    _ => {}
                }
            }
        }
        _ if left != right => push_change(
            changes,
            total,
            limit,
            json!({"path":path,"kind":"changed","left":left,"right":right}),
        ),
        _ => {}
    }
    Ok(())
}

fn push_change(changes: &mut Vec<Value>, total: &mut usize, limit: usize, change: Value) {
    *total += 1;
    if changes.len() < limit {
        changes.push(change);
    }
}

fn diff(args: &Value) -> Result<Value, String> {
    let left_path = Path::new(required_str(args, "leftPath")?);
    let right_path = Path::new(required_str(args, "rightPath")?);
    let format = optional_str(args, "format")?;
    let (left_format, left) = load(left_path, format)?;
    let (right_format, right) = load(right_path, format)?;
    if left_format != right_format {
        return Err("Input formats differ; pass an explicit common 'format'.".into());
    }
    let pointer = optional_str(args, "pointer")?.unwrap_or("");
    let left = selected(&left, pointer)?;
    let right = selected(&right, pointer)?;
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    let mut changes = Vec::new();
    let mut total = 0;
    collect_changes(left, right, pointer, 0, &mut changes, &mut total, limit)?;
    bounded(
        json!({"equal":total==0,"totalChanges":total,"limitReached":total>limit,"changes":changes,"pointer":pointer,"format":left_format}),
    )
}
