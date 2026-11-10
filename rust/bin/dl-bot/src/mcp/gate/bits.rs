const NAMES: &[(u32, &str)] = &[
    (0, "create_instant_invite"),
    (1, "kick_members"),
    (2, "ban_members"),
    (3, "administrator"),
    (4, "manage_channels"),
    (5, "manage_guild"),
    (6, "add_reactions"),
    (7, "view_audit_log"),
    (8, "priority_speaker"),
    (9, "stream"),
    (10, "view_channel"),
    (11, "send_messages"),
    (12, "send_tts_messages"),
    (13, "manage_messages"),
    (14, "embed_links"),
    (15, "attach_files"),
    (16, "read_message_history"),
    (17, "mention_everyone"),
    (18, "use_external_emojis"),
    (19, "view_guild_insights"),
    (20, "connect"),
    (21, "speak"),
    (22, "mute_members"),
    (23, "deafen_members"),
    (24, "move_members"),
    (25, "use_vad"),
    (26, "change_nickname"),
    (27, "manage_nicknames"),
    (28, "manage_roles"),
    (29, "manage_webhooks"),
    (30, "manage_guild_expressions"),
    (31, "use_application_commands"),
    (32, "request_to_speak"),
    (33, "manage_events"),
    (34, "manage_threads"),
    (35, "create_public_threads"),
    (36, "create_private_threads"),
    (37, "use_external_stickers"),
    (38, "send_messages_in_threads"),
    (39, "use_embedded_activities"),
    (40, "moderate_members"),
    (41, "view_creator_monetization_analytics"),
    (42, "use_soundboard"),
    (43, "create_guild_expressions"),
    (44, "create_events"),
    (45, "use_external_sounds"),
    (46, "send_voice_messages"),
    (49, "send_polls"),
    (50, "use_external_apps"),
    (51, "pin_messages"),
    (52, "bypass_slowmode"),
];

pub fn bit(name: &str) -> Option<u64> {
    NAMES
        .iter()
        .find(|(_, known)| *known == name)
        .map(|(bit, _)| 1u64 << bit)
}

pub fn all_known() -> u64 {
    NAMES.iter().fold(0, |mask, (bit, _)| mask | (1u64 << bit))
}

pub fn names(mask: u64) -> Vec<String> {
    (0..64)
        .filter(|bit| mask & (1u64 << bit) != 0)
        .map(|bit| {
            NAMES
                .iter()
                .find(|(known, _)| *known == bit)
                .map_or_else(|| format!("bit_{bit}"), |(_, name)| (*name).to_owned())
        })
        .collect()
}
