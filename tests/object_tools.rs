mod common;

use common::{call, response};
use serde_json::json;
use std::path::Path;

fn u16le(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u32le(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn u64le(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// A minimal static x86-64 ELF executable: one R-X segment, `.text` and `.shstrtab`.
fn elf64() -> Vec<u8> {
    let mut f = vec![0x7f, b'E', b'L', b'F', 2, 1, 1, 0];
    f.resize(16, 0);
    u16le(&mut f, 2); // ET_EXEC
    u16le(&mut f, 62); // EM_X86_64
    u32le(&mut f, 1);
    u64le(&mut f, 0x40_1078); // entry
    u64le(&mut f, 0x40); // program headers
    u64le(&mut f, 0x90); // section headers
    u32le(&mut f, 0);
    for v in [64, 56, 1, 64, 3, 2] {
        u16le(&mut f, v);
    }
    u32le(&mut f, 1); // PT_LOAD
    u32le(&mut f, 5); // R+X
    for v in [0, 0x40_1000, 0x40_1000, 0x8c, 0x8c, 0x1000] {
        u64le(&mut f, v);
    }
    f.extend_from_slice(&[0x31, 0xC0, 0xC3]); // .text at 0x78
    f.extend_from_slice(b"\0.text\0.shstrtab\0"); // .shstrtab at 0x7b
    f.resize(0x90, 0);
    f.extend_from_slice(&[0; 64]);
    for (name, kind, flags, addr, offset, size) in
        [(1, 1, 6, 0x40_1078, 0x78, 3), (7, 3, 0, 0, 0x7b, 17)]
    {
        u32le(&mut f, name);
        u32le(&mut f, kind);
        for v in [flags, addr, offset, size] {
            u64le(&mut f, v);
        }
        u32le(&mut f, 0);
        u32le(&mut f, 0);
        u64le(&mut f, 1);
        u64le(&mut f, 0);
    }
    f
}

/// A minimal x86-64 Mach-O executable with LC_UUID and one `__TEXT` segment.
fn macho64() -> Vec<u8> {
    let mut f = Vec::new();
    for v in [0xfeed_facf, 0x0100_0007, 3, 2, 2, 24 + 72, 0, 0] {
        u32le(&mut f, v);
    }
    u32le(&mut f, 0x1b); // LC_UUID
    u32le(&mut f, 24);
    f.extend(0u8..16);
    u32le(&mut f, 0x19); // LC_SEGMENT_64
    u32le(&mut f, 72);
    f.extend_from_slice(b"__TEXT\0\0\0\0\0\0\0\0\0\0");
    for v in [0x1_0000_0000, 0x1000, 0, 128] {
        u64le(&mut f, v);
    }
    for v in [5, 5, 0, 0] {
        u32le(&mut f, v);
    }
    f
}

/// Wraps a slice in a single-architecture universal (fat) header.
fn fat(slice: &[u8]) -> Vec<u8> {
    let mut f = Vec::new();
    for v in [0xcafe_babe, 1, 0x0100_0007, 3, 4096, slice.len() as u32, 12] {
        f.extend_from_slice(&u32::to_be_bytes(v));
    }
    f.resize(4096, 0);
    f.extend_from_slice(slice);
    f
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn elf_inspect_reads_headers_segments_and_sections() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "a.elf", &elf64());
    let result = call("elf_inspect", json!({"path":path}));
    assert_eq!(result["is64Bit"], true);
    assert_eq!(result["endian"], "little");
    assert_eq!(result["type"], "EXEC");
    assert_eq!(result["machine"], "X86_64");
    assert_eq!(result["entry"], "0x401078");
    assert_eq!(result["segments"][0]["type"], "PT_LOAD");
    assert_eq!(result["segments"][0]["flags"], "R-X");
    let names: Vec<_> = result["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["", ".text", ".shstrtab"]);
    assert_eq!(result["sections"][1]["sizeBytes"], 3);
    let limited = call("elf_inspect", json!({"path":path,"limit":1}));
    assert_eq!(limited["sections"].as_array().unwrap().len(), 1);
    assert_eq!(limited["truncated"], true);
}

#[test]
fn macho_inspect_reads_thin_and_fat_images() {
    let dir = tempfile::tempdir().unwrap();
    let thin = call(
        "macho_inspect",
        json!({"path":write(dir.path(), "thin", &macho64())}),
    );
    assert_eq!(thin["fat"], false);
    assert_eq!(thin["cpu"], "x86_64");
    assert_eq!(thin["fileType"], "EXECUTE");
    assert_eq!(thin["uuid"], "00010203-0405-0607-0809-0A0B0C0D0E0F");
    assert_eq!(thin["segments"][0]["name"], "__TEXT");
    assert_eq!(thin["segments"][0]["virtualAddress"], "0x100000000");
    let path = write(dir.path(), "fat", &fat(&macho64()));
    let universal = call("macho_inspect", json!({"path":path}));
    assert_eq!(universal["fat"], true);
    assert_eq!(universal["selectedArch"], 0);
    assert_eq!(universal["arches"][0]["offset"], 4096);
    assert_eq!(universal["uuid"], thin["uuid"]);
    assert_eq!(
        response("macho_inspect", json!({"path":path,"arch":1}))["result"]["isError"],
        true
    );
}

#[test]
fn object_tools_reject_other_formats() {
    let dir = tempfile::tempdir().unwrap();
    let text = write(dir.path(), "a.txt", b"not an executable image");
    for tool in [
        "elf_inspect",
        "macho_inspect",
        "pe_debug_info",
        "pe_resources",
        "pe_runtime_functions",
    ] {
        assert_eq!(
            response(tool, json!({"path":text}))["result"]["isError"],
            true,
            "{tool}"
        );
    }
    let elf = write(dir.path(), "a.elf", &elf64());
    assert_eq!(
        response("macho_inspect", json!({"path":elf}))["result"]["isError"],
        true
    );
}

#[cfg(windows)]
mod pe {
    use super::common::call;
    use serde_json::json;

    fn own_exe() -> &'static str {
        env!("CARGO_BIN_EXE_EveryMCP")
    }

    #[test]
    fn debug_info_builds_symbol_server_urls() {
        let result = call(
            "pe_debug_info",
            json!({"path":own_exe(),"symbolServer":"https://symbols.example/"}),
        );
        let codeview = &result["codeView"];
        assert_eq!(codeview["format"], "RSDS");
        let pdb = codeview["pdbName"].as_str().unwrap();
        assert!(pdb.to_lowercase().ends_with(".pdb"), "{pdb}");
        let key = codeview["symbolServerKey"].as_str().unwrap();
        assert_eq!(
            codeview["symbolServerUrl"],
            format!("https://symbols.example/{pdb}/{key}/{pdb}")
        );
        let guid = codeview["guid"].as_str().unwrap().replace('-', "");
        assert!(key.starts_with(&guid));
        assert!(
            result["debugEntries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["typeName"] == "codeview")
        );
    }

    #[test]
    fn runtime_functions_page_and_look_up_by_rva() {
        let first = call(
            "pe_runtime_functions",
            json!({"path":own_exe(),"limit":2,"includeUnwind":true}),
        );
        assert!(first["functionCount"].as_u64().unwrap() > 2);
        assert_eq!(first["nextOffset"], 2);
        let function = &first["functions"][0];
        assert_eq!(function["section"], ".text");
        assert!(function["unwind"]["codes"].is_array());
        let second = call(
            "pe_runtime_functions",
            json!({"path":own_exe(),"offset":1,"limit":1}),
        );
        assert_eq!(
            second["functions"][0]["begin"],
            first["functions"][1]["begin"]
        );
        let found = call(
            "pe_runtime_functions",
            json!({"path":own_exe(),"rva":function["begin"]}),
        );
        assert_eq!(found["function"]["end"], function["end"]);
    }

    #[test]
    fn resources_decode_version_info() {
        let path = std::env::var("SystemRoot").unwrap() + r"\System32\kernel32.dll";
        let result = call("pe_resources", json!({"path":path}));
        assert!(
            result["resources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["type"] == "RT_VERSION")
        );
        let version = &result["versionInfo"];
        assert_eq!(version["strings"]["InternalName"], "kernel32");
        let fixed = version["fixed"]["fileVersion"].as_str().unwrap();
        assert!(
            version["strings"]["FileVersion"]
                .as_str()
                .unwrap()
                .starts_with(fixed)
        );
    }
}
