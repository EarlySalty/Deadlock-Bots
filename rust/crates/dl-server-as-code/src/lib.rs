//! Server-as-Code fuer die Discord-Guild-Struktur.
//!
//! Das Crate haelt die reine Diff-Logik getrennt von Postgres und Serenity.
//! Live-Schreibzugriffe passieren nur ueber `apply_preview` mit bestaetigtem
//! Diff-Hash und sind standardmaessig Dry-Run.
//!
//! BotMessages/Panels sind in Phase 1 nur als Soll/Ist-Modell sichtbar.
//! Serenity-Import und Apply fuer diese Nachrichten bleiben Phase-2-Folgearbeit;
//! Apply meldet sie deshalb explizit als `skipped: not implemented`.

pub mod apply;
pub mod db;
pub mod diff;
pub mod drift;
pub mod format;
pub mod import;
pub mod model;

pub use apply::{apply_preview, ApplyChangeResult, ApplyOptions, ApplyReport};
pub use db::{
    adopt_change, load_desired_model, load_snapshot_model, persist_diff_preview, DiffPreview,
};
pub use diff::{
    diff_models, diff_models_with_options, DiffAction, DiffChange, DiffOptions, FieldDiff,
    FilterReason, ServerDiff,
};
pub use drift::{detect_and_record_drift, AutoRevertRule, DriftRecord};
pub use import::{import_live_guild_snapshot, SnapshotImportReport};
pub use model::{
    BotMessageSpec, CategorySpec, ChannelKind, ChannelSpec, DiscordId, DocumentedException,
    DynamicNamespace, GuildModel, NamespaceMatch, ObjectKind, ObjectRef, OverwriteKey,
    PermissionOverwriteSpec, RoleSpec, TargetKind,
};

pub const DEFAULT_GUILD_ID: u64 = 1_289_721_245_281_292_288;

#[derive(Debug, thiserror::Error)]
pub enum ServerAsCodeError {
    #[error("Discord-ID {value} passt nicht in BIGINT")]
    IdOutOfRange { value: u64 },
    #[error("Bitmaske {value} passt nicht in BIGINT")]
    BitmaskOutOfRange { value: u64 },
    #[error("unbekannter Kanaltyp: {0}")]
    UnknownChannelKind(String),
    #[error("unbekannter Overwrite-Zieltyp: {0}")]
    UnknownTargetKind(String),
    #[error("unbekannte Objektart: {0}")]
    UnknownObjectKind(String),
    #[error("Diff-Hash stimmt nicht: erwartet {expected}, bekommen {actual}")]
    DiffHashMismatch { expected: String, actual: String },
    #[error("Diff-Preview {0} nicht gefunden")]
    PreviewNotFound(i64),
    #[error("Snapshot {0} nicht gefunden")]
    SnapshotNotFound(i64),
    #[error("JSON konnte nicht serialisiert werden: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Serenity(Box<serenity::Error>),
    #[error("Regex-Regel {pattern} ist ungueltig: {source}")]
    Regex {
        pattern: String,
        source: regex::Error,
    },
}

pub type Result<T> = std::result::Result<T, ServerAsCodeError>;

impl From<serenity::Error> for ServerAsCodeError {
    fn from(source: serenity::Error) -> Self {
        Self::Serenity(Box::new(source))
    }
}

pub(crate) fn id_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| ServerAsCodeError::IdOutOfRange { value })
}

pub(crate) fn i64_to_id(value: i64) -> u64 {
    value as u64
}

pub(crate) fn bitmask_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| ServerAsCodeError::BitmaskOutOfRange { value })
}

pub(crate) fn i64_to_bitmask(value: i64) -> u64 {
    value as u64
}
