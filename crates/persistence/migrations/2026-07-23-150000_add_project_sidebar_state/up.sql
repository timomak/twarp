ALTER TABLE tabs ADD COLUMN project_root BLOB;
ALTER TABLE tabs ADD COLUMN project_root_initialized BOOLEAN;

ALTER TABLE windows ADD COLUMN projects_sidebar_open BOOLEAN;
ALTER TABLE windows ADD COLUMN projects_sidebar_width FLOAT;
ALTER TABLE windows ADD COLUMN right_tool_kind INTEGER;
ALTER TABLE windows ADD COLUMN right_tool_open BOOLEAN;
ALTER TABLE windows ADD COLUMN files_tool_width FLOAT;
ALTER TABLE windows ADD COLUMN code_review_tool_width FLOAT;
