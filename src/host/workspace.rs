use super::{io_error, optional_bool, optional_u64, required_str, string_array};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use walkdir::WalkDir;

const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "pyproject.toml",
    "package.json",
    "CMakeLists.txt",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Makefile",
];
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".venv",
    "venv",
    "node_modules",
    "target",
    "build",
    "dist",
    "__pycache__",
    ".pytest_cache",
    ".idea",
    ".vs",
];

pub fn execute(name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "workspace_scan" => scan(args),
        "repo_status_batch" => repo_status(args),
        "command_pipeline" => pipeline(args),
        "environment_snapshot" => environment(args),
        "file_watch" => watch(args),
        _ => Err(format!("Unknown workspace tool '{name}'.")),
    }
}

fn directory(args: &Value) -> Result<PathBuf, String> {
    let path = PathBuf::from(required_str(args, "root")?);
    if !path.is_dir() {
        return Err("'root' must name an existing directory.".into());
    }
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|e| io_error("Cannot resolve root", e))
    }
}

fn project_map(
    root: &Path,
    max_depth: usize,
) -> Result<BTreeMap<PathBuf, (BTreeSet<String>, bool)>, String> {
    let mut found = BTreeMap::<PathBuf, (BTreeSet<String>, bool)>::new();
    if root.join(".git").exists() {
        found.entry(root.to_path_buf()).or_default().1 = true;
    }
    let walker = WalkDir::new(root)
        .max_depth(max_depth + 1)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || !SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref())
        });
    for entry in walker {
        let entry = entry.map_err(|e| io_error("Cannot scan workspace", e))?;
        let name = entry.file_name().to_string_lossy();
        if entry.depth() == 0 {
            continue;
        }
        let parent = entry.path().parent().unwrap_or(root).to_path_buf();
        if entry.file_type().is_dir() && entry.path().join(".git").exists() {
            found.entry(entry.path().to_path_buf()).or_default().1 = true;
        }
        if entry.file_type().is_file() && name == ".git" {
            found.entry(parent).or_default().1 = true;
        } else if entry.file_type().is_file()
            && (MANIFESTS.contains(&name.as_ref())
                || name.ends_with(".sln")
                || name.ends_with(".csproj"))
        {
            found.entry(parent).or_default().0.insert(name.into_owned());
        }
    }
    Ok(found)
}

fn scan(args: &Value) -> Result<Value, String> {
    let root = directory(args)?;
    let max_depth = optional_u64(args, "maxDepth", 2, 5)? as usize;
    let limit = optional_u64(args, "limit", 100, 500)? as usize;
    let projects = project_map(&root, max_depth)?;
    let total = projects.len();
    let projects: Vec<Value> = projects
        .into_iter()
        .take(limit)
        .map(|(path, (manifests, git))| json!({"path":path,"manifests":manifests,"git":git}))
        .collect();
    Ok(json!({"root":root,"projects":projects,"totalMatched":total,"limitReached":total>limit}))
}

fn git(path: &Path, arguments: &[&str]) -> Result<Option<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(arguments)
        .output()
        .map_err(|e| io_error("Cannot run git", e))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(Some(
        String::from_utf8_lossy(&output.stdout).trim().to_owned(),
    ))
}

fn repo_status(args: &Value) -> Result<Value, String> {
    let root = directory(args)?;
    let max_depth = optional_u64(args, "maxDepth", 2, 5)? as usize;
    let limit = optional_u64(args, "limit", 100, 500)? as usize;
    let mut roots = BTreeSet::new();
    for path in project_map(&root, max_depth)?.into_keys() {
        if let Some(actual) = git(&path, &["rev-parse", "--show-toplevel"])? {
            roots.insert(PathBuf::from(actual));
        }
    }
    let total = roots.len();
    let mut repositories = Vec::new();
    for path in roots.into_iter().take(limit) {
        let status = git(
            &path,
            &["status", "--porcelain=v1", "--untracked-files=normal"],
        )?
        .unwrap_or_default();
        let changed = status.lines().count();
        let worktrees = git(&path, &["worktree", "list", "--porcelain"])?.unwrap_or_default();
        repositories.push(json!({
            "path":path,
            "branch":git(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"])?,
            "head":git(&path, &["rev-parse", "--verify", "HEAD"])?,
            "dirty":changed>0,
            "changedFiles":changed,
            "worktreeCount":worktrees.lines().filter(|line| line.starts_with("worktree ")).count()
        }));
    }
    Ok(
        json!({"root":root,"repositories":repositories,"totalMatched":total,"limitReached":total>limit}),
    )
}

fn pipeline(args: &Value) -> Result<Value, String> {
    let steps = args
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("'steps' must be an array.")?;
    if steps.is_empty() || steps.len() > 32 {
        return Err("'steps' must contain 1 to 32 steps.".into());
    }
    let continue_on_error = optional_bool(args, "continueOnError", false)?;
    for (index, step) in steps.iter().enumerate() {
        if !step.is_object() {
            return Err(format!("Step {index} must be an object."));
        }
        required_str(step, "executable")?;
        string_array(step, "arguments", false)?;
        super::optional_str(step, "cwd")?;
        optional_u64(step, "timeoutMs", 30000, 300000)?;
        optional_u64(step, "maxOutputBytes", 1024 * 1024, 4 * 1024 * 1024)?;
        if let Some(env) = step.get("env") {
            let map = env
                .as_object()
                .ok_or("'env' must be an object of strings.")?;
            if map.values().any(|value| !value.is_string()) {
                return Err("'env' must contain only strings.".into());
            }
        }
    }
    let mut results = Vec::new();
    let mut failed_step = None;
    for (index, step) in steps.iter().enumerate() {
        let executable = required_str(step, "executable")?;
        let result = super::process::execute("process_run", step)?;
        let success = result["exitCode"] == 0 && result["timedOut"] == false;
        results.push(
            json!({"index":index,"executable":executable,"result":result,"succeeded":success}),
        );
        if !success {
            failed_step.get_or_insert(index);
            if !continue_on_error {
                break;
            }
        }
    }
    Ok(
        json!({"steps":results,"succeeded":failed_step.is_none(),"failedStep":failed_step,"stoppedEarly":results.len()<steps.len()}),
    )
}

fn environment(args: &Value) -> Result<Value, String> {
    let executables = string_array(args, "executables", false)?;
    let variables = string_array(args, "variables", false)?;
    let version_args = if args.get("versionArguments").is_some() {
        string_array(args, "versionArguments", false)?
    } else {
        vec!["--version".into()]
    };
    if executables.len() > 32 || variables.len() > 64 || version_args.len() > 8 {
        return Err("Too many executables, variables or version arguments.".into());
    }
    let mut tools = Vec::new();
    for name in executables {
        let resolved = super::inspect::executable_resolve(&json!({"name":name}));
        match resolved {
            Ok(info) => {
                let mut cmd = Command::new(
                    info["path"]
                        .as_str()
                        .ok_or("Resolved executable has no path.")?,
                );
                cmd.args(&version_args);
                let probe = super::process::run_capture(cmd, 3000, 4096).ok();
                let version = probe
                    .as_ref()
                    .and_then(|value| {
                        value["stdout"]
                            .as_str()
                            .filter(|s| !s.trim().is_empty())
                            .or_else(|| value["stderr"].as_str())
                    })
                    .map(|s| s.lines().next().unwrap_or("").trim().to_owned());
                tools.push(
                    json!({"name":name,"available":true,"path":info["path"],"version":version}),
                );
            }
            Err(_) => tools.push(json!({"name":name,"available":false})),
        }
    }
    let mut selected = BTreeMap::new();
    for name in variables {
        selected.insert(name.clone(), std::env::var(&name).ok());
    }
    Ok(
        json!({"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"executables":tools,"variables":selected}),
    )
}

#[derive(Clone, Eq, PartialEq)]
struct FileState {
    size: u64,
    modified: Option<SystemTime>,
}

fn files_snapshot(root: &Path, recursive: bool) -> Result<BTreeMap<String, FileState>, String> {
    let max_depth = if recursive { usize::MAX } else { 1 };
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root).max_depth(max_depth).follow_links(false) {
        let entry = entry.map_err(|e| io_error("Cannot inspect watched directory", e))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if files.len() >= 10000 {
            return Err("Watched directory has more than 10000 files.".into());
        }
        let metadata = entry
            .metadata()
            .map_err(|e| io_error("Cannot inspect watched file", e))?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        files.insert(
            relative,
            FileState {
                size: metadata.len(),
                modified: metadata.modified().ok(),
            },
        );
    }
    Ok(files)
}

fn watch(args: &Value) -> Result<Value, String> {
    let root = directory(args)?;
    let recursive = optional_bool(args, "recursive", true)?;
    let timeout = optional_u64(args, "timeoutMs", 30000, 120000)?;
    let interval = optional_u64(args, "intervalMs", 200, 5000)?.max(20);
    let limit = optional_u64(args, "limit", 100, 1000)? as usize;
    let before = files_snapshot(&root, recursive)?;
    let start = Instant::now();
    loop {
        if start.elapsed() >= Duration::from_millis(timeout) {
            return Ok(
                json!({"changed":false,"changes":[],"elapsedMs":start.elapsed().as_millis()}),
            );
        }
        thread::sleep(Duration::from_millis(
            interval.min(timeout.saturating_sub(start.elapsed().as_millis() as u64)),
        ));
        let after = files_snapshot(&root, recursive)?;
        let mut changes = Vec::new();
        for (path, state) in &after {
            match before.get(path) {
                None => changes.push(json!({"path":path,"kind":"added"})),
                Some(old) if old != state => changes.push(json!({"path":path,"kind":"modified"})),
                _ => {}
            }
        }
        for path in before.keys() {
            if !after.contains_key(path) {
                changes.push(json!({"path":path,"kind":"removed"}));
            }
        }
        if !changes.is_empty() {
            changes.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
            let total = changes.len();
            changes.truncate(limit);
            return Ok(
                json!({"changed":true,"changes":changes,"totalChanges":total,"limitReached":total>limit,"elapsedMs":start.elapsed().as_millis()}),
            );
        }
    }
}
