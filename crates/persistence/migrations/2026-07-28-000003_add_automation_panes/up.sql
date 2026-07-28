-- twarp 20e: persist Automation panes (Scheduled Tasks / Skills / MCPs) by
-- which page they display. There is no other state to restore.
CREATE TABLE automation_panes (
    id INTEGER PRIMARY KEY NOT NULL REFERENCES pane_nodes(id),
    kind TEXT NOT NULL DEFAULT 'automation',
    page TEXT NOT NULL
);
