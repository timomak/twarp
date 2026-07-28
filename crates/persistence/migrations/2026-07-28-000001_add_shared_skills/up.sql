-- twarp 20c: per-provider enable toggles for the twarp-managed shared-skills
-- store (~/.twarp/skills). Content on disk is the source of truth; this table
-- only records which providers each skill is materialized for. One row per
-- skill directory name.
CREATE TABLE shared_skills (
    name TEXT PRIMARY KEY NOT NULL,
    enabled_claude BOOL NOT NULL DEFAULT 1,
    enabled_codex BOOL NOT NULL DEFAULT 1
);
