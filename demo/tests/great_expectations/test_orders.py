"""great_expectations fixture for scrutin's smoke tests.

The scrutin great_expectations runner walks this file's module globals
after execution and emits one event per `ExpectationValidationResult`
found inside any `ExpectationSuiteValidationResult` / `CheckpointResult`.

Two suite results are left in scope:

  result_clean — every expectation passes (exercises `pass`).
  result_dirty — one expectation fails on each column (exercises `fail`).

Requires `great_expectations>=1.0` to be importable in the active venv.
If GE is not installed the file errors at import time and the runner
emits a single `error` event for `<file>`; other suites are unaffected.
"""

import great_expectations as gx
import great_expectations.expectations as gxe
import pandas as pd


def _build_suite():
    suite = gx.ExpectationSuite(name="orders_suite")
    suite.add_expectation(gxe.ExpectColumnValuesToNotBeNull(column="id"))
    suite.add_expectation(
        gxe.ExpectColumnValuesToBeBetween(column="value", min_value=0, max_value=100)
    )
    return suite


def _validate(df, suite_name):
    context = gx.get_context(mode="ephemeral")
    source = context.data_sources.add_pandas(suite_name)
    asset = source.add_dataframe_asset(name="orders")
    batch_def = asset.add_batch_definition_whole_dataframe("batch")
    batch = batch_def.get_batch(batch_parameters={"dataframe": df})
    return batch.validate(_build_suite())


# Clean dataframe — both expectations pass.
_clean_df = pd.DataFrame({"id": [1, 2, 3, 4, 5], "value": [10, 20, 30, 40, 50]})
result_clean = _validate(_clean_df, "clean")

# Dirty dataframe — null id and out-of-range value, both expectations fail.
_dirty_df = pd.DataFrame({"id": [1, 2, None, 4, 5], "value": [10, 200, 30, 40, 50]})
result_dirty = _validate(_dirty_df, "dirty")
