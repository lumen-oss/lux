use darklua_core::{
    generator::{LuaGenerator, TokenBasedLuaGenerator},
    nodes::{Arguments, Expression, FunctionCall, Prefix, StringExpression},
    process::{DefaultVisitor, NodeProcessor, NodeVisitor},
    Parser,
};

use super::Error;

/// Rewrites Luau `require` paths in Lua source to flat module names.
pub(super) fn flatten_requires(code: &str, module: &str) -> Result<String, Error> {
    let mut block = Parser::default()
        .preserve_tokens()
        .parse(code)
        .map_err(|err| Error::Transpile(err.to_string()))?;
    let mut flattener = RequireFlattener {
        module,
        error: None,
    };
    DefaultVisitor::visit_block(&mut block, &mut flattener);
    if let Some(error) = flattener.error {
        return Err(error);
    }
    let mut generator = TokenBasedLuaGenerator::new(code);
    generator.write_block(&block);
    Ok(generator.into_string())
}

struct RequireFlattener<'a> {
    module: &'a str,
    error: Option<Error>,
}

impl NodeProcessor for RequireFlattener<'_> {
    fn process_function_call(&mut self, call: &mut FunctionCall) {
        if self.error.is_some() {
            return;
        }
        if !matches!(call.get_prefix(), Prefix::Identifier(id) if id.get_name() == "require") {
            return;
        }
        match call.mutate_arguments() {
            Arguments::String(string) => {
                if let Some(value) = string.get_string_value().map(str::to_string) {
                    match flatten(&value, self.module) {
                        Ok(Some(flattened)) => *string = StringExpression::from_value(flattened),
                        Ok(None) => {}
                        Err(error) => self.error = Some(error),
                    }
                }
            }
            Arguments::Tuple(tuple) => {
                for expression in tuple.iter_mut_values().take(1) {
                    if let Expression::String(string) = expression {
                        if let Some(value) = string.get_string_value().map(str::to_string) {
                            match flatten(&value, self.module) {
                                Ok(Some(flattened)) => {
                                    *expression =
                                        Expression::String(StringExpression::from_value(flattened));
                                }
                                Ok(None) => {}
                                Err(error) => self.error = Some(error),
                            }
                        }
                    }
                }
            }
            Arguments::Table(_) => {}
        }
    }
}

fn flatten(path: &str, module: &str) -> Result<Option<String>, Error> {
    if let Some(alias) = path.strip_prefix('@') {
        return match alias.split_once('/') {
            Some((_, subpath)) => Ok(Some(subpath.replace('/', "."))),
            None => Err(Error::BareAlias(alias.to_string())),
        };
    }

    let mut prefix = parent_module(module);
    let mut rest = path;
    loop {
        if let Some(r) = rest.strip_prefix("./") {
            rest = r;
        } else if let Some(r) = rest.strip_prefix("../") {
            rest = r;
            prefix = parent_module(&prefix);
        } else {
            break;
        }
    }
    if rest == path {
        return Ok(None);
    }
    let dotted = rest.replace('/', ".");
    if dotted.is_empty() {
        return Ok(None);
    }
    Ok(Some(join_module(&prefix, &dotted)))
}

fn parent_module(module: &str) -> String {
    match module.rsplit_once('.') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

fn join_module(prefix: &str, rest: &str) -> String {
    if prefix.is_empty() {
        rest.to_string()
    } else {
        format!("{prefix}.{rest}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_alias_requires() {
        assert_eq!(flatten("@dep/foo", "main").unwrap(), Some("foo".into()));
        assert_eq!(
            flatten("@dep/foo/bar", "main").unwrap(),
            Some("foo.bar".into())
        );
    }

    #[test]
    fn flattens_relative_requires() {
        assert_eq!(flatten("./foo", "main").unwrap(), Some("foo".into()));
        assert_eq!(flatten("./foo", "a.b").unwrap(), Some("a.foo".into()));
        assert_eq!(flatten("../foo", "a.b").unwrap(), Some("foo".into()));
        assert_eq!(flatten("../../foo", "a.b.c").unwrap(), Some("foo".into()));
        assert_eq!(
            flatten("./sub/foo", "main").unwrap(),
            Some("sub.foo".into())
        );
    }

    #[test]
    fn leaves_flat_requires() {
        assert_eq!(flatten("foo.bar", "main").unwrap(), None);
        assert_eq!(flatten("foo", "main").unwrap(), None);
    }

    #[test]
    fn rewrites_require_calls() {
        let out = flatten_requires("require('@dep/foo')", "main").unwrap();
        assert!(out.contains("foo"));
        assert!(!out.contains('@'));

        let out = flatten_requires("require('./foo')\nrequire('../bar')", "main").unwrap();
        assert!(!out.contains("./"));
        assert!(!out.contains("../"));

        let out = flatten_requires("require(foo)", "main").unwrap();
        assert!(out.contains("require(foo)"));
    }
}
