use anyhow::{Context, Result, ensure};
use brain_maintenance::{
    digest,
    integration::{
        activation::ActivationPlan,
        artifacts::Artifacts,
        config_writer::ConfigWriter,
        runner::{load_maintenance, registration_id, require_operator_config},
        runtime_config::RuntimeConfig,
    },
};
use brain_storage::PgStore;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() == 2 || (args.len() == 3 && args[2] == "--apply"),
        "expected_release_and_optional_apply_required"
    );
    let apply = args.len() == 3;
    let runtime = RuntimeConfig::load(Path::new("/etc/deadlock-brain/maintenance-runtime.json"))?;
    require_operator_config(&runtime.maintenance_config)?;
    let maintenance_bytes = std::fs::read(&runtime.maintenance_config)?;
    let maintenance = load_maintenance(&runtime.maintenance_config)?;
    let bots = maintenance
        .repositories
        .iter()
        .find(|repo| repo.id == "Deadlock-Bots")
        .context("bots_owner_missing")?;
    let docs = maintenance
        .repositories
        .iter()
        .find(|repo| repo.id == "Deadlock-Docs")
        .context("docs_owner_missing")?;
    let targets = [
        "public/discord-server/dm-concierge.html",
        "public/discord-server/paten.html",
    ];
    for target in targets {
        ensure!(
            docs.doc_targets.iter().any(|item| item == target)
                && !bots.doc_targets.iter().any(|item| item == target),
            "owner_transition_not_registered"
        );
    }
    let retired: Vec<_> = targets
        .iter()
        .map(|target| format!("maintenance-docs:{}", registration_id(bots, target)))
        .collect();
    let secrets: BTreeMap<_, _> = dl_token_secrets::values(&runtime.infisical_config)
        .await
        .map_err(|_| anyhow::anyhow!("secret_source_unavailable"))?
        .into_iter()
        .collect();
    let pg = &runtime.postgres;
    ensure!(
        pg.username == "brain_ingest" && pg.auth == brain_serve::config::DatabaseAuth::Password,
        "existing_ingest_role_required"
    );
    let password = secrets
        .get(pg.password_env.as_deref().context("postgres_secret_name")?)
        .context("postgres_secret_missing")?;
    let options = PgConnectOptions::new_without_pgpass()
        .host(pg.socket_dir.to_str().context("postgres_socket")?)
        .port(pg.port)
        .username(&pg.username)
        .database(&pg.database)
        .password(password)
        .ssl_mode(sqlx::postgres::PgSslMode::Disable)
        .options([("statement_timeout", "120000"), ("lock_timeout", "10000")]);
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(30))
        .connect_with(options)
        .await
        .map_err(|_| anyhow::anyhow!("maintenance_database_unavailable"))?;
    let store = PgStore::new(pool);
    store.check_maintenance_schema().await?;
    let artifacts = Artifacts::open(&runtime.artifact_dir)?;
    let locked_artifacts = artifacts.clone();
    let _publication_lock =
        tokio::task::spawn_blocking(move || locked_artifacts.publication_lock()).await??;
    let writer = ConfigWriter::lock_async(runtime.serve_config.clone()).await?;
    let before = writer.read()?;
    let serve =
        brain_serve::Config::parse(&before).map_err(|_| anyhow::anyhow!("serve_config_invalid"))?;
    ensure!(serve.release.id == args[1], "active_release_changed");
    let base = store.snapshot(&serve.release.id).await?;
    ensure!(
        base.release.knowledge_version == serve.release.knowledge_version,
        "active_knowledge_version_changed"
    );
    let independent: BTreeSet<_> = serve
        .credentials
        .iter()
        .filter_map(|grant| grant.release.as_ref())
        .chain(
            serve
                .internal_operator
                .iter()
                .map(|operator| &operator.release),
        )
        .map(|pin| pin.id.clone())
        .filter(|id| id != &serve.release.id)
        .collect();
    for id in independent {
        let other = store.snapshot(&id).await?;
        ensure!(
            retired
                .iter()
                .all(|source| !other.release.source_revisions.contains_key(source)),
            "independent_release_still_pins_retired_source"
        );
    }
    let expected_documents = [
        (
            "public/discord-server/dm-concierge.html",
            "b79779609c5789d52ab59f235fced1cc9f29bd1f3287f1d49b775a2687d19b35",
        ),
        (
            "public/discord-server/paten.html",
            "a5fee61821e32d594829ff2ac78e52de88a3e51b4a61ea9ce055fb348a88a019",
        ),
        (
            "public/discord-server/coaching.html",
            "95c893a6eb413d7c8cd88fa8ecdbb269f5140b8643f5dd0f565cb289710eff5c",
        ),
        (
            "public/discord-server/faq-bot-selbst.html",
            "b767ad0d233997c2985d541589142ff9c07a5e100055400e33f62110d235edba",
        ),
        (
            "public/discord-server/module/brain.html",
            "b0ccef65ff08051b8b012efbb6aaa527e6bef5f075cc1dac93939f4e24fa3a1f",
        ),
    ];
    for (target, hash) in expected_documents {
        let source = format!("maintenance-docs:{}", registration_id(docs, target));
        ensure!(
            base.revisions
                .iter()
                .any(|record| record.source_id == source
                    && record.logical_id == target
                    && record.content_hash == hash
                    && base
                        .release
                        .source_revisions
                        .get(&source)
                        .and_then(|pins| pins.get(target))
                        == Some(&record.revision)),
            "corrected_document_not_pinned"
        );
    }
    let mut next = base.release.clone();
    for (source, target) in retired.iter().zip(targets) {
        let pins = next
            .source_revisions
            .get(source)
            .context("old_source_not_pinned")?;
        ensure!(
            pins.len() == 1 && pins.contains_key(target),
            "retirement_would_remove_unrelated_document"
        );
        next.source_revisions.remove(source);
    }
    next.created_at_epoch = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())?;
    let key = digest(&serde_json::to_vec(&next)?);
    next.release_id = format!("maintenance-retire-{key}");
    next.knowledge_version = format!("docs-retire-{key}");
    ensure!(
        std::fs::read(&runtime.maintenance_config)? == maintenance_bytes,
        "maintenance_config_changed"
    );
    if !apply {
        println!(
            "{}",
            serde_json::json!({"status":"planned", "base_release":base.release.release_id, "removed_sources":retired, "corrected_documents_checked":5, "config_sha256":digest(&before)})
        );
        return Ok(());
    }
    let plan = ActivationPlan::prepare(&runtime, &artifacts, &next, &writer)?;
    store.publish_release(&next).await?;
    plan.wait_for_restart_window().await;
    let proof = match plan.activate(&writer).await {
        Ok(proof) => proof,
        Err(error) => {
            plan.rollback(&writer)
                .await
                .context("transition_rollback_failed")?;
            return Err(error.context("transition_activation_failed_and_rolled_back"));
        }
    };
    println!(
        "{}",
        serde_json::json!({"status":"activated", "base_release":base.release.release_id, "removed_sources":retired, "corrected_documents_checked":5, "activation":proof})
    );
    Ok(())
}
