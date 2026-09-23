mod common;
use common::call;
use serde_json::json;

#[test]
fn structured_data_query_reads_five_formats_with_one_pointer() {
    let temp = tempfile::tempdir().unwrap();
    let cases = [
        (
            "sample.json",
            "{\"items\":[{\"name\":\"Ada\"}]}",
            "/items/0/name",
            "Ada",
        ),
        (
            "sample.toml",
            "[person]\nname = 'Ada'\n",
            "/person/name",
            "Ada",
        ),
        (
            "sample.yaml",
            "person:\n  name: Ada\n",
            "/person/name",
            "Ada",
        ),
        (
            "sample.xml",
            "<person><name>Ada</name></person>",
            "/person/name",
            "Ada",
        ),
        ("sample.csv", "name,age\nAda,30\n", "/0/name", "Ada"),
    ];
    for (filename, data, pointer, expected) in cases {
        let path = temp.path().join(filename);
        std::fs::write(&path, data).unwrap();
        let result = call(
            "structured_data_query",
            json!({"path":path,"pointer":pointer}),
        );
        assert_eq!(result["value"], expected, "{filename}");
    }
}

#[test]
fn xml_parent_does_not_duplicate_child_text() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("sample.xml");
    std::fs::write(&path, "<person><name>Ada</name></person>").unwrap();
    let result = call("structured_data_query", json!({"path":path}));
    assert_eq!(result["value"], json!({"person":{"name":"Ada"}}));
}

#[test]
fn structured_data_diff_reports_changed_added_and_removed() {
    let temp = tempfile::tempdir().unwrap();
    let left = temp.path().join("left.json");
    let right = temp.path().join("right.json");
    std::fs::write(&left, r#"{"same":1,"changed":2,"removed":3}"#).unwrap();
    std::fs::write(&right, r#"{"same":1,"changed":4,"added":5}"#).unwrap();
    let result = call(
        "structured_data_diff",
        json!({"leftPath":left,"rightPath":right}),
    );
    assert_eq!(result["equal"], false);
    let changes = result["changes"].as_array().unwrap();
    assert!(
        changes
            .iter()
            .any(|v| v["path"] == "/changed" && v["kind"] == "changed")
    );
    assert!(
        changes
            .iter()
            .any(|v| v["path"] == "/added" && v["kind"] == "added")
    );
    assert!(
        changes
            .iter()
            .any(|v| v["path"] == "/removed" && v["kind"] == "removed")
    );
}

#[test]
fn dependency_inventory_reports_declarations_and_lock_versions() {
    let temp = tempfile::tempdir().unwrap();
    let rust = temp.path().join("rust");
    let python = temp.path().join("python");
    let node = temp.path().join("node");
    for path in [&rust, &python, &node] {
        std::fs::create_dir(path).unwrap();
    }
    std::fs::write(
        rust.join("Cargo.toml"),
        "[package]\nname='example'\nversion='0.1.0'\n[dependencies]\nserde='1.0'\n",
    )
    .unwrap();
    std::fs::write(
        rust.join("Cargo.lock"),
        "version = 3\n[[package]]\nname = 'serde'\nversion = '1.0.200'\n",
    )
    .unwrap();
    std::fs::write(
        python.join("pyproject.toml"),
        "[project]\nname='example'\nversion='0.1.0'\ndependencies=['requests==2.32.0']\n",
    )
    .unwrap();
    std::fs::write(
        node.join("package.json"),
        r#"{"dependencies":{"react":"^19.0.0"}}"#,
    )
    .unwrap();
    std::fs::write(
        node.join("package-lock.json"),
        r#"{"packages":{"node_modules/react":{"version":"19.1.0"}}}"#,
    )
    .unwrap();
    let result = call("dependency_inventory", json!({"root":temp.path()}));
    let projects = result["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 3);
    let rust_deps = projects.iter().find(|p| p["ecosystem"] == "cargo").unwrap()["dependencies"]
        .as_array()
        .unwrap();
    assert!(
        rust_deps
            .iter()
            .any(|d| d["name"] == "serde" && d["resolvedVersion"] == "1.0.200")
    );
    let py_deps = projects
        .iter()
        .find(|p| p["ecosystem"] == "python")
        .unwrap()["dependencies"]
        .as_array()
        .unwrap();
    assert!(
        py_deps
            .iter()
            .any(|d| d["name"] == "requests" && d["requirement"] == "requests==2.32.0")
    );
    let node_deps = projects.iter().find(|p| p["ecosystem"] == "npm").unwrap()["dependencies"]
        .as_array()
        .unwrap();
    assert!(
        node_deps
            .iter()
            .any(|d| d["name"] == "react" && d["resolvedVersion"] == "19.1.0")
    );
}

#[test]
fn dependency_inventory_marks_ambiguous_lock_versions() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname='example'\nversion='0.1.0'\n[dependencies]\nserde='1.0'\n",
    )
    .unwrap();
    std::fs::write(temp.path().join("Cargo.lock"), "version = 3\n[[package]]\nname = 'serde'\nversion = '1.0.100'\n[[package]]\nname = 'serde'\nversion = '1.0.200'\n").unwrap();
    let result = call("dependency_inventory", json!({"root":temp.path()}));
    let dep = &result["projects"][0]["dependencies"][0];
    assert!(dep["resolvedVersion"].is_null());
    assert_eq!(dep["resolvedVersions"], json!(["1.0.100", "1.0.200"]));
}

#[test]
fn dependency_inventory_reads_nuget_and_local_python_install() {
    let temp = tempfile::tempdir().unwrap();
    let dotnet = temp.path().join("dotnet");
    let python = temp.path().join("python");
    std::fs::create_dir(&dotnet).unwrap();
    std::fs::create_dir_all(python.join(".venv/Lib/site-packages/requests-2.32.0.dist-info"))
        .unwrap();
    std::fs::write(dotnet.join("App.csproj"), "<Project><ItemGroup><PackageReference Include=\"Example\" Version=\"1.2.3\" /></ItemGroup></Project>").unwrap();
    std::fs::write(
        python.join("pyproject.toml"),
        "[project]\nname='example'\nversion='0.1.0'\ndependencies=['requests==2.32.0']\n",
    )
    .unwrap();
    let result = call("dependency_inventory", json!({"root":temp.path()}));
    let projects = result["projects"].as_array().unwrap();
    let nuget = projects.iter().find(|p| p["ecosystem"] == "nuget").unwrap();
    assert_eq!(nuget["dependencies"][0]["name"], "Example");
    let python = projects
        .iter()
        .find(|p| p["ecosystem"] == "python")
        .unwrap();
    assert_eq!(python["dependencies"][0]["installedVersion"], "2.32.0");
}
