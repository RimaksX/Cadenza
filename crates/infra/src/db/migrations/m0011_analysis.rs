//! The background analysis work queue.

pub const SQL: &str = r#"
CREATE TABLE analysis_jobs (
    id            TEXT    PRIMARY KEY,
    media_file_id TEXT    NOT NULL REFERENCES media_files (id) ON DELETE CASCADE,

    kind          TEXT    NOT NULL CHECK (kind IN ('metadata', 'hash', 'features')),
    state         TEXT    NOT NULL DEFAULT 'queued'
                  CHECK (state IN ('queued', 'running', 'done', 'failed')),

    -- Higher runs first. Metadata beats hashing beats feature extraction: the
    -- library is unusable without titles but merely less clever without BPM.
    priority      INTEGER NOT NULL DEFAULT 0,

    attempts      INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    error         TEXT,

    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,

    -- A failure without a reason is a dead end for whoever debugs it.
    CHECK (state <> 'failed' OR error IS NOT NULL)
) STRICT;

-- What `claim_next` reads. Partial, so the index stays small once most of the
-- library is analysed and the table is mostly 'done' rows.
CREATE INDEX analysis_jobs_claim
    ON analysis_jobs (priority DESC, created_at)
    WHERE state = 'queued';

-- One outstanding job per file and kind. This is what makes enqueueing
-- idempotent: re-queuing a file already waiting is a constraint violation the
-- repository swallows, rather than a duplicate the worker does twice.
CREATE UNIQUE INDEX analysis_jobs_outstanding_key
    ON analysis_jobs (media_file_id, kind)
    WHERE state IN ('queued', 'running');
"#;
