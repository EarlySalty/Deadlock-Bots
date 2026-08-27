-- Discord Verify-Gate: Pre-Gate gegen Scam-Accounts, die frische Konten per
-- DM anschreiben. Neue Mitglieder mit einem Konto juenger als die Schwelle, die
-- nicht ueber einen persoenlichen Invite kommen, landen in Quarantaene und
-- muessen eine feste Frage beantworten. Diese Tabellen halten den offenen
-- Zustand (pending) und die Liste der ueber das Gate gekickten Accounts, damit
-- ein Rejoin nach Kick freien Zugang bekommt.

CREATE TABLE IF NOT EXISTS bot.verify_gate_pending (
    guild_id      BIGINT      NOT NULL,
    user_id       BIGINT      NOT NULL,
    dm_channel_id BIGINT      NULL,
    attempts      INT         NOT NULL DEFAULT 0,
    state         TEXT        NOT NULL DEFAULT 'awaiting_start',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    deadline_at   TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (guild_id, user_id)
);

-- Der Frist-Kick-Scheduler laeuft periodisch ueber abgelaufene Deadlines.
CREATE INDEX IF NOT EXISTS idx_verify_gate_pending_deadline
    ON bot.verify_gate_pending (deadline_at);

CREATE TABLE IF NOT EXISTS bot.verify_gate_kicked (
    guild_id  BIGINT      NOT NULL,
    user_id   BIGINT      NOT NULL,
    kicked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, user_id)
);
