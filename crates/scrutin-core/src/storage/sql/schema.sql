CREATE TABLE IF NOT EXISTS runs (
    run_id          TEXT PRIMARY KEY,
    timestamp       TEXT NOT NULL,
    hostname        TEXT,
    ci              TEXT,
    scrutin_version TEXT NOT NULL,

    git_commit      TEXT,
    git_branch      TEXT,
    git_dirty       INTEGER,

    repo_name       TEXT,
    repo_url        TEXT,
    repo_root       TEXT,

    build_number    TEXT,
    build_id        TEXT,
    build_name      TEXT,
    build_url       TEXT,

    os_platform     TEXT,
    os_release      TEXT,
    os_version      TEXT,
    os_arch         TEXT
);

CREATE TABLE IF NOT EXISTS results (
    run_id          TEXT NOT NULL,
    run_seq         INTEGER NOT NULL,
    file            TEXT NOT NULL,
    tool            TEXT NOT NULL,
    tool_version    TEXT,
    app_name        TEXT,
    app_version     TEXT,
    subject_kind    TEXT NOT NULL,
    subject_name    TEXT NOT NULL,
    subject_parent  TEXT,
    outcome         TEXT NOT NULL,
    duration_ms     INTEGER NOT NULL DEFAULT 0,
    retries         INTEGER NOT NULL DEFAULT 0,
    total           INTEGER,
    failed          INTEGER,
    fraction        REAL,
    FOREIGN KEY (run_id) REFERENCES runs(run_id)
);

CREATE INDEX IF NOT EXISTS idx_results_run_id       ON results(run_id);
CREATE INDEX IF NOT EXISTS idx_results_run_seq      ON results(run_seq);
CREATE INDEX IF NOT EXISTS idx_results_outcome      ON results(outcome);
CREATE INDEX IF NOT EXISTS idx_results_file_subject ON results(file, subject_name);
CREATE INDEX IF NOT EXISTS idx_results_tool         ON results(tool);

CREATE TABLE IF NOT EXISTS extras (
    run_id TEXT NOT NULL,
    key    TEXT NOT NULL,
    value  TEXT NOT NULL,
    PRIMARY KEY (run_id, key),
    FOREIGN KEY (run_id) REFERENCES runs(run_id)
);

CREATE TABLE IF NOT EXISTS dependencies (
    source_file TEXT NOT NULL,
    test_file   TEXT NOT NULL,
    PRIMARY KEY (source_file, test_file)
);

CREATE INDEX IF NOT EXISTS idx_dependencies_test ON dependencies(test_file);

CREATE TABLE IF NOT EXISTS hashes (
    file TEXT PRIMARY KEY,
    hash INTEGER NOT NULL
);
