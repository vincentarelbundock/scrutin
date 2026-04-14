## fetch_remote() unconditionally stops(); the file errors out mid-run.
data <- fetch_remote("https://example.invalid")
expect_true(!is.null(data))
