CREATE TYPE artifact_type AS ENUM (
    'raw_video',
    'frame',
    'glb',
    'reveal_clip',
    'final_mp4'
);

CREATE TABLE artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    artifact_type artifact_type NOT NULL,
    storage_key TEXT NOT NULL CHECK (length(trim(storage_key)) > 0),
    content_type TEXT,
    byte_size BIGINT CHECK (byte_size IS NULL OR byte_size >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (run_id, artifact_type),
    CHECK (storage_key NOT LIKE '/%'),
    CHECK (storage_key NOT LIKE '%..%')
);

CREATE TABLE published_videos (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL UNIQUE REFERENCES runs(id) ON DELETE RESTRICT,
    date DATE NOT NULL UNIQUE,
    animal TEXT NOT NULL CHECK (length(trim(animal)) > 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    final_video_storage_key TEXT NOT NULL CHECK (length(trim(final_video_storage_key)) > 0),
    published_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (final_video_storage_key NOT LIKE '/%'),
    CHECK (final_video_storage_key NOT LIKE '%..%')
);

CREATE INDEX artifacts_run_id_idx ON artifacts (run_id);
CREATE INDEX artifacts_type_idx ON artifacts (artifact_type);
CREATE INDEX published_videos_published_at_idx ON published_videos (published_at DESC);
CREATE INDEX published_videos_date_idx ON published_videos (date DESC);

CREATE TRIGGER artifacts_set_updated_at
BEFORE UPDATE ON artifacts
FOR EACH ROW
EXECUTE FUNCTION set_updated_at_timestamp();

CREATE TRIGGER published_videos_set_updated_at
BEFORE UPDATE ON published_videos
FOR EACH ROW
EXECUTE FUNCTION set_updated_at_timestamp();
