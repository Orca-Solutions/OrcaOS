-- SPDX-FileCopyrightText: The OrcaOS Authors
-- SPDX-License-Identifier: GPL-3.0-or-later

-- Scenario A on the conventional composition: what PostgreSQL holds.
--
-- Scorecard: configuration (counted separately from code). Category: grants
-- (journal, grant records, budget), approvals (proposals).

-- One journal writer per run. A command locks its run's row for one
-- transaction, so the row is the single point of order.
CREATE TABLE IF NOT EXISTS journals (
    run text PRIMARY KEY,
    writer jsonb NOT NULL
);

-- A root activity's CPU allowance, and every reservation against it.
CREATE TABLE IF NOT EXISTS budgets (
    run text,
    root text,
    cpu_ms bigint NOT NULL,
    PRIMARY KEY (run, root)
);

CREATE TABLE IF NOT EXISTS reservations (
    run text,
    reservation text,
    activity text NOT NULL,
    root text NOT NULL,
    cpu_ms bigint NOT NULL,
    result text NOT NULL,
    released boolean NOT NULL DEFAULT false,
    at_ms bigint NOT NULL,
    PRIMARY KEY (run, reservation)
);

-- What the approval service shows the principal. A retried submission
-- carries the same key and gets the proposal it already made.
CREATE TABLE IF NOT EXISTS proposals (
    run text,
    proposal_id text,
    activity text NOT NULL,
    message text NOT NULL,
    payload_hash text NOT NULL,
    submission text,
    PRIMARY KEY (run, proposal_id),
    UNIQUE (run, submission)
);
