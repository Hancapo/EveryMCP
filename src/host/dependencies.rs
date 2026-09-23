use super::{data, io_error, optional_u64, workspace};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

pub fn execute(args: &Value) -> Result<Value, String> {
    let root = workspace::directory(args)?;
    let max_depth = optional_u64(args, "maxDepth", 2, 5)? as usize;
    let limit = optional_u64(args, "limit", 100, 500)? as usize;
    let map = workspace::project_map(&root, max_depth)?;
    let mut projects = Vec::new();
    for (path, (manifests, _)) in map {
        for manifest in manifests {
            let item = if manifest == "Cargo.toml" {
                Some(cargo(&path)?)
            } else if manifest == "pyproject.toml" {
                Some(python(&path)?)
            } else if manifest == "package.json" {
                Some(npm(&path)?)
            } else if manifest.ends_with(".csproj") {
                Some(nuget(&path, &manifest)?)
            } else {
                None
            };
            if let Some(item) = item {
                projects.push(item);
            }
        }
    }
    let total = projects.len();
    projects.truncate(limit);
    Ok(json!({"root":root,"projects":projects,"totalMatched":total,"limitReached":total>limit}))
}

fn read_optional(path: &Path, format: &str) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    data::load(path, Some(format)).map(|(_, value)| Some(value))
}

fn lock_packages(value: Option<&Value>) -> BTreeMap<String, Vec<String>> {
    let mut versions = BTreeMap::new();
    if let Some(packages) = value
        .and_then(|v| v.get("package"))
        .and_then(Value::as_array)
    {
        for package in packages {
            if let (Some(name), Some(version)) =
                (package["name"].as_str(), package["version"].as_str())
            {
                let entry = versions
                    .entry(name.to_ascii_lowercase())
                    .or_insert_with(Vec::new);
                if !entry.iter().any(|current| current == version) {
                    entry.push(version.to_owned());
                }
            }
        }
    }
    versions
}

fn pinned(requirement: &Value, ecosystem: &str) -> Option<String> {
    let raw = requirement.as_str()?;
    match ecosystem {
        "python" => raw
            .split("==")
            .nth(1)
            .filter(|version| !version.contains([',', ';', ' ']))
            .map(str::to_owned),
        "cargo" => raw.strip_prefix('=').map(str::to_owned),
        "npm" | "nuget"
            if raw.chars().next().is_some_and(|c| c.is_ascii_digit())
                && raw.chars().all(|c| c.is_ascii_digit() || c == '.') =>
        {
            Some(raw.to_owned())
        }
        _ => None,
    }
}

fn dependency(
    name: &str,
    scope: &str,
    requirement: Value,
    resolved: Option<&str>,
    installed: Option<&str>,
    ecosystem: &str,
) -> Value {
    let pin = pinned(&requirement, ecosystem);
    let mismatch = pin.as_ref().is_some_and(|expected| {
        resolved.is_some_and(|actual| actual != expected)
            || installed.is_some_and(|actual| actual != expected)
    });
    json!({"name":name,"scope":scope,"requirement":requirement,
        "resolvedVersion":resolved,"installedVersion":installed,"pinMismatch":mismatch})
}

fn cargo(path: &Path) -> Result<Value, String> {
    let manifest = path.join("Cargo.toml");
    let (_, document) = data::load(&manifest, Some("toml"))?;
    let lock = read_optional(&path.join("Cargo.lock"), "toml")?;
    let versions = lock_packages(lock.as_ref());
    let mut dependencies = Vec::new();
    for (field, scope) in [
        ("dependencies", "runtime"),
        ("dev-dependencies", "development"),
        ("build-dependencies", "build"),
    ] {
        if let Some(map) = document.get(field).and_then(Value::as_object) {
            for (name, spec) in map {
                let requirement = spec.get("version").cloned().unwrap_or_else(|| spec.clone());
                let matches = versions
                    .get(&name.to_ascii_lowercase())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let resolved = if matches.len() == 1 {
                    Some(matches[0].as_str())
                } else {
                    None
                };
                let mut item = dependency(name, scope, requirement, resolved, None, "cargo");
                item["resolvedVersions"] = json!(matches);
                dependencies.push(item);
            }
        }
    }
    Ok(
        json!({"path":path,"manifest":manifest,"ecosystem":"cargo","dependencies":dependencies,"lockfile":lock.as_ref().map(|_| path.join("Cargo.lock"))}),
    )
}

fn normalized(name: &str) -> String {
    name.to_ascii_lowercase().replace(['_', '.'], "-")
}

fn python_installed(path: &Path) -> BTreeMap<String, String> {
    let mut roots = vec![
        path.join(".venv/Lib/site-packages"),
        path.join("venv/Lib/site-packages"),
    ];
    for env in [".venv", "venv"] {
        let lib = path.join(env).join("lib");
        if let Ok(children) = std::fs::read_dir(lib) {
            for child in children.flatten() {
                roots.push(child.path().join("site-packages"));
            }
        }
    }
    let mut installed = BTreeMap::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten().take(5000) {
            let filename = entry.file_name().to_string_lossy().to_string();
            let Some(stem) = filename.strip_suffix(".dist-info") else {
                continue;
            };
            if let Some((name, version)) = stem.rsplit_once('-') {
                installed.insert(normalized(name), version.to_owned());
            }
        }
    }
    installed
}

fn python_name(requirement: &str) -> String {
    requirement
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect()
}

fn python(path: &Path) -> Result<Value, String> {
    let manifest = path.join("pyproject.toml");
    let (_, document) = data::load(&manifest, Some("toml"))?;
    let lock = read_optional(&path.join("uv.lock"), "toml")?;
    let versions = lock_packages(lock.as_ref());
    let installed = python_installed(path);
    let mut dependencies = Vec::new();
    let mut groups: Vec<(&str, &Value)> = Vec::new();
    if let Some(required) = document.pointer("/project/dependencies") {
        groups.push(("runtime", required));
    }
    if let Some(optional) = document
        .pointer("/project/optional-dependencies")
        .and_then(Value::as_object)
    {
        groups.extend(optional.iter().map(|(name, value)| (name.as_str(), value)));
    }
    if let Some(required) = document.pointer("/build-system/requires") {
        groups.push(("build", required));
    }
    for (scope, requirements) in groups {
        let Some(requirements) = requirements.as_array() else {
            return Err(format!(
                "Invalid dependency list in '{}'.",
                manifest.display()
            ));
        };
        for requirement in requirements {
            let raw = requirement
                .as_str()
                .ok_or("Python dependency must be a string.")?;
            let name = python_name(raw);
            if name.is_empty() {
                return Err(format!("Cannot identify Python dependency '{raw}'."));
            }
            let key = normalized(&name);
            let matches = versions.get(&key).map(Vec::as_slice).unwrap_or(&[]);
            let resolved = if matches.len() == 1 {
                Some(matches[0].as_str())
            } else {
                None
            };
            let mut item = dependency(
                &name,
                scope,
                json!(raw),
                resolved,
                installed.get(&key).map(String::as_str),
                "python",
            );
            item["resolvedVersions"] = json!(matches);
            dependencies.push(item);
        }
    }
    Ok(
        json!({"path":path,"manifest":manifest,"ecosystem":"python","dependencies":dependencies,"lockfile":lock.as_ref().map(|_|path.join("uv.lock"))}),
    )
}

fn npm(path: &Path) -> Result<Value, String> {
    let manifest = path.join("package.json");
    let (_, document) = data::load(&manifest, Some("json"))?;
    let lock = read_optional(&path.join("package-lock.json"), "json")?;
    let mut dependencies = Vec::new();
    for (field, scope) in [
        ("dependencies", "runtime"),
        ("devDependencies", "development"),
        ("optionalDependencies", "optional"),
        ("peerDependencies", "peer"),
    ] {
        if let Some(map) = document.get(field).and_then(Value::as_object) {
            for (name, requirement) in map {
                let resolved = lock
                    .as_ref()
                    .and_then(|value| {
                        value.pointer(&format!(
                            "/packages/node_modules~1{}",
                            name.replace('~', "~0").replace('/', "~1")
                        ))
                    })
                    .and_then(|value| value.get("version"))
                    .and_then(Value::as_str);
                let installed_path = path.join("node_modules").join(name).join("package.json");
                let installed = read_optional(&installed_path, "json")?
                    .and_then(|value| value["version"].as_str().map(str::to_owned));
                dependencies.push(dependency(
                    name,
                    scope,
                    requirement.clone(),
                    resolved,
                    installed.as_deref(),
                    "npm",
                ));
            }
        }
    }
    Ok(
        json!({"path":path,"manifest":manifest,"ecosystem":"npm","dependencies":dependencies,"lockfile":lock.as_ref().map(|_|path.join("package-lock.json"))}),
    )
}

fn nuget(path: &Path, filename: &str) -> Result<Value, String> {
    let manifest = path.join(filename);
    let source = data::read_text(&manifest)?;
    let document =
        roxmltree::Document::parse(&source).map_err(|e| io_error("Invalid MSBuild XML", e))?;
    let lock = read_optional(&path.join("packages.lock.json"), "json")?;
    let mut dependencies = Vec::new();
    for node in document
        .descendants()
        .filter(|node| node.has_tag_name("PackageReference"))
    {
        let Some(name) = node
            .attribute("Include")
            .or_else(|| node.attribute("Update"))
        else {
            continue;
        };
        let requirement = node
            .attribute("Version")
            .or_else(|| {
                node.children()
                    .find(|child| child.has_tag_name("Version"))
                    .and_then(|child| child.text())
            })
            .unwrap_or("");
        let resolved = lock
            .as_ref()
            .and_then(|value| value.get("dependencies"))
            .and_then(Value::as_object)
            .and_then(|frameworks| {
                frameworks
                    .values()
                    .find_map(|framework| framework.get(name))
            })
            .and_then(|entry| entry["resolved"].as_str());
        dependencies.push(dependency(
            name,
            "runtime",
            json!(requirement),
            resolved,
            None,
            "nuget",
        ));
    }
    Ok(
        json!({"path":path,"manifest":manifest,"ecosystem":"nuget","dependencies":dependencies,"lockfile":lock.as_ref().map(|_|path.join("packages.lock.json"))}),
    )
}
