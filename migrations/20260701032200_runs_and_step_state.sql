CREATE TYPE run_status AS ENUM (
    'pending',
    'in_progress',
    'failed',
    'complete'
);

CREATE TYPE pipeline_step AS ENUM (
    'pick_animal',
    'generate_video',
    'extract_frame',
    'image_to_3d',
    'render_reveal',
    'assemble',
    'upload',
    'record_published_video'
);

CREATE TYPE pipeline_step_status AS ENUM (
    'pending',
    'in_progress',
    'failed',
    'complete'
);

CREATE TABLE runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    date DATE NOT NULL UNIQUE,
    animal TEXT NOT NULL CHECK (length(trim(animal)) > 0),
    status run_status NOT NULL DEFAULT 'pending',
    current_step pipeline_step,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        status = 'failed'
        OR error IS NULL
    )
);

CREATE TABLE run_step_states (
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    step pipeline_step NOT NULL,
    status pipeline_step_status NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    error TEXT,
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (run_id, step),
    CHECK (
        completed_at IS NULL
        OR started_at IS NOT NULL
    ),
    CHECK (
        status = 'failed'
        OR error IS NULL
    )
);

CREATE INDEX runs_status_idx ON runs (status);
CREATE INDEX runs_date_status_idx ON runs (date, status);
CREATE INDEX run_step_states_status_idx ON run_step_states (status);

CREATE OR REPLACE FUNCTION set_updated_at_timestamp()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER runs_set_updated_at
BEFORE UPDATE ON runs
FOR EACH ROW
EXECUTE FUNCTION set_updated_at_timestamp();

CREATE TRIGGER run_step_states_set_updated_at
BEFORE UPDATE ON run_step_states
FOR EACH ROW
EXECUTE FUNCTION set_updated_at_timestamp();
