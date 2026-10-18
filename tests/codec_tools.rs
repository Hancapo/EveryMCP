mod common;

use common::{call, response};
use serde_json::json;
use std::fs;

#[test]
fn hash_compute_matches_reference_vectors_and_folds_case() {
    let fox = "The quick brown fox jumps over the lazy dog";
    for (algorithm, input, expected) in [
        ("joaat", "a", "0xCA2E9442"),
        ("joaat", fox, "0x519E91F5"),
        ("fnv1a_32", "a", "0xE40C292C"),
        ("fnv1_64", "a", "0xAF63BD4C8601B7BE"),
        ("murmur3_32", fox, "0x2E4FF723"),
        ("xxh64", "", "0xEF46DB3751D8E999"),
        ("djb2", "a", "0x0002B606"),
    ] {
        let result = call("hash_compute", json!({"algorithm":algorithm,"data":input}));
        assert_eq!(result["hex"], expected, "{algorithm}({input:?})");
    }
    let folded = call(
        "hash_compute",
        json!({"algorithm":"joaat","inputs":["A","a"],"caseFold":"lower"}),
    );
    assert_eq!(folded["results"][0]["hex"], folded["results"][1]["hex"]);
    assert_eq!(folded["results"][0]["signed"], "-902917054");
    let seeded = call(
        "hash_compute",
        json!({"algorithm":"murmur3_32","data":"","seed":"1"}),
    );
    assert_eq!(seeded["hex"], "0x514E28B7");
    let hex = call(
        "hash_compute",
        json!({"algorithm":"fnv1a_32","data":"61","encoding":"hex"}),
    );
    assert_eq!(hex["hex"], "0xE40C292C");
    let error = response(
        "hash_compute",
        json!({"algorithm":"fnv1a_32","data":"a","seed":"1"}),
    );
    assert_eq!(error["result"]["isError"], true);
}

#[test]
fn hash_dictionary_match_combines_affixes_and_reads_wordlists() {
    let temp = tempfile::tempdir().unwrap();
    let wordlist = temp.path().join("words.txt");
    fs::write(&wordlist, "zentorno\r\nadder\n\nt20\n").unwrap();
    let target = call(
        "hash_compute",
        json!({"algorithm":"joaat","data":"prop_adder_lod"}),
    );
    let signed = call("hash_compute", json!({"algorithm":"joaat","data":"t20"}))["signed"].clone();
    let result = call(
        "hash_dictionary_match",
        json!({"algorithm":"joaat","targets":[target["hex"],signed,"0x12345678"],
            "path":wordlist,"words":["ADDER"],"caseFold":"lower",
            "prefixes":["","prop_"],"suffixes":["","_lod"]}),
    );
    assert_eq!(result["matchedCount"], 2);
    assert_eq!(result["unmatched"], json!(["0x12345678"]));
    assert_eq!(result["complete"], true);
    assert_eq!(result["wordsRead"], 4);
    assert_eq!(result["candidatesTried"], 16);
    let matches = result["matches"].as_array().unwrap();
    let found: Vec<_> = matches
        .iter()
        .flat_map(|m| m["candidates"].as_array().unwrap())
        .collect();
    assert!(found.contains(&&json!("prop_ADDER_lod")));
    assert!(found.contains(&&json!("t20")));
    let capped = call(
        "hash_dictionary_match",
        json!({"algorithm":"joaat","targets":["0x1"],"words":["a","b","c"],"maxCandidates":2}),
    );
    assert_eq!(capped["maxCandidatesReached"], true);
    assert_eq!(capped["complete"], false);
}

#[test]
fn data_transform_round_trips_codecs_and_compression() {
    let result = call(
        "data_transform",
        json!({"data":"Hello, EveryMCP!","inputEncoding":"utf8","outputEncoding":"utf8","steps":[
            {"op":"xor","key":"5A A5"},{"op":"rol","bits":3},{"op":"add","value":200},
            {"op":"gzip_compress"},{"op":"base64_encode"},{"op":"base64_decode"},
            {"op":"gzip_decompress"},{"op":"sub","value":200},{"op":"ror","bits":3},
            {"op":"xor","key":"5AA5"},{"op":"zlib_compress"},{"op":"zlib_decompress"},
            {"op":"deflate_compress"},{"op":"deflate_decompress"},
            {"op":"lz4_frame_compress"},{"op":"lz4_frame_decompress"},
            {"op":"base32_encode"},{"op":"base32_decode"},{"op":"hex_encode"},{"op":"hex_decode"}
        ]}),
    );
    assert_eq!(result["output"], "Hello, EveryMCP!");
    assert_eq!(result["steps"].as_array().unwrap().len(), 20);
    let base32 = call(
        "data_transform",
        json!({"data":"foobar","inputEncoding":"utf8","outputEncoding":"utf8","steps":[{"op":"base32_encode"}]}),
    );
    assert_eq!(base32["output"], "MZXW6YTBOI======");
    let base64 = call(
        "data_transform",
        json!({"data":"fo","inputEncoding":"utf8","outputEncoding":"utf8","steps":[{"op":"base64_encode"}]}),
    );
    assert_eq!(base64["output"], "Zm8=");
    let block = call(
        "data_transform",
        json!({"data":"00".repeat(4096),"steps":[{"op":"lz4_block_compress"}]}),
    );
    let restored = call(
        "data_transform",
        json!({"data":block["output"],"steps":[{"op":"lz4_block_decompress","size":4096},{"op":"slice","offset":4090}]}),
    );
    assert_eq!(restored["output"], "00 00 00 00 00 00");
    assert_eq!(restored["sizeBytes"], 6);
}

#[test]
fn data_transform_reads_file_ranges_and_writes_output() {
    let temp = tempfile::tempdir().unwrap();
    let input = temp.path().join("blob.bin");
    fs::write(&input, b"junk\x13\x37\x13\x37tail").unwrap();
    let output = temp.path().join("out.bin");
    let result = call(
        "data_transform",
        json!({"path":input,"offset":4,"length":4,"steps":[{"op":"xor","key":"1337"},{"op":"not"},{"op":"reverse"}],
            "outputPath":output,"maxOutputBytes":2}),
    );
    assert_eq!(result["output"], "FF FF");
    assert_eq!(result["truncated"], true);
    assert_eq!(fs::read(&output).unwrap(), vec![0xff; 4]);
    let again = response(
        "data_transform",
        json!({"path":input,"steps":[{"op":"not"}],"outputPath":output}),
    );
    assert_eq!(again["result"]["isError"], true);
    let bomb = response(
        "data_transform",
        json!({"data":"00","steps":[{"op":"gzip_decompress"}]}),
    );
    assert_eq!(bomb["result"]["isError"], true);
}
