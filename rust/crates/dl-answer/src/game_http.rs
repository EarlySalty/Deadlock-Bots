//! Spielbelege aus dem laufenden Brain-Dienst, ohne zweiten KI-Aufruf.
use std::{collections::HashSet, time::Duration};

use serde::Deserialize;
use zeroize::Zeroizing;

use crate::{AnswerError, Evidence, Retrieved, Retriever, Source};

pub struct HttpRetriever {
    base_url: String,
    token: Zeroizing<String>,
}

impl HttpRetriever {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            base_url,
            token: Zeroizing::new(token),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RetrievalWire {
    contract_version: String,
    request_id: String,
    knowledge_release: String,
    status: String,
    evidence: Vec<EvidenceWire>,
    truncated: bool,
    out_of_domain: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceWire {
    citation_id: String,
    label: String,
    text: String,
    kind: String,
}

#[async_trait::async_trait]
impl Retriever for HttpRetriever {
    async fn retrieve(&self, question: &str) -> Result<Retrieved, AnswerError> {
        let mut url = url::Url::parse(&self.base_url).map_err(|_| AnswerError::InvalidEvidence)?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
            None => false,
        };
        if !loopback
            || !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || self.token.trim().is_empty()
        {
            return Err(AnswerError::Retrieval);
        }
        url.set_path("/v1/retrieve");
        url.set_query(None);
        url.set_fragment(None);
        let request_id = format!(
            "bot-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| AnswerError::Retrieval)?;
        let mut response = client
            .post(url)
            .bearer_auth(self.token.as_str())
            .json(&serde_json::json!({
                "request_id": request_id,
                "conversation_id": request_id,
                "text": question,
                "requested_scopes": ["bot.public"],
                "profile": "explain",
            }))
            .send()
            .await
            .map_err(transport_error)?;
        if !response.status().is_success() {
            tracing::warn!(
                status = response.status().as_u16(),
                "Brain-Belegabruf lieferte einen HTTP-Fehler"
            );
            return Err(AnswerError::Retrieval);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(AnswerError::InvalidEvidence);
            }
            bytes.extend_from_slice(&chunk);
        }
        let wire = serde_json::from_slice(&bytes).map_err(|_| AnswerError::InvalidEvidence)?;
        validate_wire(wire, &request_id)
    }
}

fn transport_error(error: reqwest::Error) -> AnswerError {
    if error.is_timeout() {
        AnswerError::Timeout
    } else {
        AnswerError::Retrieval
    }
}

fn validate_wire(wire: RetrievalWire, request_id: &str) -> Result<Retrieved, AnswerError> {
    if wire.contract_version != "brain.public.v1"
        || wire.request_id != request_id
        || wire.knowledge_release.trim().is_empty()
        || wire.evidence.len() > 100
    {
        return Err(AnswerError::InvalidEvidence);
    }
    match wire.status.as_str() {
        "unavailable" => return Err(AnswerError::Retrieval),
        "unauthorized_evidence" => return Err(AnswerError::InvalidEvidence),
        "insufficient_evidence" if wire.evidence.is_empty() => {
            return Ok(Retrieved {
                out_of_domain: wire.out_of_domain,
                truncated: wire.truncated,
                evidence: vec![],
            })
        }
        "answered" if !wire.evidence.is_empty() && !wire.out_of_domain => {}
        _ => return Err(AnswerError::InvalidEvidence),
    }
    let mut ids = HashSet::new();
    let mut evidence = Vec::new();
    for item in wire.evidence {
        if item.citation_id.trim().is_empty()
            || item.citation_id.len() > 256
            || !ids.insert(item.citation_id.clone())
            || item.label.trim().is_empty()
            || item.label.chars().count() > 256
            || item.text.trim().is_empty()
            || !matches!(
                item.kind.as_str(),
                "fact" | "rule" | "prose" | "mechanic" | "population" | "replay"
            )
        {
            return Err(AnswerError::InvalidEvidence);
        }
        evidence.push(Evidence {
            id: format!("G{}", item.citation_id),
            source: Source::GameData { title: item.label },
            // Eine Veröffentlichung ist keine Creator-Verifizierung. Die ursprüngliche Art bleibt sichtbar.
            text: format!("Belegart: {}\n{}", item.kind, item.text),
            observed_at: None,
        });
    }
    Ok(Retrieved {
        evidence,
        out_of_domain: false,
        truncated: wire.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(status: &str, evidence: Vec<EvidenceWire>) -> RetrievalWire {
        RetrievalWire {
            contract_version: "brain.public.v1".into(),
            request_id: "probe".into(),
            knowledge_release: "release".into(),
            status: status.into(),
            evidence,
            truncated: false,
            out_of_domain: false,
        }
    }
    fn item(kind: &str) -> EvidenceWire {
        EvidenceWire {
            citation_id: "cite-123".into(),
            label: "Beleg 1".into(),
            text: "Veröffentlichter Inhalt".into(),
            kind: kind.into(),
        }
    }

    #[test]
    fn oeffentliche_prosa_wird_nicht_als_creator_verifiziert() {
        let result = validate_wire(wire("answered", vec![item("prose")]), "probe").unwrap();
        assert!(matches!(
            &result.evidence[0].source,
            Source::GameData { .. }
        ));
        assert!(result.evidence[0].text.starts_with("Belegart: prose\n"));
    }

    #[test]
    fn fehlende_belege_sind_keine_transportstoerung() {
        assert!(
            validate_wire(wire("insufficient_evidence", vec![]), "probe")
                .unwrap()
                .evidence
                .is_empty()
        );
        assert!(matches!(
            validate_wire(wire("unavailable", vec![]), "probe"),
            Err(AnswerError::Retrieval)
        ));
    }

    #[test]
    fn falsche_identitaeten_doppelte_und_private_belege_werden_abgewiesen() {
        assert!(validate_wire(wire("answered", vec![item("fact")]), "anderer-request").is_err());
        assert!(
            validate_wire(wire("answered", vec![item("fact"), item("fact")]), "probe").is_err()
        );
        assert!(validate_wire(wire("unauthorized_evidence", vec![item("fact")]), "probe").is_err());
        assert!(validate_wire(wire("answered", vec![item("unknown")]), "probe").is_err());
    }

    #[test]
    fn oeffentliche_vertragsgrenze_akzeptiert_100_und_verwirft_101_belege() {
        for count in [100, 101] {
            let evidence = (0..count)
                .map(|index| {
                    let mut value = item("fact");
                    value.citation_id = format!("cite-{index}");
                    value
                })
                .collect();
            let result = validate_wire(wire("answered", evidence), "probe");
            assert_eq!(result.is_ok(), count == 100);
            if let Ok(result) = result {
                assert_eq!(result.evidence.len(), 100);
            }
        }
    }

    #[tokio::test]
    async fn externe_ziele_und_leere_tokens_werden_vor_dem_request_abgewiesen() {
        for (url, token) in [
            ("https://example.com", "fixture"),
            ("http://127.0.0.1:8788", ""),
        ] {
            assert!(HttpRetriever::new(url.into(), token.into())
                .retrieve("Frage")
                .await
                .is_err());
        }
    }
}
