-- Scrim-Teams: Coach als echter Discord-Nutzer statt Freitext.
-- Rein additiv: bestehende Spalten und die compile-checked query!-Makros des
-- Bots (dl-squads / dl-community) bleiben unberuehrt. Kein .sqlx-Cache-Rebuild noetig.

-- scrim.teams.coach ist ein Freitext-Name ("Leo", "Deniz") — nicht verknuepft, tippfehleranfaellig,
-- und veraltet, sobald jemand ein Team uebernimmt. Vor allem: Aus einem Namen kann der Bot keine
-- Discord-Rolle vergeben.
--
-- coach_discord_id verweist auf den echten Nutzer (Quelle fuers Dropdown: coaching.coaches,
-- alle aktiven Coaches haben eine discord_user_id). Damit bekommt der Coach die Team-Rolle
-- automatisch — und verliert sie wieder, wenn ein anderer das Team uebernimmt.
--
-- BEWUSST keine FK auf coaching.coaches: Ein Coach kann dort inaktiv werden, ohne dass die
-- Team-Zuordnung stillschweigend verschwindet — das waere ein Datenverlust, den keiner bemerkt.
-- scrim.teams.coach (Text) bleibt als Anzeige-Fallback fuer die 4 Bestandsteams erhalten.
ALTER TABLE scrim.teams
    ADD COLUMN IF NOT EXISTS coach_discord_id BIGINT;

-- Der Rollen-Sync fragt "welche Teams coacht dieser Nutzer?" — bei jedem Coach-Wechsel,
-- fuer Vorher- und Nachher-Zustand.
CREATE INDEX IF NOT EXISTS teams_coach_discord_id_idx
    ON scrim.teams (coach_discord_id)
    WHERE coach_discord_id IS NOT NULL;
