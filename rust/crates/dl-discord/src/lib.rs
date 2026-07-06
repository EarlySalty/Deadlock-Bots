//! Discord-Glue für dl-bot.
//!
//! - [`adapter`]: serenity-basierte Implementierung der Ports von dl-broker
//!   und dl-changelog. REST-Aktionen funktionieren OHNE Gateway (nur Token);
//!   Cache-abhängige Abfragen (Voice-Member, Rollen-Mitglieder) brauchen die
//!   Gateway-Verbindung und melden das sonst sauber als Fehler.
//! - [`dispatcher`]: der EINE Event-Verteiler — Domänen subscriben auf
//!   normalisierte Events statt eigene Listener zu registrieren (ersetzt die
//!   5×voice/6×message-Listener-Wildwuchs des Python-Originals).
//! - [`gateway`]: serenity-Client-Aufbau. Der Gateway-Start ist user-gated
//!   (DL_BOT_GATEWAY=1) — bis zum Cutover hält der Python-Bot die Session.

pub mod adapter;
mod core_user_sync;
pub mod dispatch;
pub mod dispatcher;
pub mod gateway;
pub mod interactions;
pub mod invite_tracker;

pub use adapter::DiscordAdapter;
pub use dispatcher::{
    ChannelEvent, Dispatcher, GatewayEvent, InteractionEvent, MemberEvent, MessageAttachment,
    MessageEvent, PresenceEvent, RoleEvent, VoiceEvent,
};
pub use interactions::{
    BridgeAttachment, BridgeInteraction, BridgeReply, ChannelSender, CommandSpec,
    InteractionHandler, InteractionRouter, ModalField, ModalSpec, ResponseMessageHook,
};
pub use invite_tracker::InviteTracker;
