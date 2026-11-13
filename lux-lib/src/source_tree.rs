use crate::workspace;

pub(crate) fn is_vcs_dir(name: &str) -> bool {
    matches!(name, ".git" | ".jj" | ".hg" | "_darcs" | ".svn" | ".bzr")
}

pub(crate) fn is_lux_generated(name: &str) -> bool {
    matches!(
        name,
        workspace::LUX_DIR_NAME | "lux.lock" | ".luarc.json" | ".emmyrc.json"
    )
}
