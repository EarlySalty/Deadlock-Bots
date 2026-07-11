-- Beweis, dass Steam den Playtest-Invite bereits bekommen hat.
--
-- `dispatch_claimed_at` schützt nur die ABSICHT ("ich arbeite dran"), nicht die
-- TAT. Der Beweis für den Versand war bisher der Audit-Eintrag — der aber erst
-- NACH dem Versand geschrieben wird und genau dabei fehlschlagen kann. Folge:
-- Zeile bleibt stehen, Claim läuft nach 120 s ins Stale, der nächste Pass
-- sieht "kein Audit" und schickt denselben Invite ein zweites Mal.
--
-- `invite_sent_at` wird unmittelbar nach dem erfolgreichen Steam-Versand
-- gesetzt, VOR dem Audit-Insert. Ist es gesetzt, wird nie wieder gesendet —
-- dann fehlt nur noch das Audit und wird nachgeholt.
ALTER TABLE steam.invite_requests
    ADD COLUMN IF NOT EXISTS invite_sent_at BIGINT;
