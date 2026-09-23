use super::{data, optional_str, optional_u64, required_str};
use regex::Regex;
use serde_json::{Value, json};
use std::path::Path;

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "diagnostics_parse" => diagnostics(args),
        "test_results_parse" => tests(args),
        _ => Err(format!("Unknown report tool '{name}'.")),
    }
}

fn source(args: &Value) -> Result<String, String> {
    match (optional_str(args, "text")?, optional_str(args, "path")?) {
        (Some(text), None) => Ok(text.to_owned()),
        (None, Some(path)) => data::read_text(Path::new(path)),
        _ => Err("Pass exactly one of 'text' or 'path'.".into()),
    }
}

fn diagnostics(args: &Value) -> Result<Value, String> {
    let source = source(args)?;
    if source.len() > 8 * 1024 * 1024 {
        return Err("Diagnostic input exceeds 8 MiB.".into());
    }
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    let msvc = Regex::new(r"^(?P<file>.+?)\((?P<line>\d+)(?:,(?P<column>\d+))?\):\s*(?P<severity>fatal error|error|warning|note)\s*(?P<code>[A-Za-z]+\d+)?\s*:\s*(?P<message>.+)$").unwrap();
    let gcc = Regex::new(r"^(?P<file>.+?):(?P<line>\d+)(?::(?P<column>\d+))?:\s*(?P<severity>fatal error|error|warning|note):\s*(?P<message>.+)$").unwrap();
    let cmake = Regex::new(r"^CMake (?P<severity>Error|Warning) at (?P<file>.+?):(?P<line>\d+)(?: \([^)]+\))?:\s*(?P<message>.+)$").unwrap();
    let rust_head =
        Regex::new(r"^(?P<severity>error|warning)(?:\[(?P<code>[^]]+)\])?:\s*(?P<message>.+)$")
            .unwrap();
    let rust_location =
        Regex::new(r"^\s*-->\s*(?P<file>.+?):(?P<line>\d+):(?P<column>\d+)\s*$").unwrap();
    let mut diagnostics = Vec::new();
    let mut total = 0usize;
    let mut pending_rust: Option<(String, Option<String>, String)> = None;
    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        let mut item = None;
        if let Some(caps) = msvc
            .captures(line)
            .or_else(|| gcc.captures(line))
            .or_else(|| cmake.captures(line))
        {
            let severity = caps
                .name("severity")
                .map(|m| m.as_str().to_ascii_lowercase())
                .unwrap_or_default();
            let line_number = caps
                .name("line")
                .and_then(|m| m.as_str().parse::<u64>().ok());
            let column = caps
                .name("column")
                .and_then(|m| m.as_str().parse::<u64>().ok());
            item = Some(
                json!({"file":caps.name("file").map(|m|m.as_str()),"line":line_number,
                "column":column,"severity":severity,"code":caps.name("code").map(|m|m.as_str()),
                "message":caps.name("message").map(|m|m.as_str().trim()),"raw":line.chars().take(4096).collect::<String>()}),
            );
            pending_rust = None;
        } else if let Some(caps) = rust_location.captures(line) {
            if let Some((severity, code, message)) = pending_rust.take() {
                item = Some(json!({"file":caps.name("file").map(|m|m.as_str()),
                    "line":caps.name("line").and_then(|m|m.as_str().parse::<u64>().ok()),
                    "column":caps.name("column").and_then(|m|m.as_str().parse::<u64>().ok()),
                    "severity":severity,"code":code,"message":message,"raw":line.chars().take(4096).collect::<String>()}));
            }
        } else if let Some(caps) = rust_head.captures(line) {
            if let Some((severity, code, message)) = pending_rust.take() {
                total += 1;
                if diagnostics.len() < limit {
                    diagnostics.push(json!({"file":null,"line":null,"column":null,"severity":severity,"code":code,"message":message}));
                }
            }
            pending_rust = Some((
                caps["severity"].to_owned(),
                caps.name("code").map(|m| m.as_str().to_owned()),
                caps["message"].to_owned(),
            ));
        }
        if let Some(item) = item {
            total += 1;
            if diagnostics.len() < limit {
                diagnostics.push(item);
            }
        }
    }
    if let Some((severity, code, message)) = pending_rust {
        total += 1;
        if diagnostics.len() < limit {
            diagnostics.push(json!({"file":null,"line":null,"column":null,"severity":severity,"code":code,"message":message}));
        }
    }
    Ok(json!({"diagnostics":diagnostics,"totalMatched":total,"limitReached":total>limit}))
}

fn tests(args: &Value) -> Result<Value, String> {
    let path = Path::new(required_str(args, "path")?);
    let format = optional_str(args, "format")?
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| {
            match path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "trx" => "trx",
                "tap" => "tap",
                _ => "junit",
            }
            .to_owned()
        });
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    let text = data::read_text(path)?;
    match format.as_str() {
        "junit" | "trx" => xml_results(&text, &format, limit),
        "tap" => tap_results(&text, limit),
        _ => Err("'format' must be junit, trx or tap.".into()),
    }
}

fn xml_results(text: &str, format: &str, limit: usize) -> Result<Value, String> {
    let document =
        roxmltree::Document::parse(text).map_err(|e| format!("Invalid test result XML: {e}"))?;
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();
    if format == "junit" {
        for case in document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "testcase")
        {
            total += 1;
            let name = case.attribute("name").unwrap_or("");
            let class_name = case.attribute("classname");
            if case
                .children()
                .any(|child| child.is_element() && child.tag_name().name() == "skipped")
            {
                skipped += 1;
            } else if let Some(issue) = case.children().find(|child| {
                child.is_element() && matches!(child.tag_name().name(), "failure" | "error")
            }) {
                failed += 1;
                if failures.len() < limit {
                    failures.push(json!({"name":name,"className":class_name,"message":issue.attribute("message").or_else(||issue.text()).unwrap_or("")}));
                }
            } else {
                passed += 1;
            }
        }
    } else {
        for case in document
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "UnitTestResult")
        {
            total += 1;
            let name = case.attribute("testName").unwrap_or("");
            match case
                .attribute("outcome")
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "passed" => passed += 1,
                "failed" | "error" => {
                    failed += 1;
                    let message = case
                        .descendants()
                        .find(|child| child.is_element() && child.tag_name().name() == "Message")
                        .and_then(|child| child.text())
                        .unwrap_or("");
                    if failures.len() < limit {
                        failures.push(json!({"name":name,"message":message}));
                    }
                }
                _ => skipped += 1,
            }
        }
    }
    Ok(
        json!({"format":format,"total":total,"passed":passed,"failed":failed,"skipped":skipped,"failures":failures,"limitReached":failed>limit}),
    )
}

fn tap_results(text: &str, limit: usize) -> Result<Value, String> {
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        let (ok, rest) = if let Some(rest) = line.strip_prefix("not ok") {
            (false, rest)
        } else if let Some(rest) = line.strip_prefix("ok") {
            (true, rest)
        } else {
            continue;
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            continue;
        }
        total += 1;
        if line.contains("# SKIP") {
            skipped += 1;
        } else if ok {
            passed += 1;
        } else {
            failed += 1;
            if failures.len() < limit {
                failures.push(json!({"name":rest.trim().trim_start_matches(char::is_numeric).trim_start_matches('-').trim(),"message":line}));
            }
        }
    }
    Ok(
        json!({"format":"tap","total":total,"passed":passed,"failed":failed,"skipped":skipped,"failures":failures,"limitReached":failed>limit}),
    )
}
