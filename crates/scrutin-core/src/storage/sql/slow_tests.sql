SELECT file, subject_name,
       AVG(duration_ms) AS avg_ms,
       MAX(duration_ms) AS max_ms,
       COUNT(*) AS runs
FROM results
WHERE subject_name != '' AND duration_ms > 0
GROUP BY file, subject_name
HAVING runs >= ?1 AND avg_ms > ?2
ORDER BY avg_ms DESC
LIMIT ?3
