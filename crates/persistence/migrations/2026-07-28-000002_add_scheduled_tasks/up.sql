-- twarp 20d: locally scheduled agent tasks (Automation > Scheduled Tasks).
-- Each row is a cron-scheduled headless agent run with an optional fallback
-- provider. `id` is a UUID string; times are unix seconds.
CREATE TABLE scheduled_tasks (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    prompt TEXT NOT NULL,
    cwd TEXT NOT NULL,
    -- 5-field cron expression (min hour dom month dow).
    schedule TEXT NOT NULL,
    -- 'claude' | 'codex'
    provider TEXT NOT NULL,
    fallback_provider TEXT,
    model TEXT,
    effort TEXT,
    permission_mode TEXT,
    enabled BOOL NOT NULL DEFAULT 1,
    catch_up BOOL NOT NULL DEFAULT 0,
    next_run_at BIGINT,
    created_at BIGINT NOT NULL
);

-- Run history, pruned to the newest ~20 rows per task on insert.
CREATE TABLE scheduled_task_runs (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL,
    started_at BIGINT NOT NULL,
    finished_at BIGINT,
    provider_used TEXT NOT NULL,
    -- 'running' | 'success' | 'error' | 'both_failed'
    outcome TEXT NOT NULL,
    session_id TEXT,
    summary TEXT
);
