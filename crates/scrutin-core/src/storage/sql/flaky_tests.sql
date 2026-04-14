WITH recent AS (
    SELECT run_id FROM runs ORDER BY timestamp DESC LIMIT ?1
),
stats AS (
    SELECT file, subject_name,
           SUM(CASE WHEN outcome IN ('fail','error') THEN 1 ELSE 0 END) AS failures,
           SUM(CASE WHEN retries > 0 AND outcome = 'pass' THEN 1 ELSE 0 END) AS retry_passes,
           COUNT(*) AS total
    FROM results
    WHERE run_id IN (SELECT run_id FROM recent) AND subject_name != ''
    GROUP BY file, subject_name
)
SELECT file, subject_name, failures, retry_passes, total
FROM stats
WHERE total >= ?2
  AND ((failures > 0 AND failures < total) OR retry_passes > 0)
