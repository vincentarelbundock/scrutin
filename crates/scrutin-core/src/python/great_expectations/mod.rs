//! great_expectations tool support: plugin impl + tool constants.
//!
//! great_expectations is a data-validation framework, not a unit-test
//! framework. A GE "test file" under scrutin is a plain Python script that,
//! when executed, leaves one or more `CheckpointResult` /
//! `ExpectationSuiteValidationResult` objects in its module globals. The
//! runner walks those objects and emits one event per
//! `ExpectationValidationResult`.
//!
//! This is the Python analogue of the `r/pointblank/` plugin: declarative
//! result objects in module scope, runner introspects them. We deliberately
//! do *not* discover GE checkpoints under `gx/` ourselves: composing the
//! validation is the user's job, scrutin only runs files and reads results.

pub mod plugin;

/// Conventional path (relative to the project root) where GE test files
/// live. Cohabits cleanly with `tests/` (pytest).
pub const TEST_DIR: &str = "tests/great_expectations";
