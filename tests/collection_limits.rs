mod common;
use common::{call, response};
use serde_json::json;
use std::fs::{self, File};
use std::io::Write;
use zip::{ZipWriter, write::SimpleFileOptions};

#[test]
fn csv_accepts_100000_rows_and_rejects_the_next_row() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rows.csv");
    fs::write(&path, format!("value\n{}", "x\n".repeat(100_000))).unwrap();
    let result = call(
        "structured_data_query",
        json!({"path":path,"pointer":"/99999/value"}),
    );
    assert_eq!(result["value"], "x");
    let diff = call(
        "structured_data_diff",
        json!({"leftPath":path,"rightPath":path}),
    );
    assert_eq!(diff["equal"], true);
    writeln!(File::options().append(true).open(&path).unwrap(), "extra").unwrap();
    let rejected = response("structured_data_query", json!({"path":path,"pointer":"/0"}));
    assert_eq!(rejected["result"]["isError"], true);
}

#[test]
fn patch_accepts_100000_matches_and_preserves_file_on_excess() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("patch.txt");
    fs::write(&path, "a".repeat(100_000)).unwrap();
    let result = call(
        "file_patch",
        json!({"path":path,"find":"a","replace":"b","expectedMatches":100000}),
    );
    assert_eq!(result["matches"], 100_000);
    assert_eq!(fs::read_to_string(&path).unwrap(), "b".repeat(100_000));
    let rejected = response(
        "file_patch",
        json!({"path":path,"find":"b","replace":"c","expectedMatches":100001}),
    );
    assert_eq!(rejected["result"]["isError"], true);
    assert_eq!(fs::read_to_string(&path).unwrap(), "b".repeat(100_000));
}

#[test]
fn selective_extraction_accepts_100000_zip_entries_and_rejects_100001() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("many.zip");
    let destination = temp.path().join("out");
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    for index in 0..100_000 {
        zip.start_file(index.to_string(), options).unwrap();
    }
    zip.finish().unwrap();
    let result = call(
        "archive_extract_selected",
        json!({"archive":path,"destination":destination,"entries":["99999"]}),
    );
    assert_eq!(result["fileCount"], 1);
    assert!(destination.join("99999").is_file());
    let listing = call("archive_inspect", json!({"archive":path,"limit":100000}));
    assert_eq!(listing["entries"].as_array().unwrap().len(), 100_000);
    assert_eq!(listing["truncated"], false);

    let mut zip =
        ZipWriter::new_append(File::options().read(true).write(true).open(&path).unwrap()).unwrap();
    zip.start_file("extra", options).unwrap();
    zip.finish().unwrap();
    for tool in ["archive_extract", "archive_extract_selected"] {
        let rejected = response(
            tool,
            json!({"archive":path,"destination":temp.path().join("rejected"),"entries":["extra"]}),
        );
        assert_eq!(rejected["result"]["isError"], true);
        assert!(!temp.path().join("rejected").exists());
    }
}

#[test]
fn manifest_limit_is_inclusive_and_accepts_100000_as_maximum() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("one"), "").unwrap();
    fs::write(temp.path().join("two"), "").unwrap();
    let manifest = call("directory_manifest", json!({"root":temp.path(),"limit":2}));
    assert_eq!(manifest["limitReached"], false);
    assert_eq!(
        call("artifact_manifest", json!({"root":temp.path(),"limit":2}))["fileCount"],
        2
    );
    for tool in ["directory_manifest", "artifact_manifest"] {
        assert_eq!(
            call(tool, json!({"root":temp.path(),"limit":100000}))["fileCount"],
            2
        );
        assert_eq!(
            response(tool, json!({"root":temp.path(),"limit":100001}))["result"]["isError"],
            true
        );
    }
    fs::write(temp.path().join("three"), "").unwrap();
    assert_eq!(
        call("directory_manifest", json!({"root":temp.path(),"limit":2}))["limitReached"],
        true
    );
    assert_eq!(
        response("artifact_manifest", json!({"root":temp.path(),"limit":2}))["result"]["isError"],
        true
    );
}

#[test]
fn directory_consumers_accept_more_than_10000_entries() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("files");
    fs::create_dir(&root).unwrap();
    for index in 0..10_001 {
        File::create(root.join(index.to_string())).unwrap();
    }
    let archive = call(
        "archive_create",
        json!({"paths":[root],"archive":temp.path().join("files.zip")}),
    );
    assert_eq!(archive["fileCount"], 10_001);
    assert_eq!(
        call("artifact_manifest", json!({"root":root}))["fileCount"],
        10_001
    );
    assert_eq!(
        call("manifest_diff", json!({"left":root,"right":root}))["equal"],
        true
    );
    assert_eq!(
        call("file_watch", json!({"root":root,"timeoutMs":0}))["changed"],
        false
    );
}
