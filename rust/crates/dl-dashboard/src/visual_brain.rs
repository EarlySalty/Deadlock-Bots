use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::web::{err_text, DashboardApp};

const MAX_NODES: usize = 1500;
const MAX_LINKS: usize = 6000;
const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
struct Config {
    site_root: PathBuf,
    #[serde(default)]
    knowledge_status_path: Option<PathBuf>,
}

#[derive(Deserialize, Serialize)]
struct KnowledgeStatus {
    schema_version: u32,
    generated_at: String,
    active_snapshot: String,
    refresh_status: RefreshStatus,
    repositories: Vec<RepositoryStatus>,
    totals: KnowledgeCounts,
    gaps: Vec<KnowledgeGap>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum RefreshStatus {
    Ok,
    Failed,
}

#[derive(Deserialize, Serialize)]
struct KnowledgeCounts {
    public_documents: u64,
    verified_documents: u64,
    pending_review: u64,
    excluded_documents: u64,
}

#[derive(Deserialize, Serialize)]
struct RepositoryStatus {
    id: String,
    label: String,
    revision: String,
    #[serde(flatten)]
    counts: KnowledgeCounts,
    last_verified_at: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum GapStatus {
    PendingReview,
    Excluded,
}

#[derive(Deserialize, Serialize)]
struct KnowledgeGap {
    source_id: String,
    title: String,
    reason: String,
    status: GapStatus,
}

/// A fixed, configured snapshot; never accepts a caller-supplied file path.
pub async fn knowledge_status(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return private_response(response);
    }
    match load_knowledge_status(&app).await {
        Ok(status) => private_response(axum::Json(status).into_response()),
        Err(()) => private_response(err_text(
            503,
            "Noch kein belegter Wissensprüfstand verfügbar.",
        )),
    }
}

async fn load_knowledge_status(app: &DashboardApp) -> Result<KnowledgeStatus, ()> {
    let repo = app.repo_root().ok_or(())?;
    let config = tokio::fs::read(repo.join("assets/visual-brain.json"))
        .await
        .map_err(|_| ())?;
    let config: Config = serde_json::from_slice(&config).map_err(|_| ())?;
    let path = config
        .knowledge_status_path
        .filter(|path| path.is_absolute())
        .ok_or(())?;
    read_knowledge_status(&path).await
}

async fn read_knowledge_status(path: &Path) -> Result<KnowledgeStatus, ()> {
    let file = tokio::fs::File::open(path).await.map_err(|_| ())?;
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| ())?;
    if bytes.len() > 1024 * 1024 {
        return Err(());
    }
    let status: KnowledgeStatus = serde_json::from_slice(&bytes).map_err(|_| ())?;
    if status.schema_version != 1
        || chrono::DateTime::parse_from_rfc3339(&status.generated_at).is_err()
    {
        return Err(());
    }
    Ok(status)
}

#[cfg(test)]
mod knowledge_status_tests {
    use super::*;

    #[tokio::test]
    async fn status_ist_typisiert_begrenzt_und_keine_rohdatei() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("status.json");
        let value = serde_json::json!({
            "schema_version": 1, "generated_at": "2026-09-20T00:00:00Z",
            "active_snapshot":"abc123", "refresh_status":"ok",
            "repositories":[], "gaps":[],
            "totals":{"public_documents":3,"verified_documents":1,"pending_review":2,"excluded_documents":0},
            "debug_private_path":"must-not-be-returned"
        });
        std::fs::write(&path, value.to_string()).expect("fixture");
        let status = read_knowledge_status(&path).await.expect("status");
        let output = serde_json::to_value(status).expect("json");
        assert_eq!(output["totals"]["verified_documents"], 1);
        assert!(output.get("debug_private_path").is_none());
        std::fs::write(&path, vec![b' '; 1024 * 1024 + 1]).expect("oversize fixture");
        assert!(read_knowledge_status(&path).await.is_err());
        std::fs::write(&path, "{}").expect("invalid fixture");
        assert!(read_knowledge_status(&path).await.is_err());
    }
}

#[derive(Default, Deserialize)]
struct Metadata {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    doc_group: String,
    #[serde(default)]
    stand: String,
}

#[derive(Deserialize)]
struct RawNode {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    source_file: String,
    #[serde(default)]
    source_location: String,
    #[serde(default)]
    metadata: Metadata,
}

#[derive(Clone, Deserialize, Serialize)]
struct Link {
    source: String,
    target: String,
    #[serde(default)]
    relation: String,
    #[serde(default)]
    confidence: String,
}

#[derive(Deserialize)]
struct RawGraph {
    nodes: Vec<RawNode>,
    links: Vec<Link>,
}

#[derive(Serialize)]
struct Node {
    id: String,
    label: String,
    source_file: String,
    source_location: String,
    kind: String,
    group: String,
    stand: String,
    evidence_links: usize,
}

#[derive(Serialize)]
struct Graph {
    nodes: Vec<Node>,
    links: Vec<Link>,
    source_nodes: usize,
    source_links: usize,
    corpus_nodes: usize,
    unlinked_documents: usize,
    omitted_nodes: usize,
    omitted_links: usize,
    generated_at: String,
}

#[derive(PartialEq)]
struct Stamp {
    path: PathBuf,
    modified: SystemTime,
    len: u64,
}

struct Cached {
    stamp: Stamp,
    loaded_at: Instant,
    value: Result<Bytes, &'static str>,
}

static GRAPH_CACHE: OnceLock<Arc<Mutex<Option<Cached>>>> = OnceLock::new();

async fn site_root(app: &DashboardApp) -> Result<PathBuf, Response> {
    let Some(repo) = app.repo_root() else {
        return Err(err_text(503, "Wissenskarte ist noch nicht eingerichtet."));
    };
    let bytes = tokio::fs::read(repo.join("assets/visual-brain.json"))
        .await
        .map_err(|error| {
            tracing::warn!(kind = ?error.kind(), "Wissenskarten-Konfiguration nicht lesbar");
            err_text(503, "Wissenskarte ist noch nicht eingerichtet.")
        })?;
    match serde_json::from_slice::<Config>(&bytes) {
        Ok(config) if config.site_root.is_absolute() => Ok(config.site_root),
        _ => {
            tracing::warn!("Wissenskarten-Konfiguration ungültig");
            Err(err_text(
                503,
                "Die Konfiguration der Wissenskarte ist ungültig.",
            ))
        }
    }
}

fn private_response(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Cookie"));
    response
}

pub async fn graph(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return private_response(response);
    }
    let root = match site_root(&app).await {
        Ok(root) => root,
        Err(response) => return private_response(response),
    };
    match cached_graph(root.join("brain/model.json")).await {
        Ok(bytes) => private_response(
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
                bytes,
            )
                .into_response(),
        ),
        Err(message) => private_response(err_text(503, message)),
    }
}

pub async fn library(State(app): State<DashboardApp>, request: Request) -> Response {
    if let Err(response) = app.guard_full(request.headers()).await {
        return private_response(response);
    }
    let root = match site_root(&app).await {
        Ok(root) => root,
        Err(response) => return private_response(response),
    };
    match ServeFile::new(root.join("vendor/vis-network.min.js"))
        .oneshot(request)
        .await
    {
        Ok(response) => private_response(response.map(Body::new)),
        Err(_) => private_response(err_text(503, "Graphanzeige ist nicht verfügbar.")),
    }
}

pub async fn ui(State(app): State<DashboardApp>, headers: HeaderMap) -> Response {
    if let Err(response) = app.guard_full(&headers).await {
        return private_response(response);
    }
    private_response(
        (
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            include_str!("../../../../service/static/visual-brain.js"),
        )
            .into_response(),
    )
}

async fn cached_graph(path: PathBuf) -> Result<Bytes, &'static str> {
    let metadata = tokio::fs::metadata(&path)
        .await
        .map_err(|_| "Die Wissenskarte wurde noch nicht bereitgestellt.")?;
    if metadata.len() > MAX_SOURCE_BYTES {
        return Err("Die Wissenskarte ist zu groß und muss neu aufbereitet werden.");
    }
    let stamp = Stamp {
        path,
        modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        len: metadata.len(),
    };
    let mut cache = GRAPH_CACHE
        .get_or_init(|| Arc::new(Mutex::new(None)))
        .clone()
        .lock_owned()
        .await;
    if let Some(cached) = cache.as_ref() {
        if cached.stamp == stamp
            && (cached.value.is_ok() || cached.loaded_at.elapsed().as_secs() < 30)
        {
            return cached.value.clone();
        }
    }
    let source = stamp.path.clone();
    let modified = stamp.modified;
    tokio::task::spawn_blocking(move || {
        let value = load_graph(&source, modified);
        *cache = Some(Cached {
            stamp,
            loaded_at: Instant::now(),
            value: value.clone(),
        });
        value
    })
    .await
    .map_err(|_| "Die Wissenskarte konnte nicht aufbereitet werden.")?
}

fn load_graph(path: &Path, modified: SystemTime) -> Result<Bytes, &'static str> {
    let file =
        std::fs::File::open(path).map_err(|_| "Die Wissenskarte konnte nicht geöffnet werden.")?;
    if file
        .metadata()
        .map_err(|_| "Wissenskarten-Datei nicht lesbar.")?
        .len()
        > MAX_SOURCE_BYTES
    {
        return Err("Die Wissenskarte ist zu groß und muss neu aufbereitet werden.");
    }
    let reader = std::io::BufReader::with_capacity(1024 * 1024, file.take(MAX_SOURCE_BYTES + 1));
    let raw: RawGraph = serde_json::from_reader(reader)
        .map_err(|_| "Die Wissenskarte ist beschädigt und muss neu erstellt werden.")?;
    let graph = reduce_graph(raw, modified)?;
    serde_json::to_vec(&graph)
        .map(Bytes::from)
        .map_err(|_| "Die Wissenskarte konnte nicht aufbereitet werden.")
}

struct Selection {
    seeds: HashSet<String>,
    selected: HashSet<String>,
    omitted: usize,
}

fn select_nodes(raw: &RawGraph) -> Result<Selection, &'static str> {
    let existing: HashSet<_> = raw.nodes.iter().map(|node| node.id.as_str()).collect();
    let seeds: HashSet<_> = raw
        .nodes
        .iter()
        .filter(|node| node.metadata.kind == "corpus")
        .map(|node| node.id.clone())
        .collect();
    if seeds.is_empty() {
        return Err("Die Wissenskarte enthält noch keine Wissensdokumente.");
    }
    if seeds.len() > MAX_NODES {
        return Err(
            "Zu viele Wissensdokumente für diese Ansicht. Die Karte muss aufgeteilt werden.",
        );
    }
    let mut selected = seeds.clone();
    let direct: HashSet<_> = raw
        .links
        .iter()
        .filter(|link| {
            link.relation == "documents"
                && seeds.contains(&link.source)
                && existing.contains(link.target.as_str())
        })
        .map(|link| link.target.clone())
        .collect();
    add_sorted_nodes(&mut selected, direct);
    let mut neighbors = HashSet::new();
    for link in &raw.links {
        if selected.contains(&link.source)
            && !selected.contains(&link.target)
            && existing.contains(link.target.as_str())
        {
            neighbors.insert(link.target.clone());
        } else if selected.contains(&link.target)
            && !selected.contains(&link.source)
            && existing.contains(link.source.as_str())
        {
            neighbors.insert(link.source.clone());
        }
    }
    let omitted = neighbors.len().saturating_sub(MAX_NODES - selected.len());
    add_sorted_nodes(&mut selected, neighbors);
    Ok(Selection {
        seeds,
        selected,
        omitted,
    })
}

fn add_sorted_nodes(selected: &mut HashSet<String>, candidates: HashSet<String>) {
    let mut candidates: Vec<_> = candidates
        .into_iter()
        .filter(|id| !selected.contains(id))
        .collect();
    candidates.sort();
    selected.extend(candidates.into_iter().take(MAX_NODES - selected.len()));
}

fn node_view(node: RawNode, evidence_links: usize) -> Node {
    Node {
        id: node.id,
        label: node.label,
        source_file: node.source_file,
        source_location: node.source_location,
        kind: if node.metadata.kind == "corpus" {
            "document"
        } else {
            "code"
        }
        .into(),
        group: node.metadata.doc_group,
        stand: node.metadata.stand,
        evidence_links,
    }
}

fn evidence_counts(raw: &RawGraph, seeds: &HashSet<String>) -> HashMap<String, usize> {
    let existing: HashSet<_> = raw.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut counts = HashMap::new();
    for link in &raw.links {
        if link.relation == "documents"
            && seeds.contains(&link.source)
            && existing.contains(link.target.as_str())
        {
            *counts.entry(link.source.clone()).or_default() += 1;
        }
    }
    counts
}

fn reduce_graph(raw: RawGraph, modified: SystemTime) -> Result<Graph, &'static str> {
    let selection = select_nodes(&raw)?;
    let evidence = evidence_counts(&raw, &selection.seeds);
    let source_nodes = raw.nodes.len();
    let source_links = raw.links.len();
    let mut links: Vec<_> = raw
        .links
        .into_iter()
        .filter(|link| {
            selection.selected.contains(&link.source) && selection.selected.contains(&link.target)
        })
        .collect();
    links.sort_by(|a, b| {
        (a.relation != "documents", &a.source, &a.target, &a.relation).cmp(&(
            b.relation != "documents",
            &b.source,
            &b.target,
            &b.relation,
        ))
    });
    let omitted_links = links.len().saturating_sub(MAX_LINKS);
    links.truncate(MAX_LINKS);
    let mut nodes: Vec<_> = raw
        .nodes
        .into_iter()
        .filter(|node| selection.selected.contains(&node.id))
        .map(|node| {
            let count = evidence.get(&node.id).copied().unwrap_or(0);
            node_view(node, count)
        })
        .collect();
    nodes.sort_by(|a, b| (&a.kind, &a.label, &a.id).cmp(&(&b.kind, &b.label, &b.id)));
    Ok(Graph {
        nodes,
        links,
        source_nodes,
        source_links,
        corpus_nodes: selection.seeds.len(),
        unlinked_documents: selection.seeds.len() - evidence.len(),
        omitted_nodes: selection.omitted,
        omitted_links,
        generated_at: chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wissenskarte_behaelt_dokumente_und_nur_belegte_beziehungen() {
        let raw = serde_json::from_value(json!({
            "nodes": [
                {"id":"doc", "label":"Concierge", "metadata":{"kind":"corpus"}},
                {"id":"unlinked", "metadata":{"kind":"corpus"}},
                {"id":"code", "metadata":{"file_url":"file:///private"}},
                {"id":"neighbor"}, {"id":"unrelated"}
            ],
            "links": [
                {"source":"doc", "target":"code", "relation":"documents"},
                {"source":"code", "target":"neighbor", "relation":"calls"},
                {"source":"neighbor", "target":"unrelated", "relation":"calls"}
            ]
        }))
        .unwrap();
        let graph = reduce_graph(raw, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.links.len(), 2);
        assert_eq!(graph.unlinked_documents, 1);
        let encoded = serde_json::to_string(&graph).unwrap();
        assert!(!encoded.contains("file://"));
        assert!(!encoded.contains("unrelated"));
    }

    #[test]
    fn leeres_korpus_ist_ein_sichtbarer_fehler() {
        let raw = RawGraph {
            nodes: vec![],
            links: vec![],
        };
        assert!(reduce_graph(raw, SystemTime::UNIX_EPOCH).is_err());
    }

    #[test]
    fn grosse_nachbarschaft_bleibt_begrenzt_und_zaehlt_auslassungen_einmal() {
        let mut nodes = vec![serde_json::json!({"id":"doc", "metadata":{"kind":"corpus"}})];
        let mut links = Vec::new();
        for index in 0..1600 {
            nodes.push(serde_json::json!({"id":format!("code-{index}")}));
            links.push(serde_json::json!({"source":"doc", "target":format!("code-{index}"), "relation":"documents"}));
        }
        let raw = serde_json::from_value(serde_json::json!({"nodes":nodes,"links":links})).unwrap();
        let graph = reduce_graph(raw, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(graph.nodes.len(), MAX_NODES);
        assert_eq!(graph.omitted_nodes, 101);
        assert_eq!(graph.unlinked_documents, 0);
        assert_eq!(
            graph
                .nodes
                .iter()
                .find(|node| node.id == "doc")
                .unwrap()
                .evidence_links,
            1600
        );
    }

    #[tokio::test]
    async fn cache_wird_beim_neuen_dateistand_erneuert() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("graph.json");
        std::fs::write(
            &path,
            r#"{"nodes":[{"id":"a","metadata":{"kind":"corpus"}}],"links":[]}"#,
        )
        .unwrap();
        let first = cached_graph(path.clone()).await.unwrap();
        let same = cached_graph(path.clone()).await.unwrap();
        assert_eq!(first.as_ptr(), same.as_ptr());
        std::fs::write(
            &path,
            r#"{"nodes":[{"id":"neuer-stand","metadata":{"kind":"corpus"}}],"links":[]}"#,
        )
        .unwrap();
        let updated = cached_graph(path).await.unwrap();
        assert_ne!(first, updated);
        assert!(String::from_utf8_lossy(&updated).contains("neuer-stand"));
    }
}
