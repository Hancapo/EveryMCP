mod common;
use common::call;
use serde_json::json;
use std::process::Command;

#[test]
fn workspace_scan_finds_manifests_without_entering_build_outputs() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("sample");
    std::fs::create_dir_all(project.join("target").join("ignored")).unwrap();
    std::fs::write(project.join("Cargo.toml"), "[package]\nname='sample'\n").unwrap();
    std::fs::write(
        project
            .join("target")
            .join("ignored")
            .join("pyproject.toml"),
        "",
    )
    .unwrap();
    let result = call("workspace_scan", json!({"root":temp.path()}));
    assert_eq!(result["projects"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["projects"][0]["path"],
        project.to_string_lossy().as_ref()
    );
    assert_eq!(result["projects"][0]["manifests"][0], "Cargo.toml");
}

#[test]
fn repo_status_batch_reports_dirty_repository() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    assert!(
        Command::new("git")
            .arg("init")
            .arg(&repo)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(repo.join("new.txt"), "new").unwrap();
    let result = call("repo_status_batch", json!({"root":temp.path()}));
    assert_eq!(result["repositories"].as_array().unwrap().len(), 1);
    assert_eq!(result["repositories"][0]["dirty"], true);
    assert!(result["repositories"][0]["changedFiles"].as_u64().unwrap() >= 1);
}

#[test]
fn command_pipeline_stops_on_failed_step() {
    let result = call(
        "command_pipeline",
        json!({"steps":[
            {"executable":"git","arguments":["--version"]},
            {"executable":"git","arguments":["--invalid-option"]},
            {"executable":"git","arguments":["--version"]}
        ]}),
    );
    assert_eq!(result["steps"].as_array().unwrap().len(), 2);
    assert_eq!(result["succeeded"], false);
    assert_eq!(result["failedStep"], 1);
}

#[test]
fn environment_snapshot_reports_selected_executable_and_variable() {
    let result = call(
        "environment_snapshot",
        json!({"executables":["git"],"variables":["PATH"]}),
    );
    assert_eq!(result["executables"][0]["name"], "git");
    assert_eq!(result["executables"][0]["available"], true);
    assert!(result["variables"]["PATH"].as_str().is_some());
}

#[test]
fn file_watch_reports_new_file() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_path_buf();
    let target = root.join("new.txt");
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(700));
        std::fs::write(target, "data").unwrap();
    });
    let result = call(
        "file_watch",
        json!({"root":root,"timeoutMs":4000,"intervalMs":40}),
    );
    writer.join().unwrap();
    assert_eq!(result["changed"], true);
    assert_eq!(result["changes"][0]["kind"], "added");
    assert_eq!(result["changes"][0]["path"], "new.txt");
}
