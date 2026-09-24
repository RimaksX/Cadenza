//! Migration bookkeeping.
//!
//! This table has to exist before any other migration can be recorded, so it is
//! migration 1 rather than something the runner creates behind the scenes: the
//! table that tracks schema changes is itself part of the schema.

pub const SQL: &str = r#"
CREATE TABLE schema_migrations (
    version    INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    -- Unix milliseconds, like every other instant in this schema.
    applied_at INTEGER NOT NULL
) STRICT;
"#;
