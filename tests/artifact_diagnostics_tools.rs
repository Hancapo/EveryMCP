mod common;
use common::call;
use common::response;
use serde_json::json;

#[test]
fn artifact_manifest_is_stable_and_manifest_diff_reports_changes() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("left");
    let right = temp.path().join("right");
    std::fs::create_dir(&left).unwrap();
    std::fs::create_dir(&right).unwrap();
    std::fs::write(left.join("same.txt"), "same").unwrap();
    std::fs::write(right.join("same.txt"), "same").unwrap();
    std::fs::write(left.join("changed.txt"), "before").unwrap();
    std::fs::write(right.join("changed.txt"), "after").unwrap();
    std::fs::write(right.join("added.txt"), "new").unwrap();
    let snapshot = temp.path().join("left-manifest.json");
    let first = call(
        "artifact_manifest",
        json!({"root":left,"outputPath":snapshot}),
    );
    let second = call("artifact_manifest", json!({"root":left}));
    assert_eq!(first["manifestSha256"], second["manifestSha256"]);
    assert!(snapshot.exists());
    let diff = call("manifest_diff", json!({"left":snapshot,"right":right}));
    assert_eq!(diff["equal"], false);
    assert!(
        diff["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == "changed.txt" && v["kind"] == "modified")
    );
    assert!(
        diff["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["path"] == "added.txt" && v["kind"] == "added")
    );
}

#[test]
fn manifest_diff_rejects_tampered_saved_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("files");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("one.txt"), "one").unwrap();
    let snapshot = temp.path().join("manifest.json");
    call(
        "artifact_manifest",
        json!({"root":root,"outputPath":snapshot}),
    );
    let mut data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&snapshot).unwrap()).unwrap();
    data["entries"][0]["sha256"] = json!("bad");
    std::fs::write(&snapshot, data.to_string()).unwrap();
    let result = response("manifest_diff", json!({"left":snapshot,"right":root}));
    assert_eq!(result["result"]["isError"], true);
}

#[test]
fn diagnostics_parse_normalizes_common_compiler_lines() {
    let text = "src/main.rs:12:3: error: mismatched types\nC:\\repo\\main.cpp(7,2): error C2143: syntax error\nCMake Error at CMakeLists.txt:4 (find_package): missing package\n";
    let result = call("diagnostics_parse", json!({"text":text}));
    let items = result["diagnostics"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["file"], "src/main.rs");
    assert_eq!(items[0]["line"], 12);
    assert_eq!(items[1]["code"], "C2143");
    assert_eq!(items[2]["file"], "CMakeLists.txt");
}

#[test]
fn diagnostics_parse_reads_rustc_code_and_location() {
    let result = call(
        "diagnostics_parse",
        json!({"text":"error[E0308]: mismatched types\n  --> src/main.rs:10:5\n"}),
    );
    assert_eq!(result["diagnostics"][0]["file"], "src/main.rs");
    assert_eq!(result["diagnostics"][0]["code"], "E0308");
}

#[test]
fn test_results_parse_reads_junit_and_trx() {
    let temp = tempfile::tempdir().unwrap();
    let junit = temp.path().join("junit.xml");
    let trx = temp.path().join("results.trx");
    std::fs::write(&junit, "<testsuite><testcase classname=\"suite\" name=\"ok\"/><testcase classname=\"suite\" name=\"bad\"><failure message=\"expected true\">trace</failure></testcase><testcase name=\"skip\"><skipped/></testcase></testsuite>").unwrap();
    std::fs::write(&trx, "<TestRun><Results><UnitTestResult testName=\"ok\" outcome=\"Passed\"/><UnitTestResult testName=\"bad\" outcome=\"Failed\"><Output><ErrorInfo><Message>boom</Message></ErrorInfo></Output></UnitTestResult></Results></TestRun>").unwrap();
    let junit_result = call("test_results_parse", json!({"path":junit}));
    assert_eq!(junit_result["total"], 3);
    assert_eq!(junit_result["failed"], 1);
    assert_eq!(junit_result["skipped"], 1);
    assert_eq!(junit_result["failures"][0]["message"], "expected true");
    let trx_result = call("test_results_parse", json!({"path":trx}));
    assert_eq!(trx_result["total"], 2);
    assert_eq!(trx_result["failed"], 1);
    assert_eq!(trx_result["failures"][0]["message"], "boom");
}

#[test]
fn test_results_parse_reads_tap() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("results.tap");
    std::fs::write(
        &path,
        "TAP version 13\n1..3\nok 1 - first\nnot ok 2 - second\nok 3 - later # SKIP no platform\n",
    )
    .unwrap();
    let result = call("test_results_parse", json!({"path":path}));
    assert_eq!(result["total"], 3);
    assert_eq!(result["passed"], 1);
    assert_eq!(result["failed"], 1);
    assert_eq!(result["skipped"], 1);
}
