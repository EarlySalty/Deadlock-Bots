//! Plattform-Verknuepfungen aus dem Discord-Profil
//! (`core.discord_platform_connections`).
//!
//! Schreiber ist das Dashboard (delegierter Discord-Login mit Scope
//! `identify connections`), Leser sind der Broker-Endpunkt `twitch-links` und
//! spaetere Pakete (Leaderboard, Concierge). Schluessel sind ausschliesslich
//! Discord-ID und Plattform-User-ID; der Login ist nur Anzeige.
//!
//! Datenschutz: Mitglieder mit Privacy-Grabstein (`core.user_privacy`
//! `opted_out` oder `deleted_at`) werden weder geschrieben noch gelesen.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::{lock_user_privacy_and_is_opted_out, CentralDbError};

/// Plattform-Kennung fuer Twitch in `core.discord_platform_connections`.
pub const PLATFORM_TWITCH: &str = "twitch";

/// Obergrenze fuer [`discord_ids_for_twitch_ids`] je Aufruf.
pub const MAX_TWITCH_IDS_PER_LOOKUP: usize = 5_000;

/// Serialisiert Twitch-Kontozuordnung, Viewer-Import und Löschung.
/// Diese Sperre muss vor allen nutzerbezogenen Privacy-Sperren stehen.
pub async fn lock_twitch_identity(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), sqlx::Error> {
    lock_twitch_identity_connection(tx).await
}

/// Dieselbe Sperre für Invitewriter, vor Guild-, Nutzer- und Zeilenlocks.
pub async fn lock_twitch_identity_connection(
    conn: &mut sqlx::PgConnection,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('core.discord_platform_connections'), hashtext('twitch_reassignment'))")
        .fetch_one(conn).await?;
    Ok(())
}

/// Eine Twitch-Verbindung, wie Discord sie fuer das Mitglied liefert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwitchConnection {
    pub twitch_user_id: String,
    pub twitch_login: String,
    pub verified: bool,
}

/// Gespeicherte Twitch-Verknuepfung eines Discord-Mitglieds.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct TwitchLink {
    pub discord_id: i64,
    pub twitch_user_id: String,
    pub twitch_login: String,
    pub verified: bool,
    pub updated_at: DateTime<Utc>,
}

/// Ergebnis von [`upsert_twitch_connection`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TwitchUpsertOutcome {
    /// Gespeichert. `replaced_discord_ids` sind Discord-IDs, deren Verknuepfung
    /// mit demselben Twitch-Konto dabei entfernt wurde (ein Twitch-Konto gehoert
    /// hoechstens einem Mitglied).
    Linked { replaced_discord_ids: Vec<i64> },
    /// Das Mitglied hat der Verarbeitung widersprochen; nichts gespeichert.
    PrivacyOptedOut,
}

/// Prueft eine Twitch-User-ID: nur Ziffern, keine fuehrende Null, passt in u64.
pub fn is_valid_twitch_user_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 20
        && !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

/// Speichert (oder aktualisiert) die Twitch-Verknuepfung eines Mitglieds.
///
/// Idempotent: derselbe Aufruf mehrfach ergibt dieselbe Zeile (nur
/// `updated_at` wandert). War dasselbe Twitch-Konto bisher einer anderen
/// Discord-ID zugeordnet, wird diese alte Zuordnung in derselben Transaktion
/// entfernt.
pub async fn upsert_twitch_connection(
    pool: &PgPool,
    discord_id: i64,
    connection: &TwitchConnection,
) -> Result<TwitchUpsertOutcome, CentralDbError> {
    let twitch_user_id = connection.twitch_user_id.trim();
    let twitch_login = connection.twitch_login.trim();
    if discord_id <= 0 || !is_valid_twitch_user_id(twitch_user_id) || twitch_login.is_empty() {
        return Err(CentralDbError::InvalidInput(
            "ungueltige Twitch-Verknuepfung".to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    // Kontowechsel verschiedener Mitglieder dürfen nicht auf demselben
    // Unique-Index konkurrieren. Der Lock gilt auch für getauschte Konten.
    lock_twitch_identity(&mut tx).await?;
    if lock_user_privacy_and_is_opted_out(&mut tx, discord_id).await? {
        tx.commit().await?;
        return Ok(TwitchUpsertOutcome::PrivacyOptedOut);
    }

    crate::community_points::consent_after_twitch_link(&mut tx, discord_id, twitch_user_id).await?;

    let replaced_discord_ids: Vec<i64> = sqlx::query_scalar(
        "DELETE FROM core.discord_platform_connections
          WHERE platform = $1
            AND platform_user_id = $2
            AND discord_id <> $3
         RETURNING discord_id",
    )
    .bind(PLATFORM_TWITCH)
    .bind(twitch_user_id)
    .bind(discord_id)
    .fetch_all(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO core.discord_platform_connections
             (discord_id, platform, platform_user_id, platform_login, verified, updated_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (discord_id, platform) DO UPDATE SET
             platform_user_id = EXCLUDED.platform_user_id,
             platform_login = EXCLUDED.platform_login,
             verified = EXCLUDED.verified,
             updated_at = EXCLUDED.updated_at",
    )
    .bind(discord_id)
    .bind(PLATFORM_TWITCH)
    .bind(twitch_user_id)
    .bind(twitch_login)
    .bind(connection.verified)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(TwitchUpsertOutcome::Linked {
        replaced_discord_ids,
    })
}

const TWITCH_LINK_COLUMNS: &str = "c.discord_id,
        c.platform_user_id AS twitch_user_id,
        c.platform_login AS twitch_login,
        c.verified,
        c.updated_at";

const NOT_OPTED_OUT: &str = "NOT EXISTS (
            SELECT 1 FROM core.user_privacy p
             WHERE p.user_id = c.discord_id
               AND (p.opted_out = TRUE OR p.deleted_at IS NOT NULL)
        )";

/// Alle Twitch-Verknuepfungen (ohne Mitglieder mit Privacy-Grabstein),
/// sortiert nach Discord-ID.
pub async fn list_twitch_links(pool: &PgPool) -> Result<Vec<TwitchLink>, CentralDbError> {
    let sql = format!(
        "SELECT {TWITCH_LINK_COLUMNS}
           FROM core.discord_platform_connections c
          WHERE c.platform = $1
            AND {NOT_OPTED_OUT}
          ORDER BY c.discord_id"
    );
    let rows = sqlx::query_as::<_, TwitchLink>(&sql)
        .bind(PLATFORM_TWITCH)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Twitch-Verknuepfung eines Discord-Mitglieds, falls vorhanden.
pub async fn twitch_link_for_discord(
    pool: &PgPool,
    discord_id: i64,
) -> Result<Option<TwitchLink>, CentralDbError> {
    let sql = format!(
        "SELECT {TWITCH_LINK_COLUMNS}
           FROM core.discord_platform_connections c
          WHERE c.platform = $1
            AND c.discord_id = $2
            AND {NOT_OPTED_OUT}"
    );
    let row = sqlx::query_as::<_, TwitchLink>(&sql)
        .bind(PLATFORM_TWITCH)
        .bind(discord_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Ordnet Twitch-User-IDs den verknuepften Discord-IDs zu. Nicht verknuepfte
/// (oder ungueltige) IDs fehlen in der Antwort. Hoechstens
/// [`MAX_TWITCH_IDS_PER_LOOKUP`] IDs je Aufruf; mehr ist ein Fehler, damit
/// Aufrufer seitenweise lesen.
pub async fn discord_ids_for_twitch_ids(
    pool: &PgPool,
    twitch_user_ids: &[String],
) -> Result<HashMap<String, i64>, CentralDbError> {
    if twitch_user_ids.len() > MAX_TWITCH_IDS_PER_LOOKUP {
        return Err(CentralDbError::InvalidInput(format!(
            "zu viele Twitch-IDs je Abfrage (max {MAX_TWITCH_IDS_PER_LOOKUP})"
        )));
    }
    let ids: Vec<String> = twitch_user_ids
        .iter()
        .map(|id| id.trim())
        .filter(|id| is_valid_twitch_user_id(id))
        .map(str::to_string)
        .collect();
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let sql = format!(
        "SELECT c.platform_user_id, c.discord_id
           FROM core.discord_platform_connections c
          WHERE c.platform = $1
            AND c.platform_user_id = ANY($2)
            AND {NOT_OPTED_OUT}"
    );
    let rows: Vec<(String, i64)> = sqlx::query_as(&sql)
        .bind(PLATFORM_TWITCH)
        .bind(&ids)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitch_user_id_pruefung() {
        assert!(is_valid_twitch_user_id("123456"));
        assert!(is_valid_twitch_user_id("18446744073709551615"));
        assert!(!is_valid_twitch_user_id(""));
        assert!(!is_valid_twitch_user_id("0123"));
        assert!(!is_valid_twitch_user_id("12a"));
        assert!(!is_valid_twitch_user_id("-5"));
        assert!(!is_valid_twitch_user_id("18446744073709551616"));
    }
}
