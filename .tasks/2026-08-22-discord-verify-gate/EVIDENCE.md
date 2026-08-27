status: aktiv 2026-08-27

# Evidence: Discord Verify-Gate (Bestandsfundstellen)

Repo: /home/nathanael/Documents/Deadlock-Bots. Jede Zeile eine verifizierte Fundstelle.

## Rollen (add/remove vorhanden, Quarantaene-Rolle fehlt)
- `rust/crates/dl-discord/src/adapter.rs:610` `add_role` ruft `http.add_member_role(guild, user, role, reason)`.
- `rust/crates/dl-discord/src/adapter.rs:656` `remove_role` ruft `http.remove_member_role(...)`, Unknown-Member wird `PortError::MemberNotFound`.
- `rust/crates/dl-discord/src/adapter.rs:628` `create_role` vorhanden.
- `rust/crates/dl-community/src/onboarding.rs:10` `ONBOARD_COMPLETE_ROLE_ID = 1304216250649415771` (verifiziert-Rolle "Deadlocker", blendet nichts aus).
- Rollen-Aufrufer und Reaktion: `rust/bin/dl-bot/src/modglue.rs` (`.add_role/.remove_role`), `rust/bin/dl-bot/src/journeyglue.rs:419` (Reaktion auf Rollen-Gewinn).

## Kick (fehlt komplett)
- `rust/crates/dl-discord/src/adapter.rs:24` `DiscordAdapter`, haelt `http: Arc<Http>` (:25). Nur add/remove/create_role, move_voice, send_*. Kein Guild-Kick.
- Neu zu ergaenzen: `self.http.kick_member(GuildId, UserId, Some(reason))` bzw. `DELETE guilds/{}/members/{}`, analog `add_role` (:610).

## Join-Event
- `rust/crates/dl-discord/src/dispatcher.rs:135` `MemberEvent::Join` Felder: `guild_id`, `user_id`, `display_name`, `account_created_at: i64` (:140), `join_position` (:142), `is_bot: bool` (:143), `metadata: serde_json::Value` (:147).
- `rust/crates/dl-discord/src/gateway.rs:446` `guild_member_addition` ruft `publish_member(MemberEvent::Join{...})` (:457), metadata (:452).
- `rust/crates/dl-discord/src/dispatcher.rs:289` `subscribe_members()` (Broadcast).
- Handler-Vorbild: `rust/crates/dl-community/src/concierge.rs:7075` (tokio-spawn-Loop ueber `members.recv()`); `rust/crates/dl-community/src/leave_survey.rs:609`.
- `rust/crates/dl-community/src/invites.rs:33` `classify_join_source(meta, website_codes) -> String`: `"invite_link"` gibt "Persoenlicher Invite" (:63), `"vanity"` (:51), `"server_discovery"` (:68), Website-Code (:45). join_source steckt im metadata-JSON, kein Event-Feld.

## DM senden/empfangen plus Interaktionen
- Senden: `rust/bin/dl-bot/src/modglue.rs:3189` `send_dm_embed`; `rust/crates/dl-voice/src/glue.rs:378` `send_dm_body`; `rust/crates/dl-discord/src/adapter.rs:272` `send_raw_dm_public`.
- Empfangen: `rust/crates/dl-discord/src/dispatcher.rs:92` `MessageEvent.guild_id: Option<u64>` (DM ist None), publiziert `rust/crates/dl-discord/src/gateway.rs:418`, Abo `dispatcher.rs:285` `subscribe_messages()`. Vorbild `rust/crates/dl-community/src/concierge.rs:7095` Message-Loop ruft `handle_routed_message`.
- Interaktionen: `rust/crates/dl-discord/src/interactions.rs:21` `BridgeInteraction` (`custom_id` :23), Trait :194 `InteractionHandler`, Router :233 `on_custom_id` (:245), Praefix-Route (:250). Registrier-Vorbild: `rust/bin/dl-bot/src/onboardglue.rs:22` `match interaction.custom_id`, :49 `router.on_custom_id("steam:openid", ...)`.

## LLM-Judge
- `rust/crates/dl-ai/src/lib.rs:120` `async fn generate_text(GenerateRequest) -> Option<String>`; `GenerateRequest` (:55).
- `rust/crates/dl-ai/src/chat_provider.rs:179` `trait ChatProvider`, `async fn chat` (:180). Provider ab :647/:655/:902.
- `rust/crates/dl-ai/src/lib.rs:42` `DEFAULT_FIREWORKS_MODEL = "accounts/fireworks/models/deepseek-v4-flash-0731"`.
- Judge-Vorbild (zweistufig, JSON-Verdict): `rust/crates/dl-moderation/src/moderation_system.rs` (`log_judge_decision` :623, `decision_details` :680), gespawnt `rust/bin/dl-bot/src/main.rs:1574`.

## PG-Persistenz
- Migrations `rust/crates/dl-central-db/migrations/`; Konvention alt `0001..0015` (`0007_bot.sql`), neu `2026MMDDNN_name.sql` (`2026070913_invite_requests.sql`). Muster `CREATE TABLE IF NOT EXISTS <schema>.<table>`.
- Verwaiste Tabelle `rust/crates/dl-central-db/migrations/0007_bot.sql:176` `bot.onboarding_pending_verify (user_id BIGINT PK, channel_id BIGINT, updated_at TIMESTAMPTZ)` ist zu duenn (kein guild_id, attempts, deadline, state).
- sqlx-Store-Vorbild: `rust/crates/dl-community/src/invites.rs:171` `sqlx::query!`; Select `rust/crates/dl-tierlist/src/data.rs:40` `hero_catalog(pool)`.

## Scheduler
- Muster `pub fn spawn(...)` aus `rust/bin/dl-bot/src/main.rs` (concierge :1609, retention :1641, leave_survey :1646, moderation :1574).
- Periodik-Vorbild: `rust/crates/dl-community/src/retention.rs:1023` `loop { sleep(3600s) }`, `SYNC_INTERVAL` (:31).

## Heroliste
- `rust/crates/dl-tierlist/src/data.rs:40` `hero_catalog(pool)` liest `SELECT hero_id, name FROM tierlist.deadlock_heroes`; Slug-Helper `data.rs:14` `slugify_hero`.
