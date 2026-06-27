-- twarp 07: remember the last-used Claude session settings (model, effort,
-- permission mode) so a freshly opened Claude pane inherits the PREVIOUS
-- session's settings instead of re-deriving them from the user's `claude`
-- shell alias every time. Single global row (id is always 0); the alias is
-- only consulted to seed this row the first time, before it has any values.
CREATE TABLE claude_session_defaults (
    id INTEGER PRIMARY KEY NOT NULL,
    model TEXT,
    effort TEXT,
    permission_mode TEXT
);
