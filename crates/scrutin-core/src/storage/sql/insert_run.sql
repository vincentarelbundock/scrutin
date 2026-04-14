INSERT INTO runs (
    run_id, timestamp, hostname, ci, scrutin_version,
    git_commit, git_branch, git_dirty,
    repo_name, repo_url, repo_root,
    build_number, build_id, build_name, build_url,
    os_platform, os_release, os_version, os_arch
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
