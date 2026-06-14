# Cutover-Gap-Audit Python-live → Rust (2026-06-14)

Erschoepfender, adversarial verifizierter Paritaets-Audit (89 Agenten, 6 Subsysteme).
72 bestaetigte Luecken, 11 widerlegt (Falsch-Positive). Interne Doku, kein Changelog.

**Kernschluss:** Web-Cutover nah (alle portierbaren Routen erledigt; Rest architektonisch
un-portierbar = Frontend-Panel-Ausblendung). Bot-Cutover weit (Prefix-Dispatch, viele
Slash-Commands + ganze Cogs fehlen).


## dashboard-http

_12 Luecken (von 14 geprueft)._

- **[BLOCKER]** `GET /api/status` <missing>
  - Core polling endpoint of the admin SPA: service/static/dashboard.html fetches /api/status to populate bot info, cog tree, namespaces, blocked list, lifecycle snapshot, health checks, standalone snapshot, auth/csrf token. grep over rust/ (crates/+bin/) finds NO route or handler for /api/status; dl-da
  - Py: `service/dashboard.py:6642 (route reg line 415)`
- **[BLOCKER]** `POST /api/deadlock/heroes (upsert hero)` <missing>  ✅ ERLEDIGT 6fc0456 (DB-Core; Steam-Sync deferred)
  - require_full_access admin write: creates/updates a deadlock_heroes row + replaces build snapshot + optional sync. Rust deadlock.rs (grep) has only deadlock_config, deadlock_config_update, deadlock_heroes(GET) — NO upsert_hero. web.rs:308 registers /api/deadlock/heroes with GET only (no .post()). SPA
  - Py: `service/dashboard.py:5861 (route reg 446)`
- [degraded] `POST /api/bot/restart` <missing>
  - SPA calls /api/bot/restart (dashboard.html). No Rust route/handler anywhere in rust/. Restarts the bot via lifecycle/nssm with cooldown+lock. Admin convenience button dies (404); operator can still restart via systemctl.
  - Py: `service/dashboard.py:6746 (route reg line 416)`
- [degraded] `POST /api/dashboard/restart` <missing>
  - SPA calls /api/dashboard/restart. No Rust equivalent. Restarts the dashboard process. Admin convenience button dies (404); systemd restart still works manually.
  - Py: `service/dashboard.py:6846 (route reg line 417)`
- [degraded] `POST /api/cogs/reload | load | unload | reload-all | reload-namespace | block | unblock | discover` <missing>
  - All 8 cog-management POSTs are called by the SPA cog panel. None exist in rust/. They operate on the live discord.py extension system (bot.reload_cog, bot.unload_many, bot.is_namespace_blocked, bot.auto_discover_cogs) which has no analogue in the Rust bot (no runtime cog hot-(un)load). So the whole 
  - Py: `service/dashboard.py:7129/7163/7185/7218/7226/7264/7292/7256 (route reg 418-424,466)`
- [degraded] `GET /api/logs + GET /api/logs/{name}` <missing>
  - SPA calls /api/logs (index) and /api/logs/{name}?lines= (tail). No Rust route/handler. Lists log files and tails them for the admin log viewer. Log-viewer tab breaks (404); logs still readable via journalctl/filesystem.
  - Py: `service/dashboard.py:7322 / 7327 (route reg 467-468)`
- [degraded] `Standalone-bot manager routes (GET /api/standalone, GET .../logs, POST .../start|stop|restart|autostart|command)` <missing>
  - SPA calls /api/standalone (4x) plus per-key logs/start/stop/restart/autostart/command. None exist in rust/. Drives the 'standalone bots' management panel via a StandaloneManager (start/stop/tail/command other bot processes). Also note _handle_status embeds _collect_standalone_snapshot, so even the s
  - Py: `service/dashboard.py:7362/7367/7387/7411/7435/7456/7506 (route reg 469-478)`
- [degraded] `POST /api/repo-activity/refresh` <missing>  ✅ ERLEDIGT 839fc1a
  - SPA 'Neu sammeln' button POSTs here. GET /api/repo-activity (the read) IS ported (web.rs:295 -> crate::repo_activity::repo_activity), but the refresh POST is not in rust/. Python shells out to the dl-repostats binary then returns the artifact. The development tab still shows cached data via the port
  - Py: `service/dashboard.py:6615 (route reg 435)`
- [degraded] `DELETE /api/deadlock/heroes/{hero_id}` <missing>  ✅ ERLEDIGT 6fc0456
  - Admin write to remove a deadlock hero entry. No Rust route (web.rs has no {hero_id} route; deadlock.rs has no delete_hero). Hero deletion impossible after cutover.
  - Py: `service/dashboard.py:6030 (route reg 447)`
- [degraded] `POST /api/deadlock/heroes/{hero_id}/sync` <missing>
  - Admin action to sync a single hero's build snapshot (drives Steam build delivery). No Rust route/handler (deadlock.rs has no sync_hero). Per-hero re-sync button breaks.
  - Py: `service/dashboard.py:6050 (route reg 448-450)`
- [degraded] `GET /internal/master/v1/discord/members (broker list_members)` <missing>
  - Broker runs inside the gateway process (dl-bot starts dl-broker at bin/dl-bot/src/main.rs:283). Python broker registers GET /internal/master/v1/discord/members (read-only, loopback, lists non-bot guild members with chunk fallback). dl-broker router (crates/dl-broker/src/lib.rs:118-182) has NO member
  - Py: `service/master_broker.py:1221 (route reg 210-213)`
- [degraded] `POST /internal/master/v1/discord/resolve-user (broker resolve_user)` <missing>
  - Python broker registers POST /internal/master/v1/discord/resolve-user. dl-broker router (lib.rs:118-182) has no resolve-user route. dl-discord/src/adapter.rs:128 comment references '_resolve_user' but that is an internal adapter helper, not the exposed broker route. No in-repo Python caller found; l
  - Py: `service/master_broker.py:2433 (route reg 255-258)`

## discord-listeners

_9 Luecken (von 32 geprueft)._

- **[BLOCKER]** `server_faq.on_message (FAQ thread chat) + whole ServerFAQ cog` <missing>
  - ServerFAQ is a DISTINCT cog from faq_chat (the /faq slash command creates a thread, AI answers each user message in that thread via the on_message listener, logs Q&A). It has setup() at :488 and is NOT in cog_blocklist.json, so it is LIVE in Python. Rust only ported faq_chat.py (crates/dl-community/
  - Py: `cogs/server_faq.py:372 (listener), :286 _handle_thread_message, :142 class ServerFAQ, :488 setup`
- **[BLOCKER]** `coaching_survey.on_voice_state_update (voice-driven coaching session completion + survey + reward role)` <missing>
  - When coach+student leave a shared voice channel, this listener auto-completes the coaching session: removes active role, assigns a 5-day reward role, sends the survey DM, sets coaching_sessions.status='completed'/survey_sent_at, and completes the coaching_request. Rust explicitly defers this: crates
  - Py: `cogs/coaching_survey.py:215 (listener), :83 _process_session_voice_state, :124 send_survey_dm, :128 reward role/status update`
- [degraded] `rules_channel.on_member_update (auto-start onboarding on membership-screening completion: pending true->false)` <missing>
  - When a member finishes Discord membership screening, this creates an onboarding thread and posts step 0 (the documented primary onboarding entry: dm_main.py:334 logs 'Onboarding läuft über den Regelkanal'). Rust gateway guild_member_update (crates/dl-discord/src/gateway.rs:128) ONLY diffs ROLES and 
  - Py: `cogs/rules_channel.py:255 (listener), :260 condition (before.pending and not after.pending), :263 _auto_start_onboarding`
- [degraded] `user_activity_analyzer.on_message — text-session tracking (text_stats / text_conversation_log writer)` <partial>
  - The Python on_message does TWO things: (1) upsert message_activity (ported), and (2) maintain text engagement SESSIONS that accumulate points/co-participants and flush into text_conversation_log + text_stats. The Rust message subscriber (crates/dl-activity/src/analyzer.rs:505 spawn_message_activity)
  - Py: `cogs/user_activity_analyzer.py:1445 (listener), :1474 _open_text_sessions, :653 _flush_expired_text_sessions, :685 INSERT text_conversation_log, :705 INSERT text_stats`
- [degraded] `ai_moderator.on_message (AI chat moderation: classify, auto-delete, propose, ragebait)` <partial>
  - Core pipeline IS ported in crates/dl-moderation/src/lib.rs (subscribe_messages at :402): MiniMax classify, auto-delete >=0.90, propose-case >=0.78, ragebait window counting -> persistent_ragebait. lib.rs:11-18 documents the DELIBERATE gaps that Python still does: context-backfill escalation (12 msgs
  - Py: `cogs/ai_moderator.py:469 (listener), :495 tone-tag threshold, :504 ragebait, :515 ragebaiter free-warning, :485/:518 image attachments`
- [degraded] `welcome_dm/dm_assistant.on_message (free-text AI DM assistant)` <missing>
  - When a user DMs the bot free text, this generates an AI reply (Gemini, OpenAI fallback) with a rate-limit + fallback view. It IS live (dm_main.setup adds BotDMAssistant). Rust gateway publishes DM messages (guild_id None, gateway.rs:79-94), but no Rust subscriber generates an AI reply to DMs. onboar
  - Py: `cogs/welcome_dm/dm_assistant.py:167 (listener); registered via cogs/welcome_dm/dm_main.py:341-344 (add_cog BotDMAssistant)`
- [cosmetic] `tempvoice/router_interface.on_ready (post the router mode-selection panel)` <partial>
  - Python posts/ensures the persistent router panel (Casual/Ranked/Street Brawl + Auto-Join buttons) on startup. Rust registers the panel BUTTON handlers (crates/dl-voice/src/router.rs:304 register, custom_ids router_mode_* and router_autojoin_toggle unchanged, wired at main.rs:135) so the already-post
  - Py: `cogs/tempvoice/router_interface.py:140 (listener), :147 _post_interface_message, :152 _build_guide_embed`
- [cosmetic] `tempvoice/interface.on_tempvoice_lane_created/owner_changed/deleted + on_ready global panel` <partial>
  - Per-lane interface posting is DISABLED in Python (interface.py comment: 'Interface im Voice-Call-Chat deaktiviert – wird nur über den dedizierten Interface-Kanal verwaltet'); the custom on_tempvoice_lane_* events only edit existing records. The real artifact is the GLOBAL interface panel. Rust regis
  - Py: `cogs/tempvoice/interface.py:523/527/531 (custom-event listeners), :136 ensure_interface_message (on_ready of that class)`
- [cosmetic] `tempvoice/core.on_guild_channel_update (lane category-change handling)` <partial>
  - Python reacts to a voice channel's category_id changing (e.g., a lane moved between categories) via on_guild_channel_update. The dispatcher has NO channel-create/update/delete event family, so this exact trigger is not surfaced. The downstream effect (re-evaluating lane layout/sorting) is approximat
  - Py: `cogs/tempvoice/core.py:2469 (listener), :2482 _handle_category_change`

## slash-commands

_28 Luecken (von 31 geprueft)._

- **[BLOCKER]** `/ticket (bug_reporter)` <missing>
  - @app_commands.command(name="ticket") with category Choices opens the support-ticket modal. No on_command("ticket") anywhere in rust/ (only on_command sites are the 11 in dl-bridges/src/steam.rs and 2 in dl-community/src/privacy_ui.rs). dl-bot sync_commands (crates/dl-discord/src/dispatch.rs) builds 
  - Py: `cogs/bug_reporter.py:205`
- **[BLOCKER]** `/turnier (customgames/turnier)` <missing>
  - @app_commands.command(name="turnier") renders the full tournament dashboard embed (period, signups, admin tools) for users. Rust dl-tournament/src/discord_ui.rs:402 register() wires only on_custom_id("turnier_panel_anmelden/abmelden/status") and on_prefix("tn:") — i.e. the panel BUTTONS exist but th
  - Py: `cogs/customgames/turnier.py:1247`
- **[BLOCKER]** `/faq (server_faq)` <missing>
  - @app_commands.command(name="faq") starts an AI FAQ-chat thread for the user. Rust dl-community/src/faq.rs:561 register() wires only on_custom_id("faq_chat:start") and on_prefix("faq_chat:close") — the panel button is ported but the /faq slash command is not. No on_command("faq").
  - Py: `cogs/server_faq.py:390`
- **[BLOCKER]** `/coaching-anfrage (coaching_panel)` <missing>
  - @app_commands.command(name="coaching-anfrage") calls _start_coaching_flow to begin a coaching request. Rust dl-community/src/coaching_requests.rs:945 register() wires only on_custom_id("coaching_panel_start"/"coaching_request_modal") and on_prefix coach_claim_/release_/cancel_ — the panel BUTTON flo
  - Py: `cogs/coaching_panel.py:323`
- **[BLOCKER]** `/meine-tags (tags/interface)` <missing>
  - @app_commands.command(name="meine-tags") (guild_only) opens the user's voice-tag self-management view. No on_command("meine-tags") in rust/; no dl-* crate registers tag slash commands (grep for "meine-tags" in rust/ is empty).
  - Py: `cogs/tags/interface.py:233`
- **[BLOCKER]** `/streamer (welcome_dm/step_streamer)` <missing>
  - @app_commands.command(name="streamer") starts the 2-step streamer-partner onboarding via DM (with partner-onboarding blacklist check + Twitch integration). Rust onboarding.rs:375 register() wires on_custom_id("rp:panel:start") and on_prefix("ob:") — the role-panel onboarding buttons, NOT this /strea
  - Py: `cogs/welcome_dm/step_streamer.py:795`
- [degraded] `/faqclose (server_faq)` <missing>
  - @app_commands.command(name="faqclose") closes the user's FAQ thread. No on_command("faqclose") in rust/. Rust faq.rs only has on_prefix("faq_chat:close") (a button), so closing via slash command is lost.
  - Py: `cogs/server_faq.py:438`
- [degraded] `/faqpanel (faq_chat)` <missing>
  - @app_commands.command(name="faqpanel") (admin, default_permissions=administrator) (re)creates the persistent FAQ panel message. No on_command("faqpanel") in rust/. The Rust faq.rs handles the panel button (on_custom_id faq_chat:start) but provides no way to (re)post the panel.
  - Py: `cogs/faq_chat.py:906`
- [degraded] `/coaching-status (coaching_panel)` <missing>
  - @app_commands.command(name="coaching-status") reads coaching_requests row and reports status to the user. No on_command("coaching-status") in rust/; coaching_requests.rs register() has no slash command.
  - Py: `cogs/coaching_panel.py:327`
- [degraded] `/coaching-session-beenden (coaching_survey)` <missing>
  - @app_commands.command(name="coaching-session-beenden") (Admin) ends a session and grants the reward role. No on_command for it; no Rust coaching_survey module registers commands.
  - Py: `cogs/coaching_survey.py:254`
- [degraded] `/coaching-survey-senden (coaching_survey)` <missing>
  - @app_commands.command(name="coaching-survey-senden") (Admin) sends a survey DM. No on_command for it in rust/.
  - Py: `cogs/coaching_survey.py:232`
- [degraded] `/coaching-analysieren (coaching_request)` <missing>
  - @app_commands.command(name="coaching-analysieren") (Admin) manually triggers analysis of a coaching request by id. No on_command for it; coaching_requests.rs ports only the button/modal flow.
  - Py: `cogs/coaching_request.py:996`
- [degraded] `/mod-tag set (tags/mod_commands)` <missing>
  - GroupCog group_name="mod-tag", subcommand set assigns a mod tag (e.g. ragebaiter) to a user. No on_command("mod-tag set") in rust/; tag system not ported.
  - Py: `cogs/tags/mod_commands.py:111`
- [degraded] `/mod-tag remove (tags/mod_commands)` <missing>
  - GroupCog mod-tag subcommand remove. No on_command("mod-tag remove") in rust/.
  - Py: `cogs/tags/mod_commands.py:158`
- [degraded] `/mod-tag list (tags/mod_commands)` <missing>
  - GroupCog mod-tag subcommand list shows a user's active mod tags. No on_command("mod-tag list") in rust/.
  - Py: `cogs/tags/mod_commands.py:193`
- [degraded] `/website-invite (website_invite_cog)` <missing>
  - @app_commands.command(name="website-invite") (owner/admin) shows the current website invite code and its usage. No on_command for it in rust/ (grep "website-invite" empty).
  - Py: `cogs/website_invite_cog.py:253`
- [degraded] `/website-invite-recreate (website_invite_cog)` <missing>
  - @app_commands.command(name="website-invite-recreate") (owner/admin) recreates invites per website subpage (Choices). No on_command in rust/.
  - Py: `cogs/website_invite_cog.py:316`
- [degraded] `/join-quellen (website_invite_cog)` <missing>
  - @app_commands.command(name="join-quellen") (owner/admin) reports join sources over N days. No on_command in rust/. Note: the underlying join-source TRACKING may be ported (task #3) but the slash REPORT command is not.
  - Py: `cogs/website_invite_cog.py:375`
- [degraded] `/clips_repost (clip_submission)` <missing>
  - @app_commands.command(name="clips_repost") (re)creates/updates the clip-submission interface message. Rust dl-community/src/clips.rs:558 register() wires only on_custom_id(clip_submit_btn_v1 / clip_perm_yes_v1 / clip_submit_modal_v1) — the submit buttons/modal are ported but the admin repost command
  - Py: `cogs/clip_submission.py:723`
- [degraded] `/clips winner_draw (clip_submission)` <missing>
  - app_commands.Group(name="clips") subcommand winner_draw draws a random winner from a time range. No on_command("clips winner_draw") in rust/; clips.rs ports only the submission buttons.
  - Py: `cogs/clip_submission.py:752`
- [degraded] `/changelog post (changelog_publisher)` <missing>
  - app_commands.Group(name="changelog", guild_only) subcommand post (requires manage_guild/admin) posts a changelog entry to Discord. No on_command("changelog post") in rust/. NOTE: the changelog HTTP endpoint :8899 IS ported (dl_changelog::router mounted in bin/dl-bot/src/main.rs:311), so the curl-bas
  - Py: `cogs/changelog_publisher.py:376`
- [degraded] `/steam_rank_sync (steam_bridge)` <missing>
  - @app_commands.command(name="steam_rank_sync") (admin) triggers a rank re-sync. The Rust steam bridge (dl-bridges/src/steam.rs) registers 11 commands but NOT steam_rank_sync/subrank_sync/sync_steam_friends (only betainvite, publish_betainvite_panel, betainvite_stats, account_verknüpfen, steam links/w
  - Py: `cogs/steam_bridge.py:895`
- [degraded] `/subrank_sync (steam_bridge)` <missing>
  - @app_commands.command(name="subrank_sync") (admin). Not among the 11 on_command registrations in dl-bridges/src/steam.rs. No on_command("subrank_sync").
  - Py: `cogs/steam_bridge.py:903`
- [degraded] `/sync_steam_friends (steam_bridge)` <missing>
  - @app_commands.command(name="sync_steam_friends") (admin). Not among the 11 on_command registrations in dl-bridges/src/steam.rs. No on_command("sync_steam_friends").
  - Py: `cogs/steam_bridge.py:911`
- [degraded] `/publish_rules_panel (rules_channel)` <missing>
  - @app_commands.command(name="publish_rules_panel") (admin) posts the rules panel with the 'Hier starten' start button into the rules channel. No on_command("publish_rules_panel") in rust/.
  - Py: `cogs/rules_channel.py:178`
- [degraded] `/retention-optout (user_retention)` <missing>
  - @app_commands.command(name="retention-optout") lets a user opt out of retention DMs. No on_command("retention-optout") in rust/; no retention slash command registered (grep empty).
  - Py: `cogs/user_retention.py:731`
- [degraded] `/retention-optin (user_retention)` <missing>
  - @app_commands.command(name="retention-optin") re-enables retention DMs. No on_command("retention-optin") in rust/.
  - Py: `cogs/user_retention.py:753`
- [degraded] `/nudgesend (steam_link_voice_nudge, hybrid_command)` <missing>
  - @commands.hybrid_command(name="nudgesend") — a hybrid command exposed both as slash AND prefix. Rust dl-voice::nudge::register (bin/dl-bot/src/main.rs:104) wires nudge interaction handlers but there is no on_command("nudgesend"); grep for "nudgesend" in rust/ is empty. The slash side is lost; need t
  - Py: `cogs/steam_link_voice_nudge.py:671`

## prefix-admin-commands

_24 Luecken (von 27 geprueft)._

- **[BLOCKER]** `Generic prefix-command (!cmd) dispatch` <partial>
  - Rust dispatch.rs only routes Interaction::Command (slash), ::Component, ::Modal. There is NO MessageCreate-based prefix parser. The ONLY message-content listeners in Rust are two hardcoded ones: spawn_admin_command_listener for !steam_* (dl-bridges/src/steam.rs:655) and spawn_command_listener for !t
  - Py: `main_bot.py (discord.py Bot with command_prefix); every @commands.command across cogs/`
- **[BLOCKER]** `!balance group (auto, voice, manual, start, status, matches, cleanup, end, turnierpanel, turnierstatus, turnierliste, austragen)` <missing>
  - User-facing team-balancing + match-creation prefix command group: !balance auto/voice/manual/start build two voice channels and move players into balanced teams; status/matches/end manage active matches. dl-tournament ports ONLY the signup panel buttons (turnier_panel_*) and the store (discord_ui.rs
  - Py: `cogs/deadlock_team_balancer.py:637-927`
- **[BLOCKER]** `Retention 'Wir-vermissen-dich' DM loop + retention_feedback button` <missing>
  - Rust retention.rs doc explicitly: 'Die Wir-vermissen-dich-DM (daily_retention_check) ist user-facing (Embed + Feedback-Button) und folgt im gebündelten UI-Pass.' No miss_you/vermissen DM-send loop and no retention_feedback button handler in Rust. After cutover, inactive users stop receiving the re-e
  - Py: `cogs/user_retention.py:110 MissYouView / :162 custom_id=retention_feedback / daily_retention_check loop`
- **[BLOCKER]** `!fhub (Feedback Hub panel setup)` <missing>
  - manage_guild command posts the anonymous Feedback-Hub panel (embed + persistent button custom_id=feedback_hub:open_modal). Neither the !fhub command nor the feedback_hub:open_modal/modal handler exist in Rust (grep feedback_hub/fhub in rust/ empty — only voice/feedback.rs which is unrelated VoiceFee
  - Py: `cogs/feedback_hub.py:184`
- **[BLOCKER]** `feedback_hub:open_modal button + modal` <missing>
  - No router.on_custom_id('feedback_hub:open_modal') anywhere in rust/. Any existing posted Feedback-Hub message's button silently no-ops after cutover; users cannot submit anonymous feedback.
  - Py: `cogs/feedback_hub.py (custom_id=feedback_hub:open_modal persistent view)`
- **[BLOCKER]** `bugreporter:create button (+ !ticket prefix fallback)` <missing>
  - grep bugreporter/bug_report/issue_report in rust/ only hits privacy.rs DB cleanup ('issue_reports' table) — no button/modal/command handler. The bugreporter:create persistent button (entry to bug/ticket modal) and the /ticket slash flow are unported. !ticket prefix is just a one-liner pointing to /t
  - Py: `cogs/bug_reporter.py custom_id=bugreporter:create; :246 !ticket fallback`
- [degraded] `!vleaderboard / !vlb / !voicetop (voice leaderboard)` <missing>
  - User-facing prefix leaderboard command. No Rust message-listener. The dl-stats web route /api/public/leaderboard/voice exists (dl-stats/src/public.rs:659) but that is an HTTP endpoint, not the in-chat !vleaderboard command — clicking/typing it in Discord after cutover does nothing.
  - Py: `cogs/voice_activity_tracker.py:1491`
- [degraded] `!tleaderboard / !tlb / !texttop (text leaderboard)` <missing>
  - User-facing prefix text-leaderboard. No Rust message-listener. dl-stats has /api/public/leaderboard/text (public.rs:710) as web only, not the chat command.
  - Py: `cogs/user_activity_analyzer.py:2112`
- [degraded] `!myactivity / !useranalysis (ua/analyze) / !messagestats (msgstats) / !vstats` <missing>
  - User-facing self-stats prefix commands (member can query own activity/messages/voice stats). No Rust prefix handler anywhere (grep for myactivity/useranalysis/messagestats/vstats in rust/ returns nothing). After cutover they silently no-op.
  - Py: `cogs/user_activity_analyzer.py:1885,2046,2151; cogs/voice_activity_tracker.py:1427`
- [degraded] `!smartping / !checkping / !memberevents (mevents) / !serverstats / !rawmember` <missing>
  - Admin/mod diagnostic & ping-helper prefix commands (smartping/serverstats/rawmember gated by manage_messages/manage_guild). No Rust equivalent. Lost on cutover but admin-only tooling.
  - Py: `cogs/user_activity_analyzer.py:1982,2022,2064,2219,2311`
- [degraded] `!vtest / !vf1 / !vf4 / !voice_status / !voice_config` <missing>
  - Admin voice-system test/diagnostic/config prefix commands (vf1/vf4/voice_status/voice_config gated administrator). No Rust message-listener. Admin tooling, lost on cutover.
  - Py: `cogs/voice_activity_tracker.py:1536,1593,1619,1645,1679`
- [degraded] `!dlvs group (trace, snapshot)` <missing>
  - manage_guild-gated diagnostic group toggling voice-status trace logging + snapshot. The voice-status WORKER is ported (dl-voice/src/status.rs) but the !dlvs admin diagnostic commands are not. Admin-only.
  - Py: `cogs/deadlock_voice_status.py:984,997,1031`
- [degraded] `!rrang group (status, anker, toggle, vcstatus, debug, info, aktualisieren, rollen, kanäle)` <missing>
  - RankVoiceManager runtime IS ported (dl-voice/src/rank.rs, spawned main.rs:354-364) so the rank-voice feature itself works. But the !rrang admin command group (status/anker/toggle/debug/aktualisieren config + diagnostics) has NO Rust prefix handler (grep rrang/rank_command in rank.rs empty). Admins l
  - Py: `cogs/rank_voice_manager.py:1253-1566`
- [degraded] `!retention_status / !retention_preview / !retention_test / !retention_test_dm / !retention_feedback` <missing>
  - administrator-gated retention admin commands (status/preview/test DM sending). No Rust prefix handler. Note: Rust retention crate is data-layer ONLY (retention.rs doc-comment explicitly states the user-facing DM 'folgt im gebündelten UI-Pass').
  - Py: `cogs/user_retention.py:777,855,883,915,985`
- [degraded] `tags:save / tags:reset / tags:{tag_key} (Meine-Tags self-service panel)` <missing>
  - The user-facing tag-selection panel buttons (tags:{tag_key} toggles, tags:save, tags:reset) have NO Rust router registration. The grep hit for 'tags:' in dl-voice/tempvoice/engine.rs is an internal tag reference, not the button handler. The TagService backend IS ported (dl-community/tags) and TempVo
  - Py: `cogs/tags/interface.py:31,203,214 (MeineTagsView persistent buttons)`
- [degraded] `ai_onboarding: aiob:start / aiob:rules_confirm buttons + !aiob` <missing>
  - ai_onboarding has NO Rust port (grep ai_onboarding in rust/ only hits privacy.rs kv_store cleanup). The persistent aiob:start button is restored at cog_load (ai_onboarding.py:438/503) for existing messages; after cutover those buttons no-op, and the QuickActions aiob:rules_confirm button is also unh
  - Py: `cogs/ai_onboarding.py:410 (aiob:start) / :260 (aiob:rules_confirm); cogs/ai_connector.py:768 (!aiob)`
- [degraded] `wdm:streamer:* buttons (streamer onboarding DM raid-bot opt-in)` <missing>
  - step_streamer.py registers StreamerIntroView + StreamerRequirementsView persistently (bot.add_view :785-786). The custom_ids wdm:streamer:intro_yes/intro_no/req_raid_bot/req_cancel and the dynamic wdm:streamer:raid_confirmed:{user_id} are NOT in onboardglue.rs register list (only wdm:q0/q1/q2/q4/qS 
  - Py: `cogs/welcome_dm/step_streamer.py:352,437,575,643,751 (intro_yes/intro_no/req_raid_bot/raid_confirmed:{id}/req_cancel)`
- [degraded] `welcome_dm channel-flow step buttons (wdm:q0/q1/q2/qS, steam:next)` <partial>
  - onboardglue.rs registers these custom_ids but the handler returns a generic 'Dieser Onboarding-Schritt wird gerade umgebaut' stub for everything except wdm:q4:confirm/steam:openid/dma:fallback:*. So the step-navigation of the original welcome-DM flow is intentionally degraded to a placeholder; the a
  - Py: `cogs/welcome_dm/step_*.py (intro_next, masterbot, servertour, qS:next/status)`
- [degraded] `!tvpanel / !tempvoicepanel / !tvinterface (post TempVoice interface)` <missing>
  - manage_guild admin command that creates/updates+persists the TempVoice interface message. The TempVoice panel BUTTON handlers are fully ported (dl-voice/tempvoice/interface.rs register, main.rs:116) so existing panels keep working, but there is NO Rust path to (re)post/refresh the panel — grep ensur
  - Py: `cogs/tempvoice/interface.py:209`
- [degraded] `!security_diag (spam-guard thresholds)` <missing>
  - administrator command printing active spam-guard thresholds. SecurityGuard runtime + sg:* mod-review buttons ARE ported (main.rs:150-163, modglue GuardReviewHandler), but the !security_diag chat diagnostic is not. Admin-only diagnostic loss.
  - Py: `cogs/security_guard.py:1761`
- [degraded] `!verifyrole_run / !verifyrole_diag` <unsure>
  - administrator manual-run + diagnostic for the Steam-verified-role assignment. I did not locate a Rust port of the steam_verified_role assignment loop in this slice's scope; grep verifyrole in rust/ empty. CHECK: whether the verified-role assignment itself is handled by steam-bot/steam-core (separate
  - Py: `cogs/steam_verified_role.py:808,817`
- [degraded] `!set_log_channel (steam.presence log bridge)` <missing>
  - administrator command that routes the steam.presence Python logger to a Discord channel. No Rust equivalent (grep set_log_channel/log_bridge in rust/ empty). log_bridge cog listed among completely-missing cogs in project memory. Operational logging-to-Discord helper lost.
  - Py: `cogs/helper/log_bridge.py:138`
- [degraded] `!leavesurvey_status / !leavesurvey_test / !leavesurvey_recent` <missing>
  - administrator diagnostics for the leave-survey. The leave-survey FEATURE (trigger + leave_survey:reason:/modal: buttons) IS ported (dl-community/leave_survey.rs, main.rs:248/435), but these admin status/test/recent chat commands are not. Admin tooling loss only.
  - Py: `cogs/leave_survey.py:625,682,702`
- [degraded] `!lfgtest / !lfgroute` <missing>
  - administrator LFG test/route diagnostics. The LFG responder runtime IS ported (dl-activity/lfg.rs, main.rs:441-447) but the !lfgtest/!lfgroute admin commands are not. Admin diagnostic loss only.
  - Py: `cogs/lfg.py:2342,2366`

## background-loops

_11 Luecken (von 24 geprueft)._

- **[BLOCKER]** `steam_verified_role periodic verified-role assigner (_run_once hourly + _fast_lane_assign 30s)` <missing>
  - Python runs an hourly full sweep + a 30s fast-lane that read steam_links (verified=1 AND is_steam_friend=1, see :79 and :108) and ASSIGN settings.verified_role_id to the matching members (assign_verified_role at :369/:497/:737). On the Rust side the only Verified-role logic is dl-community/src/onboa
  - Py: `cogs/steam_verified_role.py:705 (fast-lane 30s), :750/:787 (1h loop_body), :523 (_run_once)`
- **[BLOCKER]** `build_publisher (publisher_loop 10min + monitor_loop 2min)` <missing>
  - Python worker drains the hero_build_clones queue, creates BUILD_PUBLISH tasks for the Steam bridge (after checking standalone_bot_state steam GC-ready), and the monitor loop updates clone status from task results. grep 'BUILD_PUBLISH'/'hero_build_clones'/'build_publisher'/'process_queue' across all 
  - Py: `cogs/build_publisher.py:43 (publisher 600s), :44/:72 (monitor 120s), :59 (_publisher_loop), :85 (process_queue)`
- **[BLOCKER]** `coaching_survey voice-end detection loop (grants reward role + sends feedback DM + completes session)` <missing>
  - Python loop scans coaching_sessions status='active' AND survey_sent_at IS NULL every 60s, detects when coach+user leave the shared voice channel, then removes the active role, grants the 5-day reward role (sets reward_role_expires_at), sends the survey DM, and marks session+request 'completed'. In R
  - Py: `cogs/coaching_survey.py:29 (task), :62 (_run_survey_checks 60s), :71 (_scan_active_sessions), :83 (_process_session_voice_state)`
- **[BLOCKER]** `coaching_survey reservation/analysis vs separate survey scan — note` <missing>
  - Duplicate-guard note: this is the same gap as the coaching_survey voice-end loop above; listed once as blocker. Not double counting impact.
  - Py: `cogs/coaching_survey.py:29`
- [degraded] `deadlock_missing_build_alert_loop (Master Dashboard, gateway-bound Discord alert)` <missing>
  - Loop runs every DEADLOCK_MISSING_BUILD_ALERT_INTERVAL_SECONDS, claims rows from deadlock_hero_builds where sync_status='missing' and posts a Discord embed to DEADLOCK_MISSING_BUILD_ALERT_CHANNEL_ID via self.bot.get_channel (so it needs the gateway). grep 'missing_build'/'build_alert' in rust returns
  - Py: `service/dashboard.py:560 (spawn), :5266 (_deadlock_missing_build_alert_loop), :5277 (_dispatch_deadlock_missing_build_alerts)`
- [degraded] `user_retention daily_retention_check (we-miss-you DM to inactive regulars)` <missing>
  - Python sends 'miss you' DMs once/day to inactive regular users. Rust retention.rs spawn (line 179-221) only runs the 30-min sync_activity_data loop + voice-join subscriber. The crate docstring (retention.rs:9-10) explicitly says the daily_retention_check DM 'folgt im gebündelten UI-Pass' (deferred).
  - Py: `cogs/user_retention.py:361 (daily_retention_check, hourly gate on check_hour), :379 (_find_inactive_regular_users), :384 (_send_miss_you_message)`
- [degraded] `customgames/turnier auto_balance loop (5min team auto-balance)` <missing>
  - Python loop runs every 300s and auto-balances customgame signup teams (fills non-full teams from the signup pool) per active period. dl-tournament in Rust has discord_ui + store but NO tokio::spawn / auto_balance (grep 'auto_balance'/'balance_loop'/'tokio::spawn' in rust/crates/dl-tournament returns
  - Py: `cogs/customgames/turnier.py:991 (_balance_task), :1004 (_auto_balance_loop 300s), :1014 (_run_auto_balance)`
- [degraded] `user_activity_analyzer flush_text_sessions (60s text-XP flush)` <unsure>
  - Python flushes expired in-memory text-message XP sessions to DB every 60s (paired with on_message accumulation at :1471-1510). The Rust analyzer spawn (analyzer.rs:557) only spawns the 6h pattern analysis + 10min co-player tracking. grep 'text_session'/'flush'/'points_accum' in analyzer.rs returns n
  - Py: `cogs/user_activity_analyzer.py:1517 (flush_text_sessions 60s), :1520 (_flush_expired_text_sessions)`
- [cosmetic] `user_activity_analyzer cleanup_old_pings (24h ping_count reset)` <missing>
  - Python daily loop resets ping_count_30d=0 in user_activity_patterns for users not pinged in >30 days. grep 'ping_count_30d = 0'/'cleanup_old_ping' in rust returns nothing; the analyzer spawn only starts analyze+co-player loops. At cutover the 30-day ping counter never resets, so any throttling that 
  - Py: `cogs/user_activity_analyzer.py:540 (cleanup_old_pings, hours=24), :550 (resets ping_count_30d=0 for >30d-stale users)`
- [cosmetic] `ai_moderator cleanup_ragebait_hits 10min` <unsure>
  - Python loop deletes old ragebait hits every 10min. dl-moderation spawn is wired (main.rs:420) and the scan/proposal path is ported, but I did not confirm a dedicated ragebait-hit-cleanup timer inside dl-moderation. The scan itself works either way; this is only stale-row cleanup of the ragebait esca
  - Py: `cogs/ai_moderator.py:459 (cleanup_ragebait_hits, minutes=10)`
- [cosmetic] `helper/log_bridge _worker (Discord log channel pump)` <unsure>
  - Python logging handler pumps batched log lines to Discord channel 1374364800817303632. This is an internal ops log mirror, not a user-facing feature. I did not find a Rust equivalent (no grep target run for log_bridge in rust). If absent, internal logs simply stop mirroring to Discord — no user impa
  - Py: `cogs/helper/log_bridge.py:29 (task), :43 (_worker)`

## schema-and-integrations

_5 Luecken (von 10 geprueft)._

- **[BLOCKER]** `Master broker route GET /internal/master/v1/discord/members` <missing>  ✅ ERLEDIGT 77a400d
  - Python broker serves GET /discord/members (all non-bot guild members). Rust dl-broker (rust/crates/dl-broker/src/lib.rs:116-205) does NOT register this route — it has /discord/voice-channel/members (POST, different) and /discord/role-members but no plain /discord/members. The LIVE Twitch-bot calls i
  - Py: `service/master_broker.py:210 (route) + 1221 (_handle_list_members)`
- **[BLOCKER]** `Master broker route POST /internal/master/v1/discord/resolve-user` <missing>  ✅ ERLEDIGT 475da89
  - Python broker serves POST /discord/resolve-user (resolve a Discord user by id). Rust dl-broker does NOT register it — it has the differently-named /discord/resolve-names (bulk, for the Rust dashboard) but not resolve-user. The LIVE Twitch-bot calls resolve-user: rust/crates/tb-transport-discord/src/
  - Py: `service/master_broker.py:255 (route) + 2433 (_handle_resolve_user)`
- [degraded] `Master schema bootstrap (init_schema) — single idempotent owner` <partial>
  - Python init_schema creates ALL ~60+ tables idempotently in one place on startup (executescript covering schema_version, kv_store, voice_stats, steam_links, live_player_state, message_activity, member_events, etc.). Rust has NO equivalent master bootstrap. dl-bot main.rs only calls a handful of per-m
  - Py: `service/db.py:496 (init_schema), called at db.py:363`
- [degraded] `Core tables created ONLY in #[cfg(test)] — voice_stats / voice_session_log / message_activity / user_co_players / user_activity_patterns / member_events / steam_links / live_player_state` <partial>
  - Every Rust CREATE TABLE for these core tables is inside a `const DDLS`/test setup behind #[cfg(test)] (verified by reading tracker.rs:700 and analyzer.rs:575 #[cfg(test)] guard). The runtime writers (analyzer spawn_message_activity, spawn_member_events, voice tracker) INSERT/UPDATE assuming the tabl
  - Py: `service/db.py:620 (voice_stats), 628 (voice_session_log), 945 (user_co_players), 931 (user_activity_patterns), 957 (member_events), 671 (steam_links), 709 (live_player_state); message_activity at db.py:990`
- [degraded] `text_stats + text_conversation_log writer (text gamification + conversation memory)` <missing>
  - Python user_activity_analyzer.on_message writes text_stats (per-user total_messages/total_points, fed to the public text leaderboard) and text_conversation_log (per-conversation session log). NO Rust code writes either: grep for INSERT/UPDATE text_stats / text_conversation_log across rust/crates+bin
  - Py: `cogs/user_activity_analyzer.py:705 (INSERT text_stats), 685 (INSERT text_conversation_log), on_message listener at 1445; cog auto-loaded (bot_core/cog_loader.py discover, NOT in cog_blocklist.json)`
## Korrekturen (Post-Audit-Verifikation)

Nach dem Audit per Quellen-Check widerlegte Falsch-Positive — vor jedem Port
gilt: erst pruefen, ob der Cog ueberhaupt geladen ist.

- **`steam_verified_role` (background-loops, war [BLOCKER]) → KEIN Blocker,
  intentional-drop.** Der Cog steht in `bot_core/cog_loader.py:222`
  `default_excludes` und wird seit dem Steam-Cutover NICHT geladen; die
  Verified-Rolle gehoert allein dem live Rust-steam-bot (`steam-flows/
  friend_sync.rs`). Ein Port haette dl-bot dieselbe Rolle/DB schreiben lassen
  wie der Steam-Bot → Rollen-Flapping (genau der Bug, den der Exclude-Kommentar
  dokumentiert). Der widersprechende `on_ready`-Befund (intentionally-dropped)
  war korrekt; der `periodic assigner`-Blocker-Befund hat den Lade-Ausschluss
  uebersehen. NICHT portieren.

- **`build_publisher` (background-loops, war [BLOCKER]) → gehoert in den
  Rust-steam-bot, NICHT in dl-bot.** Der Cog ist ein reiner DB-Queue-Worker
  ohne jeden Discord-Bezug (keine Commands/Listener/DMs): sein `_publisher_loop`
  (10 min) liest aus `hero_build_clones` (Status `pending`) und legt
  `BUILD_PUBLISH`-Tasks in `steam_tasks` an; sein `_monitor_loop` (2 min) liest
  abgeschlossene Tasks zurueck und aktualisiert den Clone-Status. Konsumiert wird
  die Queue vom LIVE Rust-steam-bot. Verifikation im Steam-Bot-Repo
  (`Deadlock-Steam-Bot/rust/`): `steam-persistence/src/builds.rs` hat bereits
  ALLE Primitive — `insert_publish_task` (415), `has_pending_publish_task` (315,
  Duplikat-Schutz), `cancel_pending_publish_tasks`, `get_pending_publish_task_id`,
  Clone-CRUD — und `steam-core/.../handlers/builds/mod.rs` betreibt mit
  `MAINTAIN_BUILD_CATALOG`/`BUILD_CATALOG_CYCLE` schon einen Producer-Pfad, der
  konfigurierte Builds mit `hero_build_clones` abgleicht und `BUILD_PUBLISH`-Tasks
  erzeugt. **Folge:** Ein Port nach dl-bot wuerde einen zweiten, parallelen
  Producer auf dieselbe `steam_tasks`-Queue setzen → Doppel-Publishing (dieselbe
  Klasse Fehler wie steam_verified_role: zwei Owner auf einer Ressource). Der
  korrekte Zielort ist der steam-bot, der `steam_tasks` und den GC ohnehin
  besitzt; ob `MAINTAIN_BUILD_CATALOG` den Dashboard-Clone-Pfad
  (`hero_build_clones`, befuellt aus `service/dashboard.py`) bereits vollstaendig
  abdeckt oder dort noch ein dedizierter Queue-Drain fehlt, ist eine
  Cross-Repo-Entscheidung fuer den Steam-Bot — NICHT nach dl-bot portieren.

### Re-Triage der verbleibenden Bot-Cutover-Blocker (Stand nach den UI-/Cog-Ports)

Erledigt in dieser Port-Serie (alle in dl-bot, noch nicht cutover): `/faq`,
`/coaching-anfrage`, `/coaching-status`, `/turnier`, `/meine-tags`,
UPDATE_MESSAGE-Infra, `dl-broker` resolve-user + members, feedback_hub
(Button+Modal+`!fhub`-Panel), coaching_survey (Voice-Ende → Reward-Rolle +
Survey-DM), retention Miss-You-DM (Loop + Embed-DM + Feedback-Modal).

Echte verbleibende Blocker, jeweils GROSSE Multi-File-Subsysteme (kein
Slash-Wrapper) — bewusst je eigener Pass:

- **Streamer-Partner-Onboarding** (`/streamer`): die Slash-Spitze in
  `welcome_dm/step_streamer.py:795` ist trivial, dahinter haengt aber das ganze
  Partner-Subsystem (`StreamerIntroView`, 2-Schritt-DM-Flow, Partner-Blacklist,
  Twitch-Integration `TwitchPartnerIntegrationUnavailable`, ~850 Z.). Dazu der
  separate `twitch/streamer_link_matcher.py` (6-h-Loop + `twitch_link_scan`/
  `_rescan_login`-Prefix-Commands + Link-Views/Modal).
- **Ticket-/Bug-Reporter** (`/ticket` + `bugreporter:create`): **WIRD NICHT
  MIGRIERT — fliegt raus** (Projekt-Entscheidung). `bug_reporter.py` (~1272 Z.)
  wird beim Cutover ersatzlos fallengelassen, nicht nach Rust portiert. Der in
  einer fruehen Scheibe bereits begonnene Daten-Layer (`dl-community/bug_reports.rs`)
  wurde wieder entfernt. KEIN Blocker mehr.

### Stand nach weiterer Verifikation (laufend gepflegt)

- **Streamer-Partner-Onboarding → ERLEDIGT, vereinfacht.** `/streamer` (alter
  `StreamerOnboarding`-Cog) ist toter Code (Loader ueberspringt `.py` in bereits
  geladenen Paketen → nie geladen); `streamer_link_matcher` war schon portiert
  (`dl-bridges/matcher.rs`). Statt des 2-Schritt-Twitch-Flows: `/streamer`
  verweist auf die Website + merkt sich die Discord-ID 1 h, ein Watcher mappt
  einen neu auftauchenden Streamer (`dl-bridges/streamer_intent.rs`, Commits
  7cfb7ad + bfc58b8). Siehe Memory `project-streamer-link-simplified`.
- **text_stats / text_conversation_log (war [degraded]) → ERLEDIGT.**
  `dl-activity/text_stats.rs` (Commits 481dd04 + 3a7478c): Konversations-
  Sessions, Punkte, Co-Teilnehmer, Idle-Flush; `MessageEvent` bekam `is_reply`.
- **bug_reporter → DROP** (siehe oben), Memory `project-bug-reporter-dropped`.

- **`!balance` / Custom-Games-Team-Balancer (`deadlock_team_balancer.py`, live)
  → Backend KOMPLETT portiert, nur die Admin-Befehlsschicht offen.** Verifiziert:
  der Balancing-Algorithmus (`_balance_score`/`_best_split`) ist `balancer.rs`
  (Tests mit CPython-Referenzwerten), der Store (`customgames_tournament_teams/
  signups`, Perioden, Auth-Tokens) ist `store.rs`, der Turnier-USER-Flow
  (`turnier.py`: Panel `turnier_panel_*`, Team-Signup, `/turnier`) ist
  `discord_ui.rs`. OFFEN ist nur die `!balance`-Admin-Prefix-Gruppe:
  - `auto`/`voice` — Voice-Member → Raenge → `best_split` → Vorschau-Embed
    (read-only; **die natuerliche erste Scheibe**, braucht nur einen Port fuer
    Voice-Member + Rang-aus-Rollen, kein Channel-Move).
  - `start`/`manual` — erstellt 2 Match-Voice-Channels + moved Spieler.
  - `status`/`matches`/`end`/`cleanup` — Match-Lifecycle-Verwaltung.
  - `turnierpanel`/`turnierstatus`/`turnierliste`/`austragen` — groesstenteils
    Admin-Aliase auf den schon portierten Turnier-Flow (Ueberschneidung pruefen).
  Naechster Pass: `!balance auto` als read-only Erst-Scheibe.

### Konsolidierte Restliste (nach allen Verifikationen)

1. `!balance`-Admin-Befehlsschicht (Backend fertig; Algorithmus/Store/Turnier-UI da).
2. `dm_assistant` Free-Text-AI-DM-Assistent — in Rust nicht vorhanden.
3. Master-Schema-Bootstrap + Core-Tabellen nur unter `#[cfg(test)]` (Infra).
4. Kleinkram-Slash: `/faqclose`, `/faqpanel`, `/coaching-analysieren`,
   Owner-only `/coaching-session-beenden` + `/coaching-survey-senden`.
5. By-design deferred (KEIN Rust-Port): Web-Control-Routen (status/restart/
   cogs-reload/logs/standalone → Frontend-Panel-Hiding), Steam-Hero-Sync.
6. Bewusste Drops: bug_reporter, steam_verified_role (anderer Owner),
   build_publisher (gehoert in den steam-bot).
