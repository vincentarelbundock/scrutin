# Plain

A compact text summary, suitable for CI and scripting.

```bash
scrutin -r plain
```

```
● test-model.R          4 passed              87ms
✗ test-plots.R          2 passed  1 failed    43ms

── Failures ──

  FAIL  make_plot handles empty data
    `result` is NULL, not an S3 object with class "ggplot"
    At: tests/testthat/test-plots.R:23

7 passed  1 failed  0 skipped  ∷  143ms
```

The reporter prints one line per file followed by a `Failures` block with the failure messages and source locations, and a final counts line. Exit code is 0 when every file passes and 1 when any file fails.
