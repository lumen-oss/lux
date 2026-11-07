# Style guide

## Designing for qualified use

We design our modules for qualified use.
See [Designing Haskell libraries for qualified import](https://mrcjkb.dev/posts/2026-10-06-design-for-qualified-import.html)
for the rationale.

### Modules

A type is typically defined in a module whose final component is the type's name.
The components before it are the type's namespace, which re-exports the type:

```
lockfile/mod.rs          // pub use constraint::Constraint;
lockfile/constraint.rs   // Constraint

operations/mod.rs        // pub use download::Download;
operations/download.rs   // Download
```

Constructors, functions that operate on the type, and related definitions are
defined in the type's module, not in the namespace.

```rust
// lockfile/constraint.rs
pub enum Constraint { .. }

impl Constraint {
    pub fn to_string_opt(&self) -> Option<String> { .. }
}
```

### Module names

A child module is named after the type it defines, and does not repeat its namespace.
The namespace supplies the context, so the type drops it.

```rust
// Do
// lockfile/guard.rs
pub struct Guard { .. }

// lockfile/mod.rs
pub use guard::Guard;

// call site
lockfile::Guard

// Don't
// lockfile/lockfile_guard.rs
pub struct LockfileGuard { .. }

// call site
lockfile::LockfileGuard
```

### Call sites

Types are qualified by the namespace; their functions by the type's module.

```rust
// Do
use crate::{lockfile, operations};
use crate::operations::download;

lockfile::Constraint::default();
operations::Download::new(&req, &config);
download::src_rock(&package, server, config).await?;

// Don't
lockfile::constraint::Constraint::default();
operations::download::Download::new(&req, &config);
```

If the module is named after the type, the namespace is empty, so the type is
imported unqualified.

```rust
// Do
use crate::project::Project;

fn load(project: &Project) { .. }

// Don't
fn load(project: &project::Project) { .. }
```

If a type name repeats its namespace, the type is imported unqualified.

```rust
// Do
Paths::new(&tree)?;

// Don't
path::Paths::new(&tree)?;
```

Traits must be in scope for method resolution; import them instead of qualifying them.

```rust
// Do
use crate::hash::HasIntegrity;

let hash = rockspec.hash().await?;

// Don't
let hash = crate::hash::HasIntegrity::hash(&rockspec).await?;
```
