mod common;

use common::{call, response};
use serde_json::json;

#[test]
fn disasm_decodes_branches_and_rip_targets() {
    let result = call(
        "x86_disasm",
        json!({"code":"48 89 5C 24 08 E8 00 00 00 00 48 8B 05 10 00 00 00 C3","address":"0x1000"}),
    );
    assert_eq!(result["instructionCount"], 4);
    assert_eq!(result["remainingBytes"], 0);
    let rows = result["instructions"].as_array().unwrap();
    assert_eq!(rows[0]["text"], "mov [rsp+8],rbx");
    assert_eq!(rows[1]["flowControl"], "call");
    assert_eq!(rows[1]["branchTarget"], "0x100a");
    assert_eq!(rows[2]["ripTarget"], "0x1021");
    assert_eq!(rows[3]["mnemonic"], "ret");
    let gas = call(
        "x86_disasm",
        json!({"code":"55 89 E5","bits":32,"syntax":"gas"}),
    );
    assert_eq!(gas["instructions"][1]["text"], "mov %esp,%ebp");
    let limited = call("x86_disasm", json!({"code":"90 90 90","limit":2}));
    assert_eq!(limited["instructionCount"], 2);
    assert_eq!(limited["remainingBytes"], 1);
}

#[test]
fn disasm_rejects_bad_arguments() {
    for args in [
        json!({"code":"90","bits":8}),
        json!({"code":"90","syntax":"pretty"}),
        json!({"code":""}),
        json!({"code":"zz"}),
    ] {
        assert_eq!(response("x86_disasm", args)["result"]["isError"], true);
    }
}

#[test]
fn assemble_relocates_external_branches_and_keeps_internal_ones() {
    // `call 0x0` encoded at 0x1000 must be re-targeted when moved to 0x2000.
    let moved = call(
        "x86_assemble",
        json!({"code":"E8 FB EF FF FF","sourceAddress":"0x1000","targetAddress":"0x2000"}),
    );
    assert_eq!(moved["bytes"], "E8 FB DF FF FF");
    assert_eq!(moved["rewritten"], true);
    // Branches into the block move with it.
    let internal = call(
        "x86_assemble",
        json!({"code":"E8 00 00 00 00 EB FE","sourceAddress":"0x1000","targetAddress":"0x2000"}),
    );
    assert_eq!(internal["bytes"], "E8 00 00 00 00 EB FE");
    assert_eq!(
        response("x86_assemble", json!({"code":"C4"}))["result"]["isError"],
        true
    );
}

#[test]
fn signature_generate_wildcards_relative_operands_until_unique() {
    let code = "55 48 89 E5 E8 11 22 33 44 C3 55 48 89 E5 E8 55 66 77 88 90 C3";
    let result = call("signature_generate", json!({"code":code,"offset":10}));
    assert_eq!(result["pattern"], "55 48 89 E5 E8 ?? ?? ?? ?? 90");
    assert_eq!(result["idaPattern"], "55 48 89 E5 E8 ? ? ? ? 90");
    assert_eq!(result["mask"], "xxxxx????x");
    assert_eq!(result["unique"], true);
    let exact = call(
        "signature_generate",
        json!({"code":code,"offset":10,"exactOperands":true}),
    );
    assert_eq!(exact["pattern"], "55 48 89 E5 E8 55 66 77 88");
    assert_eq!(exact["wildcardCount"], 0);
    assert_eq!(
        response("signature_generate", json!({"code":code,"offset":21}))["result"]["isError"],
        true
    );
}

#[test]
fn signature_generate_rejects_ambiguous_sources() {
    for args in [
        json!({}),
        json!({"code":"90","path":"x"}),
        json!({"code":"90","rva":"0x1000"}),
        json!({"code":"90","maxBytes":0}),
    ] {
        assert_eq!(
            response("signature_generate", args)["result"]["isError"],
            true
        );
    }
}

#[cfg(windows)]
#[test]
fn signature_generate_by_rva_is_unique_in_the_whole_file() {
    let path = env!("CARGO_BIN_EXE_EveryMCP");
    let function = call(
        "pe_runtime_functions",
        json!({"path":path,"offset":100,"limit":1}),
    )["functions"][0]
        .clone();
    let by_rva = call(
        "signature_generate",
        json!({"path":path,"rva":function["begin"]}),
    );
    assert_eq!(by_rva["offset"], function["fileOffset"]);
    assert_eq!(
        by_rva["searchedBytes"],
        std::fs::metadata(path).unwrap().len()
    );
    assert_eq!(by_rva["unique"], by_rva["matchCount"] == 1);
    let by_offset = call(
        "signature_generate",
        json!({"path":path,"offset":function["fileOffset"]}),
    );
    assert_eq!(by_offset["pattern"], by_rva["pattern"]);
    if by_rva["unique"] == true {
        let found = call(
            "binary_pattern_search",
            json!({"path":path,"pattern":by_rva["pattern"]}),
        );
        assert_eq!(found["offsets"], json!([function["fileOffset"]]));
    }
}
