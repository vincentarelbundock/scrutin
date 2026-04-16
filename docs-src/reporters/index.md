# Reporters

Reporters are one-shot outputs meant for CI and scripting. They run through your test files once, emit a structured result, and exit: no watch mode, no interactive drill-in. Pick one with `-r` / `--reporter`:

```bash
scrutin -r plain                 # compact text summary
scrutin -r junit:report.xml      # JUnit XML + plain text
scrutin -r github                # GitHub Actions annotations + step summary
scrutin -r list                  # list matching files, no execution
```

Exit code is 0 when every file passes and 1 when any file fails, so reporters slot cleanly into shell pipelines and CI gates.

For live, interactive views of a run (terminal UI, browser dashboard, editor panels), see [Frontends](../frontends/index.md).

## Available reporters

- [Plain](plain.md): compact text summary for terminals and CI logs
- [JUnit XML](junit.md): XML artifact for CI platforms that parse JUnit results
- [GitHub Actions](github.md): inline PR annotations and a job step summary
- [List](list.md): print the test files that would run, without running them
