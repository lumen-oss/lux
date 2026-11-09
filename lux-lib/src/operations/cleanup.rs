use std::collections::HashSet;

use bon::Builder;

use crate::{
    config::Config,
    fs,
    store::Store,
    tree::{Error, InstallTree, Tree},
};

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Cleanup<'a> {
    config: &'a Config,
    tree: &'a Tree,
}

impl<State> CleanupBuilder<'_, State>
where
    State: cleanup_builder::State + cleanup_builder::IsComplete,
{
    pub fn cleanup(self) -> Result<(), Error> {
        let args = self._build();
        let tree = args.tree;
        let build = tree.build_tree(args.config)?;
        let test = tree.test_tree(args.config)?;
        let trees = [tree, &build, &test];

        let mut referenced = HashSet::new();
        for tree in trees {
            for package in tree.lockfile()?.rocks().values() {
                if let Ok(path) = std::fs::canonicalize(tree.layout_for(package.spec()).root) {
                    referenced.insert(path);
                }
            }
        }

        Store::new(
            tree.root()
                .parent()
                .expect("tree root has a parent")
                .join("store")
                .join(tree.version().to_string()),
        )
        .prune(&referenced)?;

        for tree in trees {
            prune_stale_slots(tree)?;
        }

        Ok(())
    }
}

fn prune_stale_slots(tree: &Tree) -> Result<(), Error> {
    let root = tree.root();
    let Some(base) = root.parent() else {
        return Ok(());
    };
    if !base.is_dir() {
        return Ok(());
    }

    let current = std::fs::canonicalize(&root).ok();
    for entry in fs::sync::read_dir(base)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with(".tree-") {
            continue;
        }

        let path = entry.path();
        if current
            .as_ref()
            .is_some_and(|current| std::fs::canonicalize(&path).is_ok_and(|path| &path == current))
        {
            continue;
        }

        let _ = fs::sync::remove_dir_all(path);
    }

    Ok(())
}
