mod common;
use common::{call, response};
use serde_json::json;
use std::fs::{self, File};
use std::io::Write;

#[test]
fn json_patch_applies_all_operations_atomically() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("data.json");
    fs::write(&path, r#"{"items":[1,2],"a/b":{"~key":"old"}}"#).unwrap();
    call(
        "json_patch",
        json!({"path":path,"operations":[
            {"op":"test","path":"/items/0","value":1},
            {"op":"add","path":"/items/-","value":3},
            {"op":"replace","path":"/a~1b/~0key","value":"new"},
            {"op":"copy","from":"/items/1","path":"/copy"},
            {"op":"move","from":"/copy","path":"/moved"},
            {"op":"remove","path":"/items/0"}
        ]}),
    );
    let actual: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        actual,
        json!({"items":[2,3],"a/b":{"~key":"new"},"moved":2})
    );
    let before = fs::read(&path).unwrap();
    let failed = response(
        "json_patch",
        json!({"path":path,"operations":[
        {"op":"replace","path":"/moved","value":9}, {"op":"test","path":"/moved","value":5}]}),
    );
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn json_patch_preserves_untouched_precise_numbers() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("precise.json");
    fs::write(
        &path,
        r#"{"id":18446744073709551617,"decimal":0.12345678901234567890123456789,"enabled":false}"#,
    )
    .unwrap();
    call(
        "json_patch",
        json!({"path":path,"operations":[
            {"op":"replace","path":"/enabled","value":true}
        ]}),
    );
    let actual = fs::read_to_string(&path).unwrap();
    assert!(actual.contains("18446744073709551617"), "{actual}");
    assert!(
        actual.contains("0.12345678901234567890123456789"),
        "{actual}"
    );
    assert!(actual.contains("\"enabled\": true"));
}

#[test]
fn json_patch_compares_numbers_exactly_in_test_operations() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("numbers.json");
    fs::write(
        &path,
        r#"{"values":[1,{"n":1000},-0.0],"id":18446744073709551617}"#,
    )
    .unwrap();
    call(
        "json_patch",
        json!({"path":path,"operations":[
            {"op":"test","path":"/values","value":[1.0,{"n":1e3},0]}
        ]}),
    );
    let before = fs::read(&path).unwrap();
    let different: serde_json::Value = serde_json::from_str("18446744073709551618").unwrap();
    let failed = response(
        "json_patch",
        json!({"path":path,"operations":[
            {"op":"test","path":"/id","value":different}
        ]}),
    );
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn json_patch_rejects_reserved_number_keys_without_modifying_source() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("reserved.json");
    for source in [
        r#"{"value":{"$serde_json::private::Number":"42"},"enabled":false}"#,
        r#"{"value":{"\u0024serde_json::private::Number":"42"},"enabled":false}"#,
    ] {
        fs::write(&path, source).unwrap();
        let failed = response(
            "json_patch",
            json!({"path":path,"operations":[
                {"op":"replace","path":"/enabled","value":true}
            ]}),
        );
        assert_eq!(failed["result"]["isError"], true, "{failed}");
        assert_eq!(fs::read_to_string(&path).unwrap(), source);
    }
}

#[test]
fn archive_test_detects_corrupt_entry_without_extracting() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("a.zip");
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    zip.start_file(
        "a",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )
    .unwrap();
    zip.write_all(b"unique-payload").unwrap();
    zip.finish().unwrap();
    assert_eq!(call("archive_test", json!({"archive":path}))["valid"], true);
    let mut bytes = fs::read(&path).unwrap();
    let offset = bytes
        .windows(b"unique-payload".len())
        .position(|b| b == b"unique-payload")
        .unwrap();
    bytes[offset] ^= 1;
    fs::write(&path, bytes).unwrap();
    let result = call("archive_test", json!({"archive":path}));
    assert_eq!(result["valid"], false);
    assert_eq!(result["failedEntries"], 1);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
}

#[test]
fn file_and_archive_pages_have_no_gaps_or_duplicates() {
    let tmp = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        fs::write(tmp.path().join(name), name).unwrap();
    }
    for tool in ["directory_manifest", "file_find"] {
        let first = call(tool, json!({"root":tmp.path(),"limit":2}));
        assert_eq!(first["nextOffset"], 2);
        let last = call(tool, json!({"root":tmp.path(),"limit":2,"offset":2}));
        assert!(last["nextOffset"].is_null());
        let paths: Vec<_> = first["entries"]
            .as_array()
            .unwrap()
            .iter()
            .chain(last["entries"].as_array().unwrap())
            .map(|e| e["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths.len(), 3);
        assert_ne!(paths[0], paths[2]);
    }
    let archive = tmp.path().join("a.zip");
    call(
        "archive_create",
        json!({"archive":archive,"paths":[tmp.path().join("a"),tmp.path().join("b"),tmp.path().join("c")]}),
    );
    let first = call("archive_inspect", json!({"archive":archive,"limit":2}));
    let last = call(
        "archive_inspect",
        json!({"archive":archive,"limit":2,"offset":first["nextOffset"]}),
    );
    assert_eq!(last["entries"][0]["name"], "c");
    assert!(last["nextOffset"].is_null());
}

#[test]
fn pattern_pages_preserve_overlapping_matches() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bytes");
    fs::write(&path, b"AAAA").unwrap();
    let first = call(
        "binary_pattern_search",
        json!({"path":path,"pattern":"41 41","limit":2}),
    );
    assert_eq!(first["offsets"], json!([0, 1]));
    let last = call(
        "binary_pattern_search",
        json!({"path":path,"pattern":"41 41","limit":2,"startOffset":first["nextOffset"]}),
    );
    assert_eq!(last["offsets"], json!([2]));
    assert!(last["nextOffset"].is_null());
}

#[test]
fn archive_pagination_keeps_entries_after_name_byte_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("long.zip");
    let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
    let mut names = Vec::new();
    for i in 0..14 {
        names.push(format!("{i:02}{}", "a".repeat(39998)));
    }
    names.push("z".into());
    for name in &names {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
    }
    zip.finish().unwrap();
    let first = call("archive_inspect", json!({"archive":path,"limit":100}));
    assert_eq!(first["nextOffset"], 13);
    let last = call(
        "archive_inspect",
        json!({"archive":path,"limit":100,"offset":13}),
    );
    assert_eq!(last["entries"][0]["name"], names[13]);
    assert_eq!(last["entries"][1]["name"], "z");
    assert!(last["nextOffset"].is_null());
}

#[test]
fn image_compare_reports_changed_pixels_and_writes_mask() {
    let tmp = tempfile::tempdir().unwrap();
    let left = tmp.path().join("left.png");
    let right = tmp.path().join("right.png");
    let output = tmp.path().join("mask.png");
    let original = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
    original.save(&left).unwrap();
    let mut modified = original.clone();
    modified.put_pixel(1, 0, image::Rgba([10, 0, 0, 255]));
    modified.save(&right).unwrap();
    let result = call(
        "image_compare",
        json!({"leftPath":left,"rightPath":right,"outputPath":output}),
    );
    assert_eq!(result["differentPixels"], 1);
    assert_eq!(
        result["boundingBox"],
        json!({"x":1,"y":0,"width":1,"height":1})
    );
    let mask = image::open(&output).unwrap().to_rgba8();
    assert_eq!(mask.get_pixel(1, 0).0, [255, 0, 255, 255]);
    assert_eq!(mask.get_pixel(0, 0).0, [0, 0, 0, 0]);
    assert_eq!(
        call(
            "image_compare",
            json!({"leftPath":left,"rightPath":right,"threshold":10})
        )["differentPixels"],
        0
    );
    assert_eq!(
        response(
            "image_compare",
            json!({"leftPath":left,"rightPath":right,"outputPath":left,"overwrite":true})
        )["result"]["isError"],
        true
    );
    image::RgbaImage::new(1, 1).save(&right).unwrap();
    assert_eq!(
        response("image_compare", json!({"leftPath":left,"rightPath":right}))["result"]["isError"],
        true
    );
}
