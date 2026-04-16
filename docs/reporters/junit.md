# JUnit XML

Writes a JUnit XML report alongside plain text output. Useful for CI platforms that parse JUnit results.

```bash
scrutin -r junit:report.xml
```

The report includes run metadata in `<properties>` and marks flaky tests. Plain text is still written to stdout, so a JUnit invocation is a drop-in replacement for `-r plain` when you also want an artifact for a test-results viewer.
