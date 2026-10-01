use crate::{model, schema};
use diesel::{connection::SimpleConnection, prelude::*};

const UPGRADE: &str =
    include_str!("../migrations/2026-10-01-000000_add_claude_history_path/up.sql");
const DOWNGRADE: &str =
    include_str!("../migrations/2026-10-01-000000_add_claude_history_path/down.sql");

#[test]
fn claude_history_migration_preserves_legacy_rows_and_round_trips_pinned_paths() {
    let mut connection = SqliteConnection::establish(":memory:").unwrap();
    connection
        .batch_execute(
            "CREATE TABLE claude_code_panes (
                id INTEGER PRIMARY KEY NOT NULL,
                kind TEXT NOT NULL DEFAULT 'claude_code',
                session_id TEXT, cwd TEXT,
                provider TEXT NOT NULL DEFAULT 'claude', spawn_origin TEXT
            );
            INSERT INTO claude_code_panes(id, session_id, cwd)
                VALUES (1, 'original-uuid', '/original');",
        )
        .unwrap();
    connection.batch_execute(UPGRADE).unwrap();
    let legacy = schema::claude_code_panes::table
        .find(1)
        .select(model::ClaudeCodePane::as_select())
        .first(&mut connection)
        .unwrap();
    assert_eq!(legacy.history_path, None);
    assert_eq!(legacy.session_id.as_deref(), Some("original-uuid"));
    assert_eq!(legacy.cwd.as_deref(), Some("/original"));

    let new = model::NewClaudeCodePane {
        id: 2,
        session_id: Some("mapped-uuid".into()),
        cwd: Some("/original".into()),
        provider: "claude".into(),
        spawn_origin: None,
        history_path: Some("/provider/projects/mapped/mapped-uuid.jsonl".into()),
    };
    diesel::insert_into(schema::claude_code_panes::table)
        .values(&new)
        .execute(&mut connection)
        .unwrap();
    let restored = schema::claude_code_panes::table
        .find(2)
        .select(model::ClaudeCodePane::as_select())
        .first(&mut connection)
        .unwrap();
    assert_eq!(restored.history_path, new.history_path);
    assert_eq!(restored.session_id, new.session_id);
    assert_eq!(restored.cwd, new.cwd);

    connection.batch_execute(DOWNGRADE).unwrap();
    let rows: Vec<(Option<String>, Option<String>)> = schema::claude_code_panes::table
        .select((
            schema::claude_code_panes::session_id,
            schema::claude_code_panes::cwd,
        ))
        .order(schema::claude_code_panes::id)
        .load(&mut connection)
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        (Some("original-uuid".into()), Some("/original".into()))
    );
}
