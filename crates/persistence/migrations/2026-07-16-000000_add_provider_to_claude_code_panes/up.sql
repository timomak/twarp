-- twarp 18a: record which runtime provider owns an agent pane. Existing panes
-- restore as Claude by default.
ALTER TABLE claude_code_panes ADD COLUMN provider TEXT NOT NULL DEFAULT 'claude';
