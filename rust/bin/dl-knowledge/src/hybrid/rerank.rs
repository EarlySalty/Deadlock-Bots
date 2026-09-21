use std::path::Path;

use anyhow::Result;

use super::config::Config;

pub trait Reranker: Send {
    fn fingerprint(&self) -> &str;
    fn scores(&mut self, question: &str, documents: &[String]) -> Result<Vec<f32>>;
}

pub fn load(root: &Path, config: &Config) -> Result<Box<dyn Reranker>> {
    #[cfg(feature = "rerank-fastembed")]
    {
        Ok(Box::new(LocalReranker::load(root, config)?))
    }
    #[cfg(not(feature = "rerank-fastembed"))]
    {
        let _ = (root, config);
        anyhow::bail!("Lokaler Reranker ist nicht eingebaut: Feature rerank-fastembed fehlt")
    }
}

#[cfg(feature = "rerank-fastembed")]
struct LocalReranker {
    model: fastembed::TextRerank,
    fingerprint: String,
    batch_size: usize,
}

#[cfg(feature = "rerank-fastembed")]
impl LocalReranker {
    fn load(root: &Path, config: &Config) -> Result<Self> {
        use anyhow::{ensure, Context};
        use sha2::{Digest, Sha256};

        config.validate()?;
        let mut hash = Sha256::new();
        hash.update(format!(
            "cross-encoder:fastembed-5.2.0:logits:v1:{}:{}",
            config.rerank_max_tokens, config.rerank_batch_size
        ));
        let mut read = |name: &str| -> Result<Vec<u8>> {
            let bytes = std::fs::read(root.join(name)).with_context(|| {
                format!("Lokale Reranker-Datei fehlt: {name}; Setup vor dem Dienststart ausführen")
            })?;
            ensure!(!bytes.is_empty(), "Leere Reranker-Datei");
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
            Ok(bytes)
        };
        let weights = read("model.onnx")?;
        let config_file = read("config.json")?;
        let tokenizer_file = read("tokenizer.json")?;
        let special_tokens_map_file = read("special_tokens_map.json")?;
        let tokenizer_config_file = read("tokenizer_config.json")?;
        let model_config: serde_json::Value = serde_json::from_slice(&config_file)?;
        let labels = model_config["num_labels"].as_u64().or_else(|| {
            model_config["id2label"]
                .as_object()
                .map(|labels| labels.len() as u64)
        });
        ensure!(
            labels == Some(1),
            "Reranker benötigt genau einen Relevanz-Logit"
        );
        let files = fastembed::TokenizerFiles {
            config_file,
            tokenizer_file,
            special_tokens_map_file,
            tokenizer_config_file,
        };
        let model = fastembed::UserDefinedRerankingModel::new(weights, files);
        let mut options = fastembed::RerankInitOptionsUserDefined::default();
        options.max_length = config.rerank_max_tokens;
        Ok(Self {
            model: fastembed::TextRerank::try_new_from_user_defined(model, options)?,
            fingerprint: format!("{:x}", hash.finalize()),
            batch_size: config.rerank_batch_size,
        })
    }
}

#[cfg(feature = "rerank-fastembed")]
impl Reranker for LocalReranker {
    fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    fn scores(&mut self, question: &str, documents: &[String]) -> Result<Vec<f32>> {
        use anyhow::ensure;

        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let results = self.model.rerank(
            question,
            documents.iter().map(String::as_str).collect(),
            false,
            Some(self.batch_size),
        )?;
        ensure!(
            results.len() == documents.len(),
            "Reranker-Anzahl ist ungültig"
        );
        let mut scores = vec![None; documents.len()];
        for result in results {
            ensure!(
                result.index < scores.len() && result.score.is_finite(),
                "Ungültiges Reranker-Ergebnis"
            );
            ensure!(
                scores[result.index].replace(result.score).is_none(),
                "Doppelter Reranker-Index"
            );
        }
        scores
            .into_iter()
            .map(|score| score.ok_or_else(|| anyhow::anyhow!("Reranker-Score fehlt")))
            .collect()
    }
}
