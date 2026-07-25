PRAGMA foreign_keys = ON;

DROP VIEW IF EXISTS hidden_join;
DROP TABLE IF EXISTS users;

CREATE TABLE users (
    id           INTEGER PRIMARY KEY NOT NULL,
    email        TEXT NOT NULL UNIQUE,
    display_name TEXT,
    active       BOOLEAN NOT NULL DEFAULT TRUE CHECK (active IN (FALSE, TRUE))
);

INSERT INTO users (email, display_name, active) VALUES
    ('ada@example.test',   'Ada', TRUE),
    ('grace@example.test', NULL,  TRUE),
    ('alan@example.test',  'Alan', FALSE);
