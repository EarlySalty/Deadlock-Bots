-- Der beta-invite-Funnel (Panel, Ticket-Kanäle, Community-Delay-Queue, Self-Heal)
-- und der nie benutzte Ko-fi-Supporter-Pfad sind ersatzlos entfallen.
-- steam.beta_invite_audit bleibt: 129 Zeilen Invite-Historie.
DROP TABLE IF EXISTS steam.beta_invite_intent;
DROP TABLE IF EXISTS steam.beta_invite_tickets;
DROP TABLE IF EXISTS steam.beta_invite_panel_clicks;
DROP TABLE IF EXISTS steam.beta_invite_friendship_auto_poll;
DROP TABLE IF EXISTS steam.beta_invite_auto_failure_alerts;
DROP TABLE IF EXISTS steam.beta_invite_pending_payments;
DROP TABLE IF EXISTS steam.beta_invite_supporter_role_grants;
DROP TABLE IF EXISTS steam.steam_quick_invites;
DROP TABLE IF EXISTS steam.steam_beta_invites;
