use std::path::Path;

use anyhow::{bail, Result};

use super::Embedder;

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    #[default]
    MiniLm,
    MultilingualE5Small,
}

#[cfg(any(feature = "dense-fastembed", feature = "dense-candle"))]
impl Profile {
    fn max_tokens(self) -> usize {
        match self {
            Self::MiniLm => super::MAX_TOKENS,
            Self::MultilingualE5Small => 512,
        }
    }

    fn contract(self, backend: &str) -> String {
        match self {
            Self::MiniLm => format!("all-MiniLM-L6-v2:{backend}:256:masked-mean:l2:v1"),
            Self::MultilingualE5Small => format!("multilingual-e5-small:{backend}:512:masked-mean:l2:query-prefix=query: :passage-prefix=passage: :v1"),
        }
    }

    fn texts(self, texts: &[String], query: bool) -> Vec<String> {
        match self {
            Self::MiniLm => texts.to_vec(),
            Self::MultilingualE5Small => texts
                .iter()
                .map(|text| format!("{}: {text}", if query { "query" } else { "passage" }))
                .collect(),
        }
    }
}

pub fn load_profile(backend: &str, root: &Path, profile: Profile) -> Result<Box<dyn Embedder>> {
    match profile {
        Profile::MiniLm => load(backend, root),
        #[cfg(feature = "dense-fastembed")]
        Profile::MultilingualE5Small if backend == "fastembed" => {
            Ok(Box::new(FastEmbedder::load_profile(root, profile)?))
        }
        Profile::MultilingualE5Small => {
            bail!("Mehrsprachiges E5 benötigt den vorhandenen FastEmbed-Laufweg")
        }
    }
}

#[cfg(all(test, feature = "dense-fastembed"))]
mod profile_tests {
    use super::*;

    #[test]
    fn e5_contract_separates_query_and_passage_without_changing_legacy() {
        let texts = vec!["Wie finde ich Mitspieler?".to_string()];
        assert_eq!(Profile::MiniLm.texts(&texts, false), texts);
        assert_eq!(Profile::MiniLm.texts(&texts, true), texts);
        assert_eq!(
            Profile::MultilingualE5Small.texts(&texts, true),
            ["query: Wie finde ich Mitspieler?"]
        );
        assert_eq!(
            Profile::MultilingualE5Small.texts(&texts, false),
            ["passage: Wie finde ich Mitspieler?"]
        );
        assert_ne!(
            Profile::MiniLm.contract("fastembed-5.2.0"),
            Profile::MultilingualE5Small.contract("fastembed-5.2.0")
        );
        assert_eq!(
            Profile::MiniLm.contract("fastembed-5.2.0"),
            "all-MiniLM-L6-v2:fastembed-5.2.0:256:masked-mean:l2:v1"
        );
    }
}

pub fn load(backend: &str, root: &Path) -> Result<Box<dyn Embedder>> {
    match backend {
        #[cfg(feature = "dense-fastembed")]
        "fastembed" => Ok(Box::new(FastEmbedder::load(root)?)),
        #[cfg(feature = "dense-candle")]
        "candle" => Ok(Box::new(CandleEmbedder::load(root)?)),
        _ => {
            let _ = root;
            bail!("Embedding-Laufweg nicht eingebaut oder unbekannt: {backend}")
        }
    }
}

#[cfg(any(feature = "dense-fastembed", feature = "dense-candle"))]
struct ModelFiles {
    weights: Vec<u8>,
    config: Vec<u8>,
    tokenizer: Vec<u8>,
    tokenizer_config: Vec<u8>,
    special_tokens: Vec<u8>,
    fingerprint: String,
}

#[cfg(any(feature = "dense-fastembed", feature = "dense-candle"))]
impl ModelFiles {
    #[cfg(feature = "dense-candle")]
    fn load(root: &Path, backend: &str, weights_name: &str) -> Result<Self> {
        Self::load_profile(root, backend, weights_name, Profile::MiniLm)
    }

    fn load_profile(
        root: &Path,
        backend: &str,
        weights_name: &str,
        profile: Profile,
    ) -> Result<Self> {
        use anyhow::{ensure, Context};
        use sha2::{Digest, Sha256};

        let mut hash = Sha256::new();
        hash.update(profile.contract(backend));
        let mut read = |name: &str| -> Result<Vec<u8>> {
            let bytes = std::fs::read(root.join(name))
                .with_context(|| format!("Lokale Modelldatei fehlt oder ist unlesbar: {name}; Setup vor dem Dienststart ausführen"))?;
            ensure!(!bytes.is_empty(), "Leere Modelldatei: {name}");
            hash.update((name.len() as u64).to_le_bytes());
            hash.update(name.as_bytes());
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(&bytes);
            Ok(bytes)
        };
        let weights = read(weights_name)?;
        let config = read("config.json")?;
        let tokenizer = read("tokenizer.json")?;
        let tokenizer_config = read("tokenizer_config.json")?;
        let special_tokens = read("special_tokens_map.json")?;
        let parsed: serde_json::Value = serde_json::from_slice(&config)?;
        ensure!(
            parsed["hidden_size"].as_u64() == Some(384),
            "Modell ist nicht 384-dimensional"
        );
        ensure!(
            parsed["num_hidden_layers"].as_u64()
                == Some(match profile {
                    Profile::MiniLm => 6,
                    Profile::MultilingualE5Small => 12,
                }),
            "Transformer-Layer passen nicht zum expliziten Embeddingprofil"
        );
        Ok(Self {
            weights,
            config,
            tokenizer,
            tokenizer_config,
            special_tokens,
            fingerprint: format!("{:x}", hash.finalize()),
        })
    }
}

#[cfg(feature = "dense-fastembed")]
struct FastEmbedder {
    model: fastembed::TextEmbedding,
    fingerprint: String,
    profile: Profile,
}

#[cfg(feature = "dense-fastembed")]
impl FastEmbedder {
    fn load(root: &Path) -> Result<Self> {
        Self::load_profile(root, Profile::MiniLm)
    }

    fn load_profile(root: &Path, profile: Profile) -> Result<Self> {
        let files = ModelFiles::load_profile(root, "fastembed-5.2.0", "model.onnx", profile)?;
        let tokenizer = fastembed::TokenizerFiles {
            tokenizer_file: files.tokenizer,
            config_file: files.config,
            special_tokens_map_file: files.special_tokens,
            tokenizer_config_file: files.tokenizer_config,
        };
        let model = fastembed::UserDefinedEmbeddingModel::new(files.weights, tokenizer)
            .with_pooling(fastembed::Pooling::Mean);
        let options =
            fastembed::InitOptionsUserDefined::new().with_max_length(profile.max_tokens());
        Ok(Self {
            model: fastembed::TextEmbedding::try_new_from_user_defined(model, options)?,
            fingerprint: files.fingerprint,
            profile,
        })
    }
}

#[cfg(feature = "dense-fastembed")]
impl Embedder for FastEmbedder {
    fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let embeddings = self
            .model
            .embed(self.profile.texts(texts, false), Some(16))?;
        for embedding in &embeddings {
            super::validate_embedding(embedding)?;
        }
        Ok(embeddings)
    }

    fn embed_queries(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let embeddings = self
            .model
            .embed(self.profile.texts(texts, true), Some(16))?;
        for embedding in &embeddings {
            super::validate_embedding(embedding)?;
        }
        Ok(embeddings)
    }
}

#[cfg(feature = "dense-candle")]
struct CandleEmbedder {
    model: candle_transformers::models::bert::BertModel,
    tokenizer: tokenizers::Tokenizer,
    fingerprint: String,
}

#[cfg(feature = "dense-candle")]
impl CandleEmbedder {
    fn load(root: &Path) -> Result<Self> {
        use candle_core::{DType, Device};
        use candle_nn::VarBuilder;
        use candle_transformers::models::bert::{BertModel, Config};
        use tokenizers::{PaddingParams, PaddingStrategy, TruncationParams};

        let files = ModelFiles::load(root, "candle-0.9.1", "model.safetensors")?;
        let config: Config = serde_json::from_slice(&files.config)?;
        let mut tokenizer = tokenizers::Tokenizer::from_bytes(files.tokenizer)
            .map_err(|err| anyhow::anyhow!("Lokaler Tokenizer: {err}"))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: super::MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|err| anyhow::anyhow!("Tokenizer-Trunkierung: {err}"))?;
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            ..Default::default()
        }));
        let vb = VarBuilder::from_buffered_safetensors(files.weights, DType::F32, &Device::Cpu)?;
        let _ = (&files.tokenizer_config, &files.special_tokens);
        Ok(Self {
            model: BertModel::load(vb, &config)?,
            tokenizer,
            fingerprint: files.fingerprint,
        })
    }
}

#[cfg(feature = "dense-candle")]
impl Embedder for CandleEmbedder {
    fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    fn embed(&mut self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        use candle_core::{DType, Device, Tensor};

        let mut result = Vec::with_capacity(texts.len());
        for batch in texts.chunks(16) {
            let encoded = self
                .tokenizer
                .encode_batch(batch.to_vec(), true)
                .map_err(|err| anyhow::anyhow!("Lokale Tokenisierung: {err}"))?;
            let shape = (encoded.len(), encoded[0].len());
            let ids: Vec<u32> = encoded
                .iter()
                .flat_map(|item| item.get_ids().iter().copied())
                .collect();
            let types: Vec<u32> = encoded
                .iter()
                .flat_map(|item| item.get_type_ids().iter().copied())
                .collect();
            let masks: Vec<u32> = encoded
                .iter()
                .flat_map(|item| item.get_attention_mask().iter().copied())
                .collect();
            let ids = Tensor::from_vec(ids, shape, &Device::Cpu)?;
            let types = Tensor::from_vec(types, shape, &Device::Cpu)?;
            let masks = Tensor::from_vec(masks, shape, &Device::Cpu)?;
            let hidden = self.model.forward(&ids, &types, Some(&masks))?;
            let mask = masks.to_dtype(DType::F32)?.unsqueeze(2)?;
            let pooled = hidden
                .broadcast_mul(&mask)?
                .sum(1)?
                .broadcast_div(&mask.sum(1)?)?;
            for mut embedding in pooled.to_vec2::<f32>()? {
                super::validate_embedding(&embedding)?;
                let norm = embedding
                    .iter()
                    .map(|value| f64::from(*value).powi(2))
                    .sum::<f64>()
                    .sqrt();
                for value in &mut embedding {
                    *value = (f64::from(*value) / norm) as f32;
                }
                result.push(embedding);
            }
        }
        Ok(result)
    }
}
