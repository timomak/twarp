pub mod model;
pub mod schema;

#[cfg(all(test, feature = "local_fs"))]
mod claude_history_tests;

#[cfg(feature = "local_fs")]
pub const MIGRATIONS: diesel_migrations::EmbeddedMigrations =
    diesel_migrations::embed_migrations!("migrations");
