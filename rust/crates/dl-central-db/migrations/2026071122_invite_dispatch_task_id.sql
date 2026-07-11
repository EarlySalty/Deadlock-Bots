-- Durable Referenz auf den laufenden Steam-Task des Playtest-Invites.
--
-- `handle_invite_command` wartet synchron auf steam-core (bis 45 s je Task).
-- Bricht der HTTP-Request vorher ab (die Discord-Bridge hatte 30 s Timeout,
-- ein Admin schließt den Client, der Prozess startet neu), dann ist die Zeile
-- zwar beansprucht, aber weder `invite_sent_at` noch das Audit sind gesetzt.
-- Ob Steam den Invite bekommen hat, weiß danach niemand mehr: der Task lief in
-- steam-core ja weiter. Der Claim verfällt nach 120 s und der Poller schickt
-- denselben Invite ein zweites Mal.
--
-- Mit der Task-ID kann der Poller den Ausgang nachschlagen, statt zu raten.
-- Gleiches Muster wie `steam.steam_friend_requests.task_id`.
ALTER TABLE steam.invite_requests
    ADD COLUMN IF NOT EXISTS dispatch_task_id BIGINT;
