# Qualifizierte Twitch-Einladungen

Discord dokumentiert für API-Fehlercode 30016 ein Maximum von 1000 aktiven Einladungen pro Guild. Referenz: https://github.com/discord/discord-api-docs/blob/ce076f016923cc841774dc51d95bde0eb25c4dcb/developers/topics/opcodes-and-status-codes.mdx

Persönliche Twitch-Einladungen werden deshalb nicht unbegrenzt bis an das Guild-Limit erzeugt. Der Broker verwendet `twitch_invites.personal_links_per_channel_max` aus `config/bot.toml`. Sobald der Grenzwert eines Zielkanals erreicht ist, liefert er den Channel-Fallback aus und erzeugt keinen weiteren Discord-Invite.

Ein Join wird nur durch einen live beobachteten Member-Join qualifikationsfähig. Rückwirkend importierte Mitglieder werden nicht aufgenommen. Der Join bleibt zunächst `pending`. Frühestens 14 Tage nach dem Join kann er `qualified` werden, wenn bis dahin kein Leave vorliegt und innerhalb der ersten 30 Tage entweder eine Voice-Session mit mindestens 15 Minuten außerhalb konfigurierter Staging-Kanäle vorliegt oder mindestens fünf Nachrichten an mindestens zwei verschiedenen Kalendertagen erfasst wurden. Nach 30 Tagen wird ein noch nicht qualifizierter Join `expired`. Terminale Zustände werden nicht zurückgesetzt.

Die Message-Prüfung verwendet ausschließlich `activity.message_metadata_events`; Nachrichteninhalte werden weder gelesen noch über den internen Endpoint ausgegeben. Voice-Sessions stammen aus `activity.voice_session_log`. AFK-Sitzungen werden bereits vom Voice-Tracker nicht als Sessions geschrieben; zusätzliche Staging-Kanäle kommen aus `twitch_invites.staging_channel_ids`.

Streamer-Referrals nutzen den bestehenden Partner-Link-Pfad `bot.streamer_link_intents`. Der Twitch-Invite-Sync spiegelt zusätzlich Twitch-ID und aktiven Partnerstatus. Sobald ein über einen persönlichen Invite beigetretenes Discord-Mitglied über den bestehenden Link-Pfad einem Twitch-Login zugeordnet ist und dieser Login aktiver Partner wird, wird genau ein `streamer_referral` in `activity.streamer_referral_credits` gespeichert. Der Werber muss zu diesem Zeitpunkt ebenfalls aktiver Partner sein. Der Credit ist idempotent und wird nicht gelöscht.
