use std::{collections::HashSet, io};

use bon::Builder;

use crate::{
    fs,
    tree::{Error, Slot as TreeSlot, Tree},
};

#[derive(Builder)]
#[builder(start_fn = new, finish_fn(name = _build, vis = ""))]
pub struct Cleanup<'a> {
    tree: &'a Tree,
}

impl<State> CleanupBuilder<'_, State>
where
    State: cleanup_builder::State + cleanup_builder::IsComplete,
{
    pub fn cleanup(self) -> Result<(), Error> {
        let args = self._build();
        let tree = args.tree;
        let regular = tree.regular();
        let build = tree.build()?;
        let test = tree.test()?;
        let slots = [&regular, &build, &test];

        let mut referenced = HashSet::new();
        for slot in slots {
            for package in slot.lockfile()?.rocks().values() {
                let path = slot.layout_for(package.spec()).root;
                match std::fs::canonicalize(&path) {
                    Ok(path) => {
                        referenced.insert(path);
                    }
                    Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                    Err(source) => {
                        return Err(fs::Error::Metadata { path, source }.into());
                    }
                }
            }
        }

        tree.prune_store(&referenced)?;

        for slot in slots {
            prune_stale_slots(slot)?;
        }

        Ok(())
    }
}

fn prune_stale_slots(slot: &TreeSlot) -> Result<(), Error> {
    let root = slot.root();
    let Some(base) = root.parent() else {
        unreachable!("tree root has no parent");
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
