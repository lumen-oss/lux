use std::{collections::HashMap, path::PathBuf};


#[derive(Debug, PartialEq, Default, Clone)]
pub struct TreesitterParserBuildSpec {
    /// Name of the parser language, e.g. "haskell"
    pub lang: String,

    /// Won't build the parser if `false`
    /// (useful for packages that only include queries)
    pub parser: bool,

    /// Must the sources be generated?
    pub generate: bool,

    /// tree-sitter grammar's location (relative to the source root)
    pub location: Option<PathBuf>,

    /// Embedded queries to be installed in the `etc/queries` directory
    pub queries: HashMap<PathBuf, String>,
}

/*
impl UserData for TreesitterParserBuildSpec {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("lang", |_, this, _: ()| Ok(this.lang.clone()));
        methods.add_method("parser", |_, this, _: ()| Ok(this.parser));
        methods.add_method("generate", |_, this, _: ()| Ok(this.generate));
        methods.add_method("location", |_, this, _: ()| Ok(this.location.clone()));
        methods.add_method("queries", |_, this, _: ()| Ok(this.queries.clone()));
    }
}
*/
