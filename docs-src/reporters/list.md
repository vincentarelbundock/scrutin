# List

Lists the test files that would run without actually running them. Useful for verifying filter patterns.

```bash
scrutin -r list
```

Honors every include, exclude, and filter group in `.scrutin/config.toml` and on the command line, so it doubles as a quick way to confirm that a `--set filter.include=...` pattern selects what you expect before paying the cost of a real run.
