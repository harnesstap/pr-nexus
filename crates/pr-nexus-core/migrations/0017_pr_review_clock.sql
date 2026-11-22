CREATE TABLE pr_review_clock (
    pr_id INTEGER PRIMARY KEY REFERENCES pull_requests(id) ON DELETE CASCADE,
    clock TEXT NOT NULL
);

CREATE TABLE viewer_team (
    account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    org TEXT NOT NULL,
    slug TEXT NOT NULL,
    PRIMARY KEY (account_id, org, slug)
);

CREATE TABLE viewer_team_state (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    known INTEGER NOT NULL
);
