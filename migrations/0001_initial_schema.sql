CREATE TABLE users (
    id            TEXT    PRIMARY KEY NOT NULL,
    apple_subject TEXT    NOT NULL UNIQUE,
    display_name  TEXT,
    created_at_ms INTEGER NOT NULL
) STRICT;

-- Only SHA-256 digests of secrets are stored; a database leak does not leak usable tokens.
CREATE TABLE sessions (
    token_hash    BLOB    PRIMARY KEY NOT NULL,
    user_id       TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at_ms INTEGER NOT NULL,
    expires_at_ms INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_user_idx ON sessions (user_id);

CREATE TABLE sign_in_challenges (
    nonce_hash    BLOB    PRIMARY KEY NOT NULL,
    expires_at_ms INTEGER NOT NULL
) STRICT;

CREATE TABLE households (
    id                       TEXT    PRIMARY KEY NOT NULL,
    name                     TEXT    NOT NULL,
    feeding_interval_minutes INTEGER NOT NULL CHECK (feeding_interval_minutes BETWEEN 15 AND 720),
    created_at_ms            INTEGER NOT NULL
) STRICT;

CREATE TABLE memberships (
    household_id TEXT    NOT NULL REFERENCES households (id) ON DELETE CASCADE,
    user_id      TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role         TEXT    NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    joined_at_ms INTEGER NOT NULL,
    PRIMARY KEY (household_id, user_id)
) STRICT;

CREATE INDEX memberships_user_idx ON memberships (user_id);
CREATE UNIQUE INDEX memberships_single_owner_idx ON memberships (household_id) WHERE role = 'owner';

CREATE TABLE invites (
    code_hash           BLOB    PRIMARY KEY NOT NULL,
    household_id        TEXT    NOT NULL REFERENCES households (id) ON DELETE CASCADE,
    role                TEXT    NOT NULL CHECK (role IN ('editor', 'viewer')),
    created_by_user_id  TEXT    NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at_ms       INTEGER NOT NULL,
    expires_at_ms       INTEGER NOT NULL,
    accepted_by_user_id TEXT    REFERENCES users (id) ON DELETE SET NULL,
    accepted_at_ms      INTEGER
) STRICT;

CREATE INDEX invites_household_idx ON invites (household_id);
CREATE INDEX invites_created_by_idx ON invites (created_by_user_id);
CREATE INDEX invites_accepted_by_idx ON invites (accepted_by_user_id);

-- Nursing has a duration and a side; bottles (breast_milk, formula) have an amount. Never both.
CREATE TABLE feedings (
    id                 TEXT    PRIMARY KEY NOT NULL,
    household_id       TEXT    NOT NULL REFERENCES households (id) ON DELETE CASCADE,
    fed_at_ms          INTEGER NOT NULL,
    kind               TEXT    NOT NULL CHECK (kind IN ('nursing', 'breast_milk', 'formula')),
    amount_ml          INTEGER CHECK (amount_ml BETWEEN 1 AND 500),
    duration_minutes   INTEGER CHECK (duration_minutes BETWEEN 1 AND 120),
    breast_side        TEXT    CHECK (breast_side IN ('left', 'right', 'both')),
    created_by_user_id TEXT    REFERENCES users (id) ON DELETE SET NULL,
    created_at_ms      INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL,
    CHECK (
        (kind = 'nursing'
            AND amount_ml IS NULL AND duration_minutes IS NOT NULL AND breast_side IS NOT NULL)
        OR (kind IN ('breast_milk', 'formula')
            AND amount_ml IS NOT NULL AND duration_minutes IS NULL AND breast_side IS NULL)
    )
) STRICT;

CREATE INDEX feedings_household_fed_at_idx ON feedings (household_id, fed_at_ms DESC, id DESC);
CREATE INDEX feedings_created_by_idx ON feedings (created_by_user_id);
