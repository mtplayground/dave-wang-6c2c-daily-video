CREATE TYPE rotation_animal AS ENUM (
    'dog',
    'cat',
    'rabbit',
    'pig',
    'chicken'
);

CREATE TABLE rotation_state (
    key TEXT PRIMARY KEY DEFAULT 'animal_rotation',
    current_position SMALLINT NOT NULL DEFAULT 0,
    current_animal rotation_animal NOT NULL DEFAULT 'dog',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (key = 'animal_rotation'),
    CHECK (current_position BETWEEN 0 AND 4),
    CHECK (
        (current_position = 0 AND current_animal = 'dog')
        OR (current_position = 1 AND current_animal = 'cat')
        OR (current_position = 2 AND current_animal = 'rabbit')
        OR (current_position = 3 AND current_animal = 'pig')
        OR (current_position = 4 AND current_animal = 'chicken')
    )
);

INSERT INTO rotation_state (key, current_position, current_animal)
VALUES ('animal_rotation', 0, 'dog');

CREATE TRIGGER rotation_state_set_updated_at
BEFORE UPDATE ON rotation_state
FOR EACH ROW
EXECUTE FUNCTION set_updated_at_timestamp();
