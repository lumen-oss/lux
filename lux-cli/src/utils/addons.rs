use eyre::Result;
use lux_lib::{project::project_toml::LocalProjectToml, project::Project, rockspec::Rockspec};
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::fs;
use tokio::process::Command;
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct AddonInstallEntry {
    pub name: String,
    pub source: String, // "luarocks", "lls_addons+https://github.com/LuaLS/LLS-Addons", or "none"
    pub required: bool,
    pub ok: bool,
    pub message: Option<String>,
    pub library_paths: Vec<String>,
    pub version: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug)]
pub struct AddonInstallReport {
    pub entries: Vec<AddonInstallEntry>,
}

pub async fn install_lls_addons_for_project(
    project: &Project,
    config: &lux_lib::config::Config,
    addons: &[String],
    fail_on_error: bool,
) -> Result<AddonInstallReport> {
    if addons.is_empty() {
        return Ok(AddonInstallReport { entries: vec![] });
    }

    let mut report = AddonInstallReport { entries: vec![] };
    for addon in addons {
        let addon_dir = addon;
        let lua_version_str = project.lua_version(config)?.to_string();
        // Git-only path
        match install_from_lls_addons_git(project, &addon_dir, lua_version_str.clone()).await {
            Err(git_err) => {
                if config.verbose() {
                    eprintln!(
                        "Warning: failed to install LuaLS addon '{}' via git: {}",
                        addon, git_err
                    );
                }
                report.entries.push(AddonInstallEntry {
                    name: addon.clone(),
                    source: "lls_addons+https://github.com/LuaLS/LLS-Addons".into(),
                    required: fail_on_error,
                    ok: false,
                    message: Some(format!("{}", git_err)),
                    library_paths: vec![],
                    version: None,
                    commit: None,
                });
            }
            Ok(commit) => {
                // Per-addon library path under .lux/<lua>/lls_addons/addons/<name>/library
                let dest_addon_library = project
                    .root()
                    .join(".lux")
                    .join(&lua_version_str)
                    .join("lls_addons")
                    .join("addons")
                    .join(&addon_dir)
                    .join("library");
                report.entries.push(AddonInstallEntry {
                    name: addon.clone(),
                    source: "lls_addons+https://github.com/LuaLS/LLS-Addons".into(),
                    required: fail_on_error,
                    ok: true,
                    message: None,
                    library_paths: vec![dest_addon_library.to_string_lossy().to_string()],
                    version: None,
                    commit,
                });
            }
        }
    }
    if fail_on_error && report.entries.iter().any(|e| !e.ok) {
        let reasons = report
            .entries
            .iter()
            .filter(|e| !e.ok)
            .map(|e| format!("{}: {}", e.name, e.message.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n  - ");
        eyre::bail!("addon installation failed:\n  - {}", reasons);
    }
    Ok(report)
}

pub fn derive_implicit_addons(local: &LocalProjectToml, _project: &Project) -> Vec<String> {
    // Gather dependency names for regular and test, 1:1 as addon names
    let mut addons = std::collections::BTreeSet::new();
    for dep in local.dependencies().current_platform() {
        addons.insert(dep.name().to_string());
    }
    for dep in local.test_dependencies().current_platform() {
        addons.insert(dep.name().to_string());
    }
    for dep in local.build_dependencies().current_platform() {
        addons.insert(dep.name().to_string());
    }
    addons.into_iter().collect()
}

async fn install_from_lls_addons_git(
    project: &Project,
    addon_dir: &str,
    lua_version: String,
) -> Result<Option<String>> {
    let cache_dir = project
        .root()
        .join(".lux")
        .join(".cache")
        .join("LLS-Addons");
    if !cache_dir.is_dir() {
        let status = Command::new("git")
            .current_dir(project.root())
            .args([
                "clone",
                "--depth",
                "1",
                "https://github.com/LuaLS/LLS-Addons",
                &cache_dir.to_string_lossy(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await?;
        if !status.success() {
            eyre::bail!("git clone LLS-Addons failed");
        }
    }
    // Init the specific addon submodule if present
    let _status = Command::new("git")
        .current_dir(&cache_dir)
        .args([
            "submodule",
            "update",
            "--init",
            &format!("addons/{}", addon_dir),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;
    // Compute submodule commit (short)
    let module_dir = cache_dir.join("addons").join(addon_dir).join("module");
    let commit = if module_dir.is_dir() {
        let out = Command::new("git")
            .current_dir(&module_dir)
            .args(["rev-parse", "--short=7", "HEAD"])
            .output()
            .await?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            None
        }
    } else {
        None
    };

    let src_library = cache_dir
        .join("addons")
        .join(addon_dir)
        .join("module")
        .join("library");
    if !src_library.is_dir() {
        eyre::bail!(
            "addon '{}' library folder not found in LLS-Addons",
            addon_dir
        );
    }

    let dest_addon_library = project
        .root()
        .join(".lux")
        .join(&lua_version)
        .join("lls_addons")
        .join("addons")
        .join(addon_dir)
        .join("library");
    fs::create_dir_all(&dest_addon_library).await?;
    copy_dir_recursive(&src_library, &dest_addon_library).await?;

    // Merge config.json settings if present
    let config_json = cache_dir
        .join("addons")
        .join(addon_dir)
        .join("module")
        .join("config.json");
    if config_json.is_file() {
        if let Ok(bytes) = fs::read(&config_json).await {
            if let Ok(mut config) = serde_json::from_slice::<Value>(&bytes) {
                // config.json shape: { "settings": { "Lua.diagnostics.globals": [...] , ... } }
                if let Some(settings) = config.get_mut("settings") {
                    patch_luarc_with_settings(
                        project,
                        &project
                            .root()
                            .join(".lux")
                            .join(&lua_version)
                            .join("lls_addons"),
                        &dest_addon_library,
                        settings.take(),
                    )
                    .await?;
                } else {
                    // still ensure paths
                    patch_luarc_with_settings(
                        project,
                        &project
                            .root()
                            .join(".lux")
                            .join(&lua_version)
                            .join("lls_addons"),
                        &dest_addon_library,
                        Value::Null,
                    )
                    .await?;
                }
            }
        }
    } else {
        // ensure paths only
        patch_luarc_with_settings(
            project,
            &project
                .root()
                .join(".lux")
                .join(&lua_version)
                .join("lls_addons"),
            &dest_addon_library,
            Value::Null,
        )
        .await?;
    }

    Ok(commit)
}

async fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in WalkDir::new(src) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).unwrap();
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).await?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).await?;
            }
            fs::copy(entry.path(), &target).await?;
        }
    }
    Ok(())
}

async fn patch_luarc_with_settings(
    project: &Project,
    user_third_party_dir: &Path,
    library_dir: &Path,
    settings: Value,
) -> Result<()> {
    let luarc_path = project.luarc_path();
    let mut root: Value = if luarc_path.is_file() {
        let content = fs::read(&luarc_path).await.unwrap_or_default();
        serde_json::from_slice(&content).unwrap_or(Value::Object(Default::default()))
    } else {
        Value::Object(Default::default())
    };

    let arr = get_or_create_array(&mut root, &["workspace", "library"]);
    let existing = std::mem::take(arr);
    let mut cleaned = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for value in existing.into_iter() {
        match value {
            Value::String(s) => {
                if s.contains("/lls_addons/share/lua/") || s.contains("\\lls_addons\\share\\lua\\")
                {
                    continue;
                }
                let candidate_abs = if std::path::Path::new(&s).is_relative() {
                    project.root().join(&s)
                } else {
                    std::path::PathBuf::from(&s)
                };
                let normalized_path = candidate_abs
                    .strip_prefix(project.root())
                    .map(|rel| rel.to_path_buf())
                    .unwrap_or(candidate_abs.clone());
                let normalized = path_to_slash_string(&normalized_path);
                if seen.insert(normalized.clone()) {
                    cleaned.push(Value::String(normalized));
                }
            }
            other => cleaned.push(other),
        }
    }

    *arr = cleaned;

    // Normalize Lua.workspace.userThirdParty entries
    let arr = get_or_create_array(&mut root, &["Lua", "workspace", "userThirdParty"]);
    let existing = std::mem::take(arr);
    let mut cleaned = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    for value in existing.into_iter() {
        match value {
            Value::String(s) => {
                let candidate_abs = if std::path::Path::new(&s).is_relative() {
                    project.root().join(&s)
                } else {
                    std::path::PathBuf::from(&s)
                };
                let normalized_path = candidate_abs
                    .strip_prefix(project.root())
                    .map(|rel| rel.to_path_buf())
                    .unwrap_or(candidate_abs.clone());
                let normalized = path_to_slash_string(&normalized_path);
                if seen.insert(normalized.clone()) {
                    cleaned.push(Value::String(normalized));
                }
            }
            other => cleaned.push(other),
        }
    }

    *arr = cleaned;

    // Ensure workspace.library contains library_dir
    push_unique_string(project, &mut root, &["workspace", "library"], library_dir);
    // Ensure Lua.workspace.userThirdParty contains the addons base dir
    push_unique_string(
        project,
        &mut root,
        &["Lua", "workspace", "userThirdParty"],
        user_third_party_dir,
    );

    // Merge settings (flat object of "Lua.*": values)
    if let Value::Object(map) = settings {
        for (k, v) in map {
            merge_key(&mut root, &k, v);
        }
    }

    let pretty = serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{}".into());
    fs::write(&luarc_path, pretty).await?;
    Ok(())
}

fn push_unique_string(project: &Project, root: &mut Value, path: &[&str], to_add: &Path) {
    let arr = get_or_create_array(root, path);
    let entry = format_luarc_path(project, to_add);
    let val = Value::String(entry);
    if !arr.iter().any(|v| v == &val) {
        arr.push(val);
    }
}

fn format_luarc_path(project: &Project, path: &Path) -> String {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project.root().join(path)
    };
    let normalized = match absolute.strip_prefix(project.root()) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => absolute,
    };
    path_to_slash_string(&normalized)
}

fn path_to_slash_string(path: &Path) -> String {
    let mut s = path.to_string_lossy().into_owned();
    if cfg!(windows) {
        s = s.replace('\\', "/");
    }
    s
}

fn get_or_create_array<'a>(root: &'a mut Value, path: &[&str]) -> &'a mut Vec<Value> {
    let mut cur = root;
    for (i, key) in path.iter().enumerate() {
        let is_last = i == path.len() - 1;
        if let Value::Object(obj) = cur {
            if is_last {
                let entry = obj
                    .entry((*key).to_string())
                    .or_insert_with(|| Value::Array(vec![]));
                if !entry.is_array() {
                    *entry = Value::Array(vec![]);
                }
                return entry.as_array_mut().unwrap();
            } else {
                cur = obj
                    .entry((*key).to_string())
                    .or_insert_with(|| Value::Object(Default::default()));
            }
        } else {
            // replace with object
            *cur = Value::Object(Default::default());
            if let Value::Object(obj) = cur {
                if is_last {
                    let entry = obj
                        .entry((*key).to_string())
                        .or_insert_with(|| Value::Array(vec![]));
                    if !entry.is_array() {
                        *entry = Value::Array(vec![]);
                    }
                    return entry.as_array_mut().unwrap();
                } else {
                    cur = obj
                        .entry((*key).to_string())
                        .or_insert_with(|| Value::Object(Default::default()));
                }
            } else {
                unreachable!();
            }
        }
    }
    unreachable!();
}

fn merge_key(root: &mut Value, dotted_key: &str, value: Value) {
    // Merge e.g. "Lua.diagnostics.globals" under root["Lua"]["diagnostics"]["globals"]
    let parts: Vec<&str> = dotted_key.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;
        if let Value::Object(obj) = cur {
            if is_last {
                match (obj.get_mut(*part), &value) {
                    (Some(existing), Value::Array(new_arr)) if existing.is_array() => {
                        let existing_arr = existing.as_array_mut().unwrap();
                        for v in new_arr {
                            if !existing_arr.contains(v) {
                                existing_arr.push(v.clone());
                            }
                        }
                    }
                    _ => {
                        obj.insert((*part).to_string(), value.clone());
                    }
                }
                return;
            } else {
                cur = obj
                    .entry((*part).to_string())
                    .or_insert_with(|| Value::Object(Default::default()));
            }
        } else {
            *cur = Value::Object(Default::default());
            if let Value::Object(obj) = cur {
                if is_last {
                    obj.insert((*part).to_string(), value.clone());
                    return;
                } else {
                    cur = obj
                        .entry((*part).to_string())
                        .or_insert_with(|| Value::Object(Default::default()));
                }
            }
        }
    }
}
