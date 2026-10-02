use std::collections::{BTreeSet, HashSet};

use darklua_core::{
    nodes::{
        Expression, FieldExpression, FunctionAssignment, FunctionCall, IndexExpression, Prefix,
    },
    process::{NodeProcessor, NodeVisitor, Scope, ScopeVisitor},
    Parser, ParserError,
};

use super::shims::Shim;

/// Luau standard library usage detected in Lua source.
pub(super) struct Analysis {
    /// Shims required by the source.
    pub shims: BTreeSet<Shim>,
    /// Luau standard library symbols that cannot be shimmed.
    pub unsupported: Vec<String>,
}

impl Analysis {
    pub(super) fn new(code: &str) -> Result<Self, ParserError> {
        let mut block = Parser::default().parse(code)?;
        let mut collector = Collector::default();
        ScopeVisitor::visit_block(&mut block, &mut collector);
        Ok(Self {
            shims: collector.shims,
            unsupported: collector.unsupported,
        })
    }
}

#[derive(Default)]
struct Collector {
    scopes: Vec<HashSet<String>>,
    shims: BTreeSet<Shim>,
    unsupported: Vec<String>,
}

impl Collector {
    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().rev().any(|scope| scope.contains(name))
    }

    fn add(&mut self, shim: Shim) {
        self.shims.insert(shim);
    }

    fn reject(&mut self, symbol: String) {
        if !self.unsupported.contains(&symbol) {
            self.unsupported.push(symbol);
        }
    }

    fn record_global(&mut self, name: &str) {
        if self.is_local(name) {
            return;
        }
        match name {
            "task" | "buffer" => self.reject(name.to_string()),
            _ => {
                if let Ok(shim) = name.parse() {
                    self.add(shim);
                }
            }
        }
    }

    fn record_member(&mut self, base: &str, field: &str) {
        if self.is_local(base) {
            return;
        }
        if matches!(base, "task" | "buffer")
            || (base == "utf8" && matches!(field, "nfcnormalize" | "nfdnormalize" | "graphemes"))
        {
            self.reject(format!("{base}.{field}"));
            return;
        }
        if let Ok(shim) = format!("{base}.{field}").parse() {
            self.add(shim);
        }
    }
}

impl NodeProcessor for Collector {
    fn process_expression(&mut self, expression: &mut Expression) {
        if let Expression::Identifier(identifier) = expression {
            self.record_global(identifier.get_name());
        }
    }

    fn process_function_call(&mut self, call: &mut FunctionCall) {
        if let Prefix::Identifier(identifier) = call.get_prefix() {
            self.record_global(identifier.get_name());
        }
    }

    fn process_field_expression(&mut self, field: &mut FieldExpression) {
        if let Prefix::Identifier(base) = field.get_prefix() {
            self.record_member(base.get_name(), field.get_field().get_name());
        }
    }

    fn process_index_expression(&mut self, index: &mut IndexExpression) {
        if let Prefix::Identifier(base) = index.get_prefix() {
            if let Expression::String(key) = index.get_index() {
                if let Some(field) = key.get_string_value() {
                    self.record_member(base.get_name(), field);
                }
            }
        }
    }
}

impl Scope for Collector {
    fn push(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn pop(&mut self) {
        self.scopes.pop();
    }

    fn insert(&mut self, identifier: &mut String) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(identifier.to_string());
        }
    }

    fn insert_self(&mut self) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert("self".into());
        }
    }

    fn insert_local(&mut self, identifier: &mut String, _value: Option<&mut Expression>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(identifier.to_string());
        }
    }

    fn insert_local_function(&mut self, function: &mut FunctionAssignment) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(function.get_identifier().get_name().to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shims(code: &str) -> BTreeSet<Shim> {
        Analysis::new(code).unwrap().shims
    }

    fn unsupported(code: &str) -> Vec<String> {
        Analysis::new(code).unwrap().unsupported
    }

    #[test]
    fn detects_member_shims() {
        let found = shims("return table.create(10), string.split('a,b', ','), math.round(1.5)");
        assert!(found.contains(&Shim::TableCreate));
        assert!(found.contains(&Shim::StringSplit));
        assert!(found.contains(&Shim::MathRound));
        assert!(!found.contains(&Shim::Bit32));
    }

    #[test]
    fn detects_library_and_global_shims() {
        let found = shims("local b = bit32.band(1, 2); local t = typeof(b); return utf8.len('x')");
        assert!(found.contains(&Shim::Bit32));
        assert!(found.contains(&Shim::Typeof));
        assert!(found.contains(&Shim::Utf8));
    }

    #[test]
    fn respects_local_shadowing() {
        let found = shims("local table = { create = function() end }\nreturn table.create(10)");
        assert!(!found.contains(&Shim::TableCreate));

        let found = shims("local task = { spawn = function() end }\ntask.spawn(print)");
        assert!(
            unsupported("local task = { spawn = function() end }\ntask.spawn(print)").is_empty()
        );
        assert!(!found.contains(&Shim::Typeof));
    }

    #[test]
    fn rejects_unsupported_globals() {
        assert_eq!(unsupported("task.spawn(print)"), vec!["task.spawn"]);
        assert_eq!(
            unsupported("return buffer.create(4)"),
            vec!["buffer.create"]
        );
        assert_eq!(
            unsupported("return utf8.nfcnormalize('x')"),
            vec!["utf8.nfcnormalize"]
        );
        assert_eq!(unsupported("local t = task"), vec!["task"]);
    }
}
