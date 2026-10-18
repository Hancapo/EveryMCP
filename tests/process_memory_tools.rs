#![cfg(windows)]

mod common;

use common::{call, response};
use serde_json::json;
use std::hint::black_box;

static MARKER: [u8; 16] = *b"\xE7\x9AEveryMCP-mark\x01";
static WORDS: [u32; 4] = [1, 2, 0xdead_beef, 4];

fn pid() -> u32 {
    std::process::id()
}

fn module_name() -> String {
    std::env::current_exe()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn memory_read_decodes_typed_values_and_reports_regions() {
    let address = format!("0x{:X}", black_box(&WORDS).as_ptr() as usize);
    let result = call(
        "process_memory_read",
        json!({"pid":pid(),"address":address,"size":16,"type":"u32"}),
    );
    assert_eq!(result["values"], json!(["1", "2", "3735928559", "4"]));
    assert_eq!(
        result["hex"],
        "01 00 00 00 02 00 00 00 EF BE AD DE 04 00 00 00"
    );
    assert_eq!(result["pointerSize"], 8);
    assert_eq!(result["region"]["type"], "MEM_IMAGE");
    let relative = result["module"].as_str().unwrap();
    assert!(
        relative.starts_with(&format!("{}+0x", module_name())),
        "{relative}"
    );
    let by_module = call(
        "process_memory_read",
        json!({"pid":pid(),"address":relative,"size":4,"type":"u32","count":1}),
    );
    assert_eq!(by_module["address"], address);
    assert_eq!(by_module["values"], json!(["1"]));

    let unreadable = response(
        "process_memory_read",
        json!({"pid":pid(),"address":"0x10","size":32}),
    );
    assert_eq!(unreadable["result"]["isError"], true);
    let partial = call(
        "process_memory_read",
        json!({"pid":pid(),"address":"0x10","size":32,"allowPartial":true}),
    );
    assert_eq!(partial["unreadableBytes"], 32);
    assert_eq!(partial["unreadableRanges"][0]["address"], "0x10");
}

#[test]
fn pointer_chain_follows_heap_hops() {
    let value = Box::new(0x1122_3344_5566_7788u64);
    let table = Box::new([0usize, &*value as *const u64 as usize, 0, 0]);
    let root = Box::new(&*table as *const [usize; 4] as usize);
    let base = format!("0x{:X}", &*root as *const usize as usize);
    let result = call(
        "pointer_chain_resolve",
        json!({"pid":pid(),"base":base,"offsets":["0x8","0"],"type":"u64"}),
    );
    black_box((&value, &table, &root));
    assert_eq!(result["resolved"], true, "{result}");
    assert_eq!(result["hops"].as_array().unwrap().len(), 2);
    assert_eq!(
        result["address"],
        format!("0x{:X}", &*value as *const u64 as usize)
    );
    assert_eq!(result["value"], "1234605616436508552");

    let broken = call(
        "pointer_chain_resolve",
        json!({"pid":pid(),"base":base,"offsets":["0x10","0"]}),
    );
    assert_eq!(broken["resolved"], false);
    assert_eq!(broken["failure"]["reason"], "null pointer");
}

#[test]
fn pattern_scan_finds_module_bytes_with_wildcards_and_resumes() {
    let marker = black_box(&MARKER).as_ptr() as usize;
    let pattern = "E7 9A 45 76 65 72 79 4D 43 50 2D ?? 61 72 6B 01";
    let result = call(
        "process_pattern_scan",
        json!({"pid":pid(),"module":module_name(),"pattern":pattern}),
    );
    let addresses: Vec<_> = result["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["address"].as_str().unwrap().to_owned())
        .collect();
    assert!(addresses.contains(&format!("0x{marker:X}")), "{result}");
    assert_eq!(result["limitReached"], false);

    let range = call(
        "process_pattern_scan",
        json!({"pid":pid(),"address":format!("0x{:X}", marker - 3),"size":19,"pattern":"45 76 65"}),
    );
    assert_eq!(
        range["matches"][0]["address"],
        format!("0x{:X}", marker + 2)
    );

    let first = call(
        "process_pattern_scan",
        json!({"pid":pid(),"address":format!("0x{marker:X}"),"size":16,"pattern":"?? 6D 61","limit":1}),
    );
    assert_eq!(first["matches"].as_array().unwrap().len(), 1);
    assert!(first["nextAddress"].is_null());
    let none = response(
        "process_pattern_scan",
        json!({"pid":pid(),"pattern":"?? ??"}),
    );
    assert_eq!(none["result"]["isError"], true);
}
