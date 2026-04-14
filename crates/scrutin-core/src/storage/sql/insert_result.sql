INSERT INTO results (
    run_id, run_seq, file, tool, tool_version, app_name, app_version,
    subject_kind, subject_name, subject_parent, outcome,
    duration_ms, retries, total, failed, fraction
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
