mod common;

use common::{call, response};
use serde_json::json;
use std::fs;
use std::path::Path;

#[test]
fn strings_cross_read_boundaries_and_continue_at_the_next_match() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("strings.bin");
    let mut data = vec![0; 65533];
    data.extend_from_slice(b"BoundaryString\0SecondString\0");
    fs::write(&path, data).unwrap();
    let first = call(
        "binary_strings",
        json!({"path":path,"encoding":"ascii","limit":1}),
    );
    assert_eq!(first["strings"][0]["offset"], 65533);
    assert_eq!(first["strings"][0]["text"], "BoundaryString");
    assert_eq!(first["nextOffset"], 65548);
    let second = call(
        "binary_strings",
        json!({"path":path,"encoding":"ascii","offset":first["nextOffset"],"limit":1}),
    );
    assert_eq!(second["strings"].as_array().unwrap().len(), 1);
    assert_eq!(second["strings"][0]["text"], "SecondString");
    assert!(second["nextOffset"].is_null());
}

#[test]
fn strings_decode_both_utf16_orders_at_odd_offsets_and_filter_unicode() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("wide.bin");
    for (encoding, little) in [("utf16le", true), ("utf16be", false)] {
        let mut data = vec![0; 65535];
        for word in "Hello 😀 café".encode_utf16() {
            data.extend_from_slice(&if little {
                word.to_le_bytes()
            } else {
                word.to_be_bytes()
            });
        }
        data.extend_from_slice(&[0; 4]);
        fs::write(&path, data).unwrap();
        let result = call(
            "binary_strings",
            json!({"path":path,"encoding":encoding,"filter":"😀"}),
        );
        assert_eq!(result["strings"].as_array().unwrap().len(), 1);
        assert_eq!(result["strings"][0]["offset"], 65535);
        assert_eq!(result["strings"][0]["text"], "Hello 😀 café");
        assert_eq!(result["strings"][0]["length"], 12);
        assert_eq!(result["strings"][0]["byteLength"], 26);
    }
}

#[test]
fn long_string_previews_are_bounded_and_filter_checks_the_whole_run() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("long.bin");
    let text = format!("{}Needle", "x".repeat(70000));
    fs::write(&path, text).unwrap();
    let result = call(
        "binary_strings",
        json!({"path":path,"encoding":"ascii","filter":"Needle"}),
    );
    assert_eq!(result["strings"][0]["length"], 70006);
    assert_eq!(result["strings"][0]["text"].as_str().unwrap().len(), 256);
    assert_eq!(result["strings"][0]["truncated"], true);
    assert!(result["nextOffset"].is_null());
}

#[test]
fn all_encodings_have_stable_byte_only_pagination_without_duplicate_starts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mixed.bin");
    let mut data = b"FirstString\0\0".to_vec();
    for word in "WideString".encode_utf16() {
        data.extend_from_slice(&word.to_le_bytes());
    }
    data.extend_from_slice(b"\0\0\0FinalString\0");
    fs::write(&path, data).unwrap();
    let all = call("binary_strings", json!({"path":path}));
    let mut matches = Vec::new();
    let mut offset = 0;
    loop {
        let page = call(
            "binary_strings",
            json!({"path":path,"offset":offset,"limit":1}),
        );
        matches.extend_from_slice(page["strings"].as_array().unwrap());
        let Some(next) = page["nextOffset"].as_u64() else {
            break;
        };
        assert!(next > offset);
        offset = next;
        assert!(matches.len() < 20);
    }
    assert_eq!(matches, *all["strings"].as_array().unwrap());
    assert!(
        matches
            .iter()
            .any(|m| m["text"] == "WideString" && m["encoding"] == "utf16le")
    );
    assert!(
        matches
            .windows(2)
            .all(|pair| pair[0]["offset"].as_u64() < pair[1]["offset"].as_u64())
    );
}

#[test]
fn entropy_has_known_zero_and_uniform_values_and_byte_pagination() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("entropy.bin");
    let mut data = vec![0; 256];
    data.extend(0..=255);
    data.extend_from_slice(&[7, 7, 7]);
    fs::write(&path, data).unwrap();
    let first = call(
        "binary_entropy",
        json!({"path":path,"blockSize":256,"limit":1}),
    );
    assert_eq!(first["blocks"][0]["entropyBitsPerByte"], 0.0);
    assert_eq!(first["nextOffset"], 256);
    let second = call(
        "binary_entropy",
        json!({"path":path,"blockSize":256,"offset":256}),
    );
    assert_eq!(second["blocks"][0]["entropyBitsPerByte"], 8.0);
    assert_eq!(second["blocks"][1]["offset"], 512);
    assert_eq!(second["blocks"][1]["sizeBytes"], 3);
    assert_eq!(second["blocks"][1]["entropyBitsPerByte"], 0.0);
    assert!(second["nextOffset"].is_null());
}

fn write_pe(path: &Path, machine: u16, imports: &[&str]) {
    let mut bytes = vec![0u8; 0x1200];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
    bytes[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
    bytes[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
    bytes[0x96..0x98].copy_from_slice(&0x2022u16.to_le_bytes());
    bytes[0x98..0x9a].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[0xb0..0xb8].copy_from_slice(&0x140000000u64.to_le_bytes());
    for (offset, value) in [
        (0xb8, 0x1000u32),
        (0xbc, 0x200),
        (0xd0, 0x2000),
        (0xd4, 0x200),
        (0x104, 16),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    let section = 0x188;
    bytes[section..section + 6].copy_from_slice(b".idata");
    for (offset, value) in [
        (section + 8, 0x1000u32),
        (section + 12, 0x1000),
        (section + 16, 0x1000),
        (section + 20, 0x200),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    if !imports.is_empty() {
        bytes[0x110..0x114].copy_from_slice(&0x1000u32.to_le_bytes());
        bytes[0x114..0x118].copy_from_slice(&((imports.len() as u32 + 1) * 20).to_le_bytes());
        for (index, name) in imports.iter().enumerate() {
            let descriptor = 0x200 + index * 20;
            let name_offset = 0x600 + index * 128;
            let thunk_offset = 0x400 + index * 16;
            let name_rva = (name_offset + 0xe00) as u32;
            let thunk_rva = (thunk_offset + 0xe00) as u32;
            bytes[descriptor..descriptor + 4].copy_from_slice(&thunk_rva.to_le_bytes());
            bytes[descriptor + 12..descriptor + 16].copy_from_slice(&name_rva.to_le_bytes());
            bytes[descriptor + 16..descriptor + 20].copy_from_slice(&thunk_rva.to_le_bytes());
            bytes[thunk_offset..thunk_offset + 8]
                .copy_from_slice(&0x8000000000000001u64.to_le_bytes());
            bytes[name_offset..name_offset + name.len()].copy_from_slice(name.as_bytes());
        }
    }
    fs::write(path, bytes).unwrap();
}

#[test]
fn pe_graph_reports_missing_cycles_mismatches_and_api_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root.exe");
    let libraries = temp.path().join("libraries");
    fs::create_dir(&libraries).unwrap();
    write_pe(
        &root,
        0x8664,
        &["a.dll", "missing.dll", "wrong.dll", "api-ms-win-test.dll"],
    );
    write_pe(&libraries.join("a.dll"), 0x8664, &["root.exe"]);
    write_pe(&libraries.join("wrong.dll"), 0x14c, &[]);
    let result = call(
        "pe_dependencies",
        json!({"path":root,"searchPaths":[libraries, temp.path()]}),
    );
    let edges = result["dependencies"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e["name"] == "missing.dll" && e["status"] == "missing")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["name"] == "wrong.dll" && e["status"] == "architectureMismatch")
    );
    assert!(
        edges
            .iter()
            .any(|e| e["name"] == "root.exe" && e["cycle"] == true)
    );
    assert_eq!(result["apiSetContracts"][0]["name"], "api-ms-win-test.dll");
    assert_eq!(result["apiSetContracts"][0]["status"], "unresolved");
    assert_eq!(result["limitReached"], false);
}

#[test]
fn pe_graph_uses_only_explicit_roots_and_reports_ambiguous_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root.exe");
    let left = temp.path().join("left");
    let right = temp.path().join("right");
    fs::create_dir(&left).unwrap();
    fs::create_dir(&right).unwrap();
    write_pe(&root, 0x8664, &["shared.dll", "kernel32.dll"]);
    write_pe(&left.join("shared.dll"), 0x8664, &[]);
    write_pe(&right.join("shared.dll"), 0x8664, &[]);
    let no_paths = call("pe_dependencies", json!({"path":root,"searchPaths":[]}));
    assert!(
        no_paths["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["status"] == "missing")
    );
    let result = call(
        "pe_dependencies",
        json!({"path":root,"searchPaths":[right,left]}),
    );
    let edge = result["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["name"] == "shared.dll")
        .unwrap();
    assert_eq!(edge["status"], "ambiguous");
    assert_eq!(edge["candidates"].as_array().unwrap().len(), 2);
    assert!(edge["resolvedPath"].is_null());
    assert_eq!(result["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn pe_graph_honors_recursion_and_shared_edge_limits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root.exe");
    write_pe(&root, 0x8664, &["a.dll", "api-ms-win-test.dll", "b.dll"]);
    write_pe(&temp.path().join("a.dll"), 0x8664, &["child.dll"]);
    let direct = call(
        "pe_dependencies",
        json!({"path":root,"searchPaths":[],"recursive":false}),
    );
    assert!(
        !direct["dependencies"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "child.dll")
    );
    let bounded = call(
        "pe_dependencies",
        json!({"path":root,"searchPaths":[],"limit":2}),
    );
    assert_eq!(
        bounded["dependencies"].as_array().unwrap().len()
            + bounded["apiSetContracts"].as_array().unwrap().len(),
        2
    );
    assert_eq!(bounded["limitReached"], true);
}

#[test]
fn binary_tools_reject_invalid_bounds_and_missing_explicit_search_paths() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("empty.bin");
    fs::write(&path, []).unwrap();
    for (name, args) in [
        ("binary_strings", json!({"path":path,"limit":0})),
        ("binary_strings", json!({"path":path,"encoding":"utf8"})),
        ("binary_strings", json!({"path":path,"minLength":257})),
        ("binary_entropy", json!({"path":path,"blockSize":0})),
        ("binary_entropy", json!({"path":path,"offset":1})),
        ("pe_dependencies", json!({"path":path})),
    ] {
        assert_eq!(response(name, args)["result"]["isError"], true);
    }
}
