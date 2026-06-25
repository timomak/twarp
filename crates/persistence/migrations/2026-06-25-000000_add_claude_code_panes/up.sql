-- twarp 07: persist Claude Code panes so an open conversation reopens after a
-- crash or Cmd+Q. twarp keeps no transcript of its own; the row only records
-- the `claude` session id (and its originating cwd) so the pane can be
-- restored via `claude --resume` against the `.jsonl` `claude` already wrote.
CREATE TABLE claude_code_panes (
    id INTEGER PRIMARY KEY NOT NULL REFERENCES pane_nodes(id),
    kind TEXT NOT NULL DEFAULT 'claude_code',
    session_id TEXT,
    cwd TEXT
);
