CREATE TABLE user_profile (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    display_name TEXT NOT NULL DEFAULT '' CHECK (length(display_name) <= 80)
);

INSERT INTO user_profile (id, display_name) VALUES (1, '');
