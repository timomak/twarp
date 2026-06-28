-- twarp 14b: persist Browser panes by last committed URL only. In-page
-- state is intentionally not restored.
CREATE TABLE browser_panes (
    id INTEGER PRIMARY KEY NOT NULL REFERENCES pane_nodes(id),
    kind TEXT NOT NULL DEFAULT 'browser',
    url TEXT
);
