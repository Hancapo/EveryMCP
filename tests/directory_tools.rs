mod common;
use common::{call, response};
use serde_json::json;
use std::fs;

#[test]
fn directory_size_totals_nested_files_and_prunes_excluded_names() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join("nested/deeper")).unwrap();
    fs::create_dir(temp.path().join("ignored")).unwrap();
    fs::write(temp.path().join("small"), "ab").unwrap();
    fs::write(temp.path().join("nested/large"), "12345").unwrap();
    fs::write(temp.path().join("nested/deeper/medium"), "123").unwrap();
    fs::write(temp.path().join("ignored/huge"), "x".repeat(100)).unwrap();
    fs::write(temp.path().join("nested/skip.txt"), "skip").unwrap();
    let result = call(
        "directory_size",
        json!({"root":temp.path(),"exclude":["ignored","skip.txt"],"limit":1}),
    );
    assert_eq!(result["totalBytes"], 10);
    assert_eq!(result["fileCount"], 3);
    assert_eq!(result["directoryCount"], 2);
    assert_eq!(result["largestFiles"][0]["path"], "nested/large");
    assert_eq!(result["largestFiles"][0]["sizeBytes"], 5);
    assert_eq!(result["largestDirectories"][0]["path"], "nested");
    assert_eq!(result["largestDirectories"][0]["sizeBytes"], 8);
    assert_eq!(result["limitReached"], true);
}

#[test]
fn excluding_one_file_keeps_its_siblings() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("a-skip"), "skip").unwrap();
    fs::write(temp.path().join("z-keep"), "keep").unwrap();
    let result = call(
        "directory_size",
        json!({"root":temp.path(),"exclude":["a-skip"]}),
    );
    assert_eq!(result["totalBytes"], 4);
    assert_eq!(result["fileCount"], 1);
}

#[test]
fn duplicates_compare_content_and_report_reclaimable_bytes_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("ignored")).unwrap();
    for (name, contents) in [("one", "abc"), ("two", "abc"), ("other", "xyz")] {
        fs::write(temp.path().join(name), contents).unwrap();
    }
    fs::write(temp.path().join("ignored/three"), "abc").unwrap();
    let result = call(
        "file_duplicates",
        json!({"root":temp.path(),"exclude":["ignored"]}),
    );
    assert_eq!(result["groups"].as_array().unwrap().len(), 1);
    assert_eq!(result["groups"][0]["paths"], json!(["one", "two"]));
    assert_eq!(result["groups"][0]["fileCount"], 2);
    assert_eq!(result["groups"][0]["sizeBytes"], 3);
    assert_eq!(result["groups"][0]["sha256"].as_str().unwrap().len(), 64);
    assert_eq!(result["duplicateBytes"], 3);
    assert!(temp.path().join("two").is_file());
}

#[test]
fn duplicate_output_is_bounded_without_losing_totals() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..1001 {
        fs::write(temp.path().join(format!("{index:04}")), "x").unwrap();
    }
    let result = call("file_duplicates", json!({"root":temp.path(),"limit":1}));
    assert_eq!(result["groups"][0]["paths"].as_array().unwrap().len(), 1000);
    assert_eq!(result["groups"][0]["fileCount"], 1001);
    assert_eq!(result["groups"][0]["pathsTruncated"], true);
    assert_eq!(result["duplicateBytes"], 1000);
    assert_eq!(result["limitReached"], true);
}

#[test]
fn sync_defaults_to_preview_without_creating_destination() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(source.join("empty")).unwrap();
    fs::write(source.join("new"), "abc").unwrap();
    let result = call(
        "directory_sync",
        json!({"source":source,"destination":destination}),
    );
    assert_eq!(result["dryRun"], true);
    assert_eq!(result["copiedFiles"], 1);
    assert_eq!(result["bytesToCopy"], 3);
    assert!(!destination.exists());
}

#[test]
fn sync_creates_missing_destination_parents_and_bounds_report_only() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("new/parent/destination");
    fs::create_dir_all(source.join("empty")).unwrap();
    fs::write(source.join("new"), "abc").unwrap();
    let result = call(
        "directory_sync",
        json!({"source":source,"destination":destination,"dryRun":false,"limit":1}),
    );
    assert_eq!(result["changes"].as_array().unwrap().len(), 1);
    assert_eq!(result["changeCount"], 3);
    assert_eq!(result["limitReached"], true);
    assert!(destination.join("empty").is_dir());
    assert_eq!(fs::read_to_string(destination.join("new")).unwrap(), "abc");
}

#[test]
fn sync_rejects_filesystem_roots_before_scanning() {
    let temp = tempfile::tempdir().unwrap();
    let filesystem_root = temp.path().ancestors().last().unwrap();
    for (source, destination) in [
        (temp.path(), filesystem_root),
        (filesystem_root, temp.path()),
    ] {
        let result = response(
            "directory_sync",
            json!({"source":source,"destination":destination,"dryRun":false,"delete":true}),
        );
        assert_eq!(result["result"]["isError"], true);
        assert!(
            result["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("filesystem roots"),
            "{result}"
        );
    }
}

#[test]
fn sync_updates_changed_content_preserves_unchanged_files_and_keeps_extras_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("same"), "same").unwrap();
    fs::write(destination.join("same"), "same").unwrap();
    fs::File::options()
        .write(true)
        .open(destination.join("same"))
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(100_000)),
        )
        .unwrap();
    let unchanged_time = fs::metadata(destination.join("same"))
        .unwrap()
        .modified()
        .unwrap();
    fs::write(source.join("changed"), "new").unwrap();
    fs::write(destination.join("changed"), "old").unwrap();
    fs::write(source.join("nested/new"), "created").unwrap();
    fs::write(destination.join("extra"), "keep").unwrap();
    let result = call(
        "directory_sync",
        json!({"source":source,"destination":destination,"dryRun":false}),
    );
    assert_eq!(result["copiedFiles"], 2);
    assert_eq!(result["unchangedFiles"], 1);
    assert_eq!(
        fs::read_to_string(destination.join("changed")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(destination.join("nested/new")).unwrap(),
        "created"
    );
    assert_eq!(
        fs::read_to_string(destination.join("extra")).unwrap(),
        "keep"
    );
    assert_eq!(
        fs::metadata(destination.join("same"))
            .unwrap()
            .modified()
            .unwrap(),
        unchanged_time
    );
}

#[test]
fn sync_deletes_extras_only_when_both_flags_explicitly_allow_it() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir_all(destination.join("extra/nested")).unwrap();
    fs::write(destination.join("extra/nested/file"), "old").unwrap();
    let preview = call(
        "directory_sync",
        json!({"source":source,"destination":destination,"delete":true}),
    );
    assert_eq!(preview["deletedFiles"], 1);
    assert!(destination.join("extra/nested/file").is_file());
    let result = call(
        "directory_sync",
        json!({"source":source,"destination":destination,"delete":true,"dryRun":false}),
    );
    assert_eq!(result["deletedFiles"], 1);
    assert_eq!(result["deletedDirectories"], 2);
    assert!(!destination.join("extra").exists());
    assert!(destination.is_dir());
}

#[test]
fn sync_rejects_overlap_and_type_conflicts_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(source.join("conflict")).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(source.join("a-new"), "new").unwrap();
    fs::write(destination.join("conflict"), "preserve").unwrap();
    for (left, right) in [
        (&source, source.clone()),
        (&source, source.join("child")),
        (&source, temp.path().to_path_buf()),
        (&source, destination.clone()),
    ] {
        let result = response(
            "directory_sync",
            json!({"source":left,"destination":right,"dryRun":false,"delete":true}),
        );
        assert_eq!(result["result"]["isError"], true, "{result}");
    }
    assert!(!destination.join("a-new").exists());
    assert_eq!(
        fs::read_to_string(destination.join("conflict")).unwrap(),
        "preserve"
    );
}

#[test]
fn directory_tools_reject_invalid_limits_and_exclusion_paths() {
    let temp = tempfile::tempdir().unwrap();
    for tool in ["directory_size", "file_duplicates"] {
        for invalid in [
            json!({"root":temp.path(),"limit":0}),
            json!({"root":temp.path(),"limit":1001}),
            json!({"root":temp.path(),"exclude":["a/b"]}),
        ] {
            assert_eq!(response(tool, invalid)["result"]["isError"], true);
        }
    }
}

#[cfg(any(unix, windows))]
#[test]
fn directory_tools_reject_symlinks_and_sync_does_not_touch_their_targets() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let outside = temp.path().join("outside");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(&outside, "preserve").unwrap();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, source.join("link"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&outside, source.join("link"));
    if let Err(error) = linked {
        #[cfg(windows)]
        assert_eq!(error.raw_os_error(), Some(1314), "{error}");
        #[cfg(unix)]
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        return;
    }
    for tool in ["directory_size", "file_duplicates"] {
        assert_eq!(
            response(tool, json!({"root":source}))["result"]["isError"],
            true
        );
    }
    assert_eq!(
        response(
            "directory_sync",
            json!({"source":source,"destination":destination,"dryRun":false,"delete":true})
        )["result"]["isError"],
        true
    );
    assert!(fs::read_dir(&destination).unwrap().next().is_none());
    assert_eq!(fs::read_to_string(outside).unwrap(), "preserve");
}

#[cfg(windows)]
#[test]
fn sync_rejects_junctions_in_destination_and_its_ancestors() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let outside = temp.path().join("outside");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(source.join("new"), "new").unwrap();
    fs::write(outside.join("keep"), "preserve").unwrap();
    let junction = destination.join("link");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&outside)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    for target in [&destination, &junction.join("child")] {
        let result = response(
            "directory_sync",
            json!({"source":source,"destination":target,"dryRun":false,"delete":true}),
        );
        assert_eq!(result["result"]["isError"], true, "{result}");
    }
    assert!(!destination.join("new").exists());
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "preserve"
    );
    fs::remove_dir(junction).unwrap();
}
