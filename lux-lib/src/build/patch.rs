use bon::Builder;
use diffy::{self, ApplyError, ParsePatchError};
use miette::Diagnostic;
use std::io;
use std::{collections::HashMap, path::PathBuf};
use thiserror::Error;
#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub(crate) struct Patch<'a> {
    #[builder(start_fn)]
    dir: &'a PathBuf,
    #[builder(start_fn)]
    patches: &'a HashMap<PathBuf, String>,
}

impl<State> PatchBuilder<'_, State>
where
    State: patch_builder::State + patch_builder::IsComplete,
{
    pub fn apply(self) -> Result<(), PatchError> {
        do_apply(self._build())
    }
}

#[derive(Error, Debug, Diagnostic)]
#[non_exhaustive]
pub enum PatchError {
    #[error("error parsing patch {0}: {1}")]
    Parse(PathBuf, ParsePatchError),
    #[error("failed to apply patch {patch_file}: error reading original file {orig_file}: {err}")]
    OriginalFileRead {
        patch_file: PathBuf,
        orig_file: PathBuf,
        err: io::Error,
    },
    #[error("error applying patch {0}: {1}")]
    Apply(PathBuf, ApplyError),
    #[error("failed to apply patch {patch_file}: error creating directory {dir}: {err}")]
    CreateDir {
        patch_file: PathBuf,
        dir: PathBuf,
        err: io::Error,
    },
    #[error(
        "failed to apply patch {patch_file}: error writing modified file {modified_file}: {err}"
    )]
    ModifiedFileWrite {
        patch_file: PathBuf,
        modified_file: PathBuf,
        err: io::Error,
    },
    #[error("failed to apply patch {patch_file}: error deleting file {file}: {err}")]
    Delete {
        patch_file: PathBuf,
        file: PathBuf,
        err: io::Error,
    },
}

#[tracing::instrument(level = "trace", skip_all)]
pub(crate) fn do_apply(args: Patch<'_>) -> Result<(), PatchError> {
    for (path, patch_str) in args.patches {
        let patch = diffy::Patch::from_str(patch_str)
            .map_err(|err| PatchError::Parse(path.clone(), err))?;

        let original_file = patch
            .original()
            .map(|file| {
                PathBuf::from(file)
                    .components()
                    .skip(1) // remove a/
                    .collect::<PathBuf>()
            })
            .filter(|relative_path| relative_path != &PathBuf::from("dev/null"))
            .map(|relative_path| args.dir.join(relative_path));

        let original_content = original_file
            .as_ref()
            .map(|file| {
                std::fs::read_to_string(file).map_err(|err| PatchError::OriginalFileRead {
                    patch_file: path.clone(),
                    orig_file: file.clone(),
                    err,
                })
            })
            .map_or(Ok(None), |v| v.map(Some))?
            .unwrap_or_default();

        let modified = patch
            .modified()
            .map(|file| {
                PathBuf::from(file)
                    .components()
                    .skip(1) // remove b/
                    .collect::<PathBuf>()
            })
            .filter(|relative_path| relative_path != &PathBuf::from("dev/null"))
            .map(|relative_path| args.dir.join(relative_path))
            .map(|modified_file| {
                let modified_content = diffy::apply(&original_content, &patch)
                    .map_err(|err| PatchError::Apply(path.clone(), err))?;
                Ok((modified_file, modified_content))
            })
            .map_or(Ok(None), |v| v.map(Some))?
            .map(|(file, modified_content)| {
                let parent = file.parent().unwrap_or(args.dir);
                std::fs::create_dir_all(parent).map_err(|err| PatchError::CreateDir {
                    patch_file: path.clone(),
                    dir: parent.to_path_buf(),
                    err,
                })?;
                std::fs::write(&file, modified_content).map_err(|err| {
                    PatchError::ModifiedFileWrite {
                        patch_file: path.clone(),
                        modified_file: file,
                        err,
                    }
                })
            })
            .map_or(Ok(None), |v| v.map(Some))?;

        if modified.is_none() {
            if let Some(original) = original_file {
                std::fs::remove_file(&original).map_err(|err| PatchError::Delete {
                    patch_file: path.clone(),
                    file: original,
                    err,
                })?
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;
    use assert_fs::TempDir;
    use std::{fs, path::PathBuf};

    #[test]
    fn test_simple_patch() {
        let temp_dir = TempDir::new().unwrap();
        let test_file =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/test/patch_test/pack.lua");
        let orig_content = std::fs::read_to_string(&test_file).unwrap();
        let added_line = r#"_G._ENV = rawget(_G, "_ENV") -- to satisfy tarantool strict mode"#;
        assert!(!orig_content.contains(added_line));
        let scripts_dir = temp_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        let patch_file = scripts_dir.join("pack.lua");
        fs::copy(&test_file, &patch_file).unwrap();
        let patches = vec![(
            PathBuf::from("test_patch.diff"),
            r#"
diff --git a/scripts/pack.lua b/scripts/pack.lua
index 959c7ed..9c6a9a1 100644
--- a/scripts/pack.lua
+++ b/scripts/pack.lua
@@ -38,4 +38,5 @@ scandir( root )

 acc={(io.open("../ABOUT"):read("*all").."\n"):gsub( "([^\n]-\n)","-- %1" ),[[
+_G._ENV = rawget(_G, "_ENV") -- to satisfy tarantool strict mode
 local _ENV,       loaded, packages, release, require_
     = _ENV or _G, {},     {},       true,    require
"#
            .to_string(),
        )]
        .into_iter()
        .collect();

        Patch::new(&temp_dir.join(""), &patches).apply().unwrap();

        let patched_content = std::fs::read_to_string(&patch_file).unwrap();
        assert!(patched_content.contains(added_line));
    }

    #[test]
    fn test_git_patch_create_file() {
        let temp_dir = TempDir::new().unwrap();
        let patches = vec![(
            PathBuf::from("test.patch"),
            r#"
diff --git a/foo/README.md b/foo/README.md
new file mode 100644
index 0000000..1cbadfb
--- /dev/null
+++ b/foo/README.md
@@ -0,0 +1 @@
+# title
"#
            .to_string(),
        )]
        .into_iter()
        .collect();
        Patch::new(&temp_dir.join(""), &patches).apply().unwrap();
        let patch_file = temp_dir.join("foo/README.md");
        let patched_content = std::fs::read_to_string(&patch_file).unwrap();
        assert!(patched_content.contains("# title"));
    }

    #[test]
    fn test_git_patch_delete_file() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.join("README.md");
        std::fs::write(&test_file, "# title").unwrap();
        let patches = vec![(
            PathBuf::from("test.patch"),
            r#"
diff --git a/README.md b/README.md
deleted file mode 100644
index 1cbadfb..0000000
--- a/README.md
+++ /dev/null
@@ -1 +0,0 @@
-# title
"#
            .to_string(),
        )]
        .into_iter()
        .collect();

        Patch::new(&temp_dir.join(""), &patches).apply().unwrap();
        assert!(!test_file.exists());
    }
}
