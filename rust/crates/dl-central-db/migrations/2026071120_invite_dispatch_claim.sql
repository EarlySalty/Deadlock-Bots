-- Atomarer Dispatch-Claim für /invite.
--
-- Ohne ihn machen Admin-Befehl und 60-s-Poller beide "Audit-Lookup ->
-- AUTH_SEND_PLAYTEST_INVITE -> Audit-Insert". Zwischen Lookup und Insert liegen
-- 25-45 s Steam-Task; in diesem Fenster sehen beide "kein Audit" und schicken
-- denselben Invite zweimal. Das kostet das Tageslimit des Bots und der
-- Eingeladene bekommt zwei Einladungen.
--
-- Wer die Spalte per bedingtem UPDATE erobert, darf senden. Der Timestamp
-- verfällt (stale-Claim), damit ein abgestürzter Sender die Zeile nicht
-- dauerhaft blockiert.
ALTER TABLE steam.invite_requests
    ADD COLUMN IF NOT EXISTS dispatch_claimed_at BIGINT;
