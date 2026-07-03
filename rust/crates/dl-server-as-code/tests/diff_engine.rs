use dl_server_as_code::{
    diff_models, format, CategorySpec, ChannelKind, ChannelSpec, DiffAction, DocumentedException,
    DynamicNamespace, GuildModel, NamespaceMatch, ObjectKind, OverwriteKey,
    PermissionOverwriteSpec, RoleSpec, TargetKind,
};

const GUILD_ID: u64 = 1289721245281292288;
const CHAT_CATEGORY: u64 = 200;
const GENERAL: u64 = 201;
const TEMPVOICE_PARENT: u64 = 300;
const TEMPVOICE_LANE: u64 = 301;
const COACHING_SCRIM_PARENT: u64 = 1_459_526_231_686_119_600;
const COACHING_TEAM_CHANNEL: u64 = 1_459_526_231_686_119_601;
const FAQ_CHANNEL: u64 = 1_459_526_231_686_119_602;
const ROLE_MEMBER: u64 = 400;
const USER_BANNED: u64 = 500;

fn category(id: u64, name: &str, position: i32) -> CategorySpec {
    CategorySpec {
        guild_id: GUILD_ID,
        category_id: id,
        name: name.to_string(),
        position,
    }
}

fn channel(id: u64, name: &str, parent: Option<u64>) -> ChannelSpec {
    ChannelSpec {
        guild_id: GUILD_ID,
        channel_id: id,
        name: name.to_string(),
        kind: ChannelKind::Text,
        topic: None,
        position: 1,
        parent_category_id: parent,
        nsfw: false,
        bitrate: None,
        user_limit: None,
        rate_limit_per_user: None,
        default_auto_archive_duration: None,
        status: None,
    }
}

fn role(id: u64, name: &str, bits: u64) -> RoleSpec {
    RoleSpec {
        guild_id: GUILD_ID,
        role_id: id,
        name: name.to_string(),
        color: 0,
        hoist: false,
        mentionable: false,
        managed: false,
        permissions_bitmask: bits,
        position: 1,
    }
}

fn overwrite(
    channel_id: u64,
    target_kind: TargetKind,
    target_id: u64,
    allow: u64,
    deny: u64,
) -> PermissionOverwriteSpec {
    PermissionOverwriteSpec {
        guild_id: GUILD_ID,
        key: OverwriteKey {
            channel_id,
            target_kind,
            target_id,
        },
        allow_bits: allow,
        deny_bits: deny,
    }
}

#[test]
fn diff_erkennt_create_update_delete_ueber_strukturobjekte() -> anyhow::Result<()> {
    let mut desired = GuildModel::new(GUILD_ID);
    desired
        .categories
        .insert(CHAT_CATEGORY, category(CHAT_CATEGORY, "Chat", 1));
    desired
        .channels
        .insert(GENERAL, channel(GENERAL, "allgemein", Some(CHAT_CATEGORY)));
    desired
        .roles
        .insert(ROLE_MEMBER, role(ROLE_MEMBER, "Member", 7));

    let mut actual = GuildModel::new(GUILD_ID);
    actual
        .categories
        .insert(CHAT_CATEGORY, category(CHAT_CATEGORY, "Chat alt", 1));
    actual.channels.insert(999, channel(999, "zombie", None));
    actual
        .roles
        .insert(ROLE_MEMBER, role(ROLE_MEMBER, "Member", 3));

    let diff = diff_models(&desired, &actual, &[], &[])?;

    assert_eq!(diff.filtered.len(), 0);
    assert_eq!(diff.changes.len(), 4);
    assert!(diff.changes.iter().any(|change| {
        change.object.kind == ObjectKind::Category
            && change.object.object_id == CHAT_CATEGORY
            && change.action == DiffAction::Update
            && change.fields.iter().any(|field| field.field == "name")
    }));
    assert!(diff.changes.iter().any(|change| {
        change.object.kind == ObjectKind::Channel
            && change.object.object_id == GENERAL
            && change.action == DiffAction::Create
    }));
    assert!(diff.changes.iter().any(|change| {
        change.object.kind == ObjectKind::Channel
            && change.object.object_id == 999
            && change.action == DiffAction::Delete
    }));
    assert!(diff.changes.iter().any(|change| {
        change.object.kind == ObjectKind::Role
            && change.object.object_id == ROLE_MEMBER
            && change.action == DiffAction::Update
            && change
                .fields
                .iter()
                .any(|field| field.field == "permissions_bitmask")
    }));
    Ok(())
}

#[test]
fn diff_prueft_overwrite_bitmasken_exakt() -> anyhow::Result<()> {
    let mut desired = GuildModel::new(GUILD_ID);
    desired.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Role,
            target_id: ROLE_MEMBER,
        },
        overwrite(GENERAL, TargetKind::Role, ROLE_MEMBER, 0b1010, 0b0100),
    );

    let mut actual = GuildModel::new(GUILD_ID);
    actual.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Role,
            target_id: ROLE_MEMBER,
        },
        overwrite(GENERAL, TargetKind::Role, ROLE_MEMBER, 0b1000, 0b1100),
    );

    let diff = diff_models(&desired, &actual, &[], &[])?;
    assert_eq!(diff.changes.len(), 1);
    let fields: Vec<_> = diff.changes[0]
        .fields
        .iter()
        .map(|field| field.field.as_str())
        .collect();
    assert_eq!(fields, vec!["allow_bits", "deny_bits"]);
    Ok(())
}

#[test]
fn absolute_positionsunterschiede_bei_gleicher_reihenfolge_erzeugen_keinen_diff(
) -> anyhow::Result<()> {
    let mut desired = GuildModel::new(GUILD_ID);
    desired
        .categories
        .insert(CHAT_CATEGORY, category(CHAT_CATEGORY, "Chat", 1));
    desired
        .categories
        .insert(TEMPVOICE_PARENT, category(TEMPVOICE_PARENT, "TempVoice", 2));
    desired
        .channels
        .insert(GENERAL, channel(GENERAL, "allgemein", Some(CHAT_CATEGORY)));
    desired
        .roles
        .insert(ROLE_MEMBER, role(ROLE_MEMBER, "Member", 7));

    let mut actual = desired.clone();
    actual
        .categories
        .get_mut(&CHAT_CATEGORY)
        .expect("chat category")
        .position = 10;
    actual
        .categories
        .get_mut(&TEMPVOICE_PARENT)
        .expect("tempvoice category")
        .position = 20;
    actual
        .channels
        .get_mut(&GENERAL)
        .expect("general channel")
        .position = 99;
    actual
        .roles
        .get_mut(&ROLE_MEMBER)
        .expect("member role")
        .position = 42;

    let diff = diff_models(&desired, &actual, &[], &[])?;

    assert!(diff.is_empty());
    Ok(())
}

#[test]
fn dynamischer_namespace_filtert_kanaele_und_overwrites_separat_aus() -> anyhow::Result<()> {
    let desired = GuildModel::new(GUILD_ID);

    let mut actual = GuildModel::new(GUILD_ID);
    actual.categories.insert(
        TEMPVOICE_PARENT,
        category(TEMPVOICE_PARENT, "TempVoice", 10),
    );
    actual.channels.insert(
        TEMPVOICE_LANE,
        channel(TEMPVOICE_LANE, "lane-abc", Some(TEMPVOICE_PARENT)),
    );
    actual.overwrites.insert(
        OverwriteKey {
            channel_id: TEMPVOICE_LANE,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(TEMPVOICE_LANE, TargetKind::Member, USER_BANNED, 1, 2),
    );

    let namespace = DynamicNamespace {
        namespace_id: Some(7),
        namespace_key: "tempvoice".to_string(),
        system_name: "dl-voice".to_string(),
        match_rule: NamespaceMatch::ParentCategory(TEMPVOICE_PARENT),
    };

    let diff = diff_models(&desired, &actual, &[namespace], &[])?;

    assert_eq!(
        diff.changes.len(),
        1,
        "die Kategorie selbst ist nicht dynamisch"
    );
    assert_eq!(diff.filtered.len(), 2);
    assert!(diff.filtered.iter().all(|filtered| {
        matches!(
            filtered.reason,
            dl_server_as_code::FilterReason::DynamicNamespace { .. }
        )
    }));
    Ok(())
}

#[test]
fn faq_und_coaching_scrim_namespaces_filtern_channel_und_overwrite_drift() -> anyhow::Result<()> {
    let desired = GuildModel::new(GUILD_ID);
    let mut actual = GuildModel::new(GUILD_ID);
    actual.categories.insert(
        COACHING_SCRIM_PARENT,
        category(COACHING_SCRIM_PARENT, "Coaching/Scrim", 1),
    );
    actual.channels.insert(
        COACHING_TEAM_CHANNEL,
        channel(
            COACHING_TEAM_CHANNEL,
            "team-leo",
            Some(COACHING_SCRIM_PARENT),
        ),
    );
    actual
        .channels
        .insert(FAQ_CHANNEL, channel(FAQ_CHANNEL, "faq-testuser", None));
    for channel_id in [COACHING_TEAM_CHANNEL, FAQ_CHANNEL] {
        actual.overwrites.insert(
            OverwriteKey {
                channel_id,
                target_kind: TargetKind::Role,
                target_id: ROLE_MEMBER,
            },
            overwrite(channel_id, TargetKind::Role, ROLE_MEMBER, 1, 2),
        );
    }

    let namespaces = [
        DynamicNamespace {
            namespace_id: Some(9),
            namespace_key: "coaching_scrim_team_channels".to_string(),
            system_name: "Coaching/Scrim".to_string(),
            match_rule: NamespaceMatch::ParentCategory(COACHING_SCRIM_PARENT),
        },
        DynamicNamespace {
            namespace_id: Some(10),
            namespace_key: "faq_channels".to_string(),
            system_name: "AI-Onboarding/FAQ".to_string(),
            match_rule: NamespaceMatch::NamePrefix("faq-".to_string()),
        },
    ];

    let diff = diff_models(&desired, &actual, &namespaces, &[])?;

    assert_eq!(
        diff.changes.len(),
        1,
        "nur die Scrim-Kategorie selbst ist nicht dynamisch"
    );
    assert_eq!(diff.filtered.len(), 4);
    assert!(diff.blocked.is_empty());
    assert!(diff.filtered.iter().all(|filtered| {
        matches!(
            filtered.reason,
            dl_server_as_code::FilterReason::DynamicNamespace { .. }
        )
    }));
    Ok(())
}

#[test]
fn dokumentierte_ausnahme_filtert_user_ban_overwrite() -> anyhow::Result<()> {
    let desired = GuildModel::new(GUILD_ID);
    let mut actual = GuildModel::new(GUILD_ID);
    actual.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(GENERAL, TargetKind::Member, USER_BANNED, 0, 1024),
    );

    let exception = DocumentedException {
        exception_id: Some(42),
        exception_key: "X1".to_string(),
        object_kind: ObjectKind::PermissionOverwrite,
        channel_id: Some(GENERAL),
        target_kind: Some(TargetKind::Member),
        target_id: Some(USER_BANNED),
        allow_bits: Some(0),
        deny_bits: Some(1024),
        reason: "PLATZHALTER rust/crates/dl-server-as-code/tests/diff_engine.rs:201".to_string(),
    };

    let diff = diff_models(&desired, &actual, &[], &[exception])?;
    assert_eq!(diff.changes.len(), 0);
    assert_eq!(diff.filtered.len(), 1);
    assert!(matches!(
        diff.filtered[0].reason,
        dl_server_as_code::FilterReason::DocumentedException {
            exception_id: Some(42),
            ..
        }
    ));
    Ok(())
}

#[test]
fn dokumentierte_ausnahme_filtert_nur_exakten_actual_zustand() -> anyhow::Result<()> {
    let mut desired = GuildModel::new(GUILD_ID);
    desired.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(GENERAL, TargetKind::Member, USER_BANNED, 0, 1024),
    );

    let mut actual = GuildModel::new(GUILD_ID);
    actual.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(GENERAL, TargetKind::Member, USER_BANNED, 0, 512),
    );

    let exception = DocumentedException {
        exception_id: Some(42),
        exception_key: "X1".to_string(),
        object_kind: ObjectKind::PermissionOverwrite,
        channel_id: Some(GENERAL),
        target_kind: Some(TargetKind::Member),
        target_id: Some(USER_BANNED),
        allow_bits: Some(0),
        deny_bits: Some(1024),
        reason: "PLATZHALTER rust/crates/dl-server-as-code/tests/diff_engine.rs:278".to_string(),
    };

    let diff = diff_models(&desired, &actual, &[], &[exception])?;
    assert_eq!(diff.filtered.len(), 0);
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(diff.changes[0].action, DiffAction::Update);
    assert!(diff.changes[0]
        .fields
        .iter()
        .any(|field| field.field == "deny_bits"));
    Ok(())
}

#[test]
fn dokumentierte_ausnahme_filtert_nicht_wenn_actual_overwrite_entfernt_wurde() -> anyhow::Result<()>
{
    let mut desired = GuildModel::new(GUILD_ID);
    desired.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(GENERAL, TargetKind::Member, USER_BANNED, 0, 1024),
    );

    let actual = GuildModel::new(GUILD_ID);
    let exception = DocumentedException {
        exception_id: Some(42),
        exception_key: "X1".to_string(),
        object_kind: ObjectKind::PermissionOverwrite,
        channel_id: Some(GENERAL),
        target_kind: Some(TargetKind::Member),
        target_id: Some(USER_BANNED),
        allow_bits: Some(0),
        deny_bits: Some(1024),
        reason: "PLATZHALTER rust/crates/dl-server-as-code/tests/diff_engine.rs:334".to_string(),
    };

    let diff = diff_models(&desired, &actual, &[], &[exception])?;
    assert_eq!(diff.filtered.len(), 0);
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(diff.changes[0].action, DiffAction::Create);
    assert_eq!(diff.changes[0].object.kind, ObjectKind::PermissionOverwrite);
    Ok(())
}

#[test]
fn dokumentierte_ausnahme_mit_allow_und_deny_filtert_nur_exakt() -> anyhow::Result<()> {
    let exception = DocumentedException {
        exception_id: Some(43),
        exception_key: "X2".to_string(),
        object_kind: ObjectKind::PermissionOverwrite,
        channel_id: Some(GENERAL),
        target_kind: Some(TargetKind::Member),
        target_id: Some(USER_BANNED),
        allow_bits: Some(64),
        deny_bits: Some(1024),
        reason: "PLATZHALTER rust/crates/dl-server-as-code/tests/diff_engine.rs:362".to_string(),
    };

    let mut desired = GuildModel::new(GUILD_ID);
    desired.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(GENERAL, TargetKind::Member, USER_BANNED, 64, 1024),
    );
    let mut actual = GuildModel::new(GUILD_ID);
    actual.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(GENERAL, TargetKind::Member, USER_BANNED, 32, 1024),
    );

    let drift = diff_models(&desired, &actual, &[], std::slice::from_ref(&exception))?;
    assert_eq!(drift.filtered.len(), 0);
    assert_eq!(drift.changes.len(), 1);
    assert_eq!(drift.changes[0].action, DiffAction::Update);
    assert_eq!(
        drift.changes[0]
            .fields
            .iter()
            .map(|field| field.field.as_str())
            .collect::<Vec<_>>(),
        vec!["allow_bits"]
    );

    let desired = GuildModel::new(GUILD_ID);
    let mut actual = GuildModel::new(GUILD_ID);
    actual.overwrites.insert(
        OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: USER_BANNED,
        },
        overwrite(GENERAL, TargetKind::Member, USER_BANNED, 64, 1024),
    );

    let exact = diff_models(&desired, &actual, &[], &[exception])?;
    assert_eq!(exact.changes.len(), 0);
    assert_eq!(exact.filtered.len(), 1);
    assert!(matches!(
        exact.filtered[0].reason,
        dl_server_as_code::FilterReason::DocumentedException {
            exception_id: Some(43),
            ..
        }
    ));
    Ok(())
}

#[test]
fn ticket_namespace_filtert_overwrite_drift_nicht() -> anyhow::Result<()> {
    let ticket_channel = 901;
    let mut desired = GuildModel::new(GUILD_ID);
    desired
        .channels
        .insert(ticket_channel, channel(ticket_channel, "ticket-17", None));
    desired.overwrites.insert(
        OverwriteKey {
            channel_id: ticket_channel,
            target_kind: TargetKind::Role,
            target_id: GUILD_ID,
        },
        overwrite(ticket_channel, TargetKind::Role, GUILD_ID, 0, 1024),
    );

    let mut actual = desired.clone();
    actual
        .overwrites
        .get_mut(&OverwriteKey {
            channel_id: ticket_channel,
            target_kind: TargetKind::Role,
            target_id: GUILD_ID,
        })
        .expect("ticket overwrite")
        .deny_bits = 0;

    let namespace = DynamicNamespace {
        namespace_id: Some(8),
        namespace_key: "ticket_channels".to_string(),
        system_name: "TicketTool".to_string(),
        match_rule: NamespaceMatch::NamePattern(r"^(ticket|closed)-[0-9]+$".to_string()),
    };

    let diff = diff_models(&desired, &actual, &[namespace], &[])?;
    assert_eq!(diff.filtered.len(), 0);
    assert_eq!(diff.changes.len(), 1);
    assert_eq!(diff.changes[0].object.kind, ObjectKind::PermissionOverwrite);
    Ok(())
}

#[test]
fn menschenlesbare_ausgabe_beschreibt_leeren_und_vollen_diff() -> anyhow::Result<()> {
    // Leerer Diff: klare Alles-in-Ordnung-Meldung.
    let desired = GuildModel::new(GUILD_ID);
    let actual = GuildModel::new(GUILD_ID);
    let diff = diff_models(&desired, &actual, &[], &[])?;
    let summary = format::human_summary(&diff);
    assert!(summary.contains("0 Änderung(en)"));
    assert!(summary.contains("Keine Abweichungen"));

    // Voller Diff: alle drei Aktionen mit Objektnamen.
    let mut desired = GuildModel::new(GUILD_ID);
    let mut actual = GuildModel::new(GUILD_ID);
    // Create: Kanal existiert nur im Soll.
    desired
        .channels
        .insert(GENERAL, channel(GENERAL, "frag-die-community", None));
    // Update: Kategorie mit abweichendem Namen.
    desired
        .categories
        .insert(CHAT_CATEGORY, category(CHAT_CATEGORY, "Chat", 1));
    actual
        .categories
        .insert(CHAT_CATEGORY, category(CHAT_CATEGORY, "Chat-Alt", 1));
    // Delete: Rolle existiert nur im Ist.
    actual
        .roles
        .insert(ROLE_MEMBER, role(ROLE_MEMBER, "Zombie-Rolle", 0));

    let diff = diff_models(&desired, &actual, &[], &[])?;
    let summary = format::human_summary(&diff);
    assert!(summary.contains("ANLEGEN: Kanal „frag-die-community“"));
    assert!(summary.contains("ÄNDERN: Kategorie „Chat“"));
    assert!(summary.contains("Felder: name"));
    assert!(summary.contains("LÖSCHEN (manueller Schritt"));
    assert!(summary.contains("„Zombie-Rolle“"));
    Ok(())
}
