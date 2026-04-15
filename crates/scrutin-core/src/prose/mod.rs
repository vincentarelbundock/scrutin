//! Prose-quality slice of scrutin: tools that lint natural-language text
//! (READMEs, docstrings, markdown). Unlike the R and Python trees this slice
//! has no single "language" attached; each plugin declares its own target
//! extensions via `default_run`.
//!
//! Currently hosts the `skyspell` spell-check plugin.

use std::sync::Arc;

use crate::project::plugin::Plugin;

pub mod skyspell;

/// Every prose plugin compiled into the binary. Called by the central plugin
/// registry in `project::plugin::all_plugins()`.
pub fn plugins() -> Vec<Arc<dyn Plugin>> {
    vec![Arc::new(skyspell::plugin::SkyspellPlugin)]
}
