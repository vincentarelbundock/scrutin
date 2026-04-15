# Great Expectations

A Python data validation framework. Test files leave `CheckpointResult` or `ExpectationSuiteValidationResult` objects in module globals. The runner emits one event per `ExpectationValidationResult`. *Scrutin* auto-detects Great Expectations when a `tests/great_expectations/` directory is present.

## Directory structure

```
myproject/
├── pyproject.toml
├── src/
│   └── myproject/
│       └── __init__.py
└── tests/
    └── great_expectations/
        └── test_orders.py
```

## Minimal example

**tests/great_expectations/test_orders.py**

```python
import great_expectations as gx
import great_expectations.expectations as gxe
import pandas as pd

suite = gx.ExpectationSuite(name="orders_suite")
suite.add_expectation(gxe.ExpectColumnValuesToNotBeNull(column="id"))
suite.add_expectation(
    gxe.ExpectColumnValuesToBeBetween(column="value", min_value=0, max_value=100)
)

context = gx.get_context(mode="ephemeral")
source = context.data_sources.add_pandas("orders")
asset = source.add_dataframe_asset(name="orders")
batch_def = asset.add_batch_definition_whole_dataframe("batch")

df = pd.DataFrame({"id": [1, 2, 3], "value": [10, 20, 30]})
batch = batch_def.get_batch(batch_parameters={"dataframe": df})
result = batch.validate(suite)
```

Every top-level `ExpectationSuiteValidationResult` or `CheckpointResult` left in module globals after execution is picked up by the runner.

## Running

```bash
scrutin myproject              # TUI
scrutin -r plain myproject     # text output
```

## Configuration

No configuration is required. To override defaults in `.scrutin/config.toml`:

```toml
[[suite]]
tool = "great_expectations"
# defaults pick up tests/great_expectations/**/test_*.py and watch src/**/*.py + lib/**/*.py
```

## Dependencies

Great Expectations and pandas must be installed in the active virtual environment. If the import fails, the runner emits a single `error` event for the file; other suites are unaffected.
