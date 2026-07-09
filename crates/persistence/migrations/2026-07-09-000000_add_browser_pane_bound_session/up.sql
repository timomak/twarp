-- twarp 14o: persist the browser pane's Claude-session binding (14j) so the
-- globe/chat cross-links survive an app restart.
ALTER TABLE browser_panes ADD COLUMN bound_claude_session TEXT;
