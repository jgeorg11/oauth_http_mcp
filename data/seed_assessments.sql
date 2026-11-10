WITH RECURSIVE months(n) AS (
    VALUES(0)
    UNION ALL
    SELECT n + 1 FROM months WHERE n < 11
)
INSERT OR IGNORE INTO project_spend (
    project_id, period, planned_minor, actual_minor
)
SELECT
    p.id,
    date('now', 'start of month', printf('-%d months', m.n)),
    100000 + p.id * 100,
    CASE
        WHEN p.id % 3 = 0 THEN 140000
        ELSE 95000
    END + p.id * 100
FROM projects AS p
CROSS JOIN months AS m;

INSERT OR IGNORE INTO project_milestones (
    id, project_id, title, due_date, completed_date
)
SELECT
    10000 + id,
    id,
    'Delivery review',
    date(
        'now',
        CASE
            WHEN id % 3 = 0 THEN '-14 days'
            ELSE '+14 days'
        END
    ),
    NULL
FROM projects;

INSERT OR IGNORE INTO project_updates (
    id, project_id, reported_date, body
)
SELECT
    20000 + id,
    id,
    date('now', '-1 day'),
    CASE
        WHEN id % 3 = 0 THEN
            'A supplier delay threatens delivery. The owner has not yet approved the mitigation plan.'
        ELSE
            'The latest review reports steady progress. No new delivery blockers were reported.'
    END
FROM projects;

