//! Modello Marian (candle) e traduzione di una frase alla volta, in modo greedy.

use crate::translate::tok::Tok;
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::marian::{Config, MTModel};
use std::collections::HashMap;
use std::path::Path;

pub struct Translator {
    model: MTModel,
    cfg: Config,
    tok: Tok,
    device: Device,
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Legge `config.json` di Hugging Face completando i campi che a volte mancano.
fn read_config(dir: &Path) -> Result<Config, String> {
    let path = dir.join("config.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("config.json non valido: {e}"))?;
    let obj = v.as_object_mut().ok_or("config.json non valido")?;
    let eos = obj
        .get("eos_token_id")
        .cloned()
        .unwrap_or(serde_json::json!(0));
    for (k, d) in [
        ("forced_eos_token_id", eos),
        ("use_cache", serde_json::json!(true)),
        ("is_encoder_decoder", serde_json::json!(true)),
        ("scale_embedding", serde_json::json!(true)),
        ("share_encoder_decoder_embeddings", serde_json::json!(true)),
    ] {
        obj.entry(k).or_insert(d);
    }
    serde_json::from_value(v)
        .map_err(|e| format!("config.json non supportato (serve un modello Marian): {e}"))
}

/// Pesi del modello da `model.safetensors` o `pytorch_model.bin`.
fn read_weights(dir: &Path, device: &Device) -> Result<HashMap<String, Tensor>, String> {
    let st = dir.join("model.safetensors");
    let bin = dir.join("pytorch_model.bin");
    let list: Vec<(String, Tensor)> = if st.is_file() {
        let bytes = std::fs::read(&st).map_err(|e| format!("{}: {e}", st.display()))?;
        candle_core::safetensors::load_buffer(&bytes, device)
            .map_err(err)?
            .into_iter()
            .collect()
    } else if bin.is_file() {
        candle_core::pickle::read_all(&bin).map_err(err)?
    } else {
        return Err(format!(
            "nessun file dei pesi (model.safetensors o pytorch_model.bin) in {}",
            dir.display()
        ));
    };
    let mut map = HashMap::new();
    for (k, t) in list {
        let t = t
            .to_dtype(DType::F32)
            .map_err(err)?
            .to_device(device)
            .map_err(err)?;
        map.insert(k, t);
    }
    // Gli embedding sono condivisi: il file può averne salvato uno solo, con un altro nome.
    if !map.contains_key("model.shared.weight") {
        let alt = [
            "model.encoder.embed_tokens.weight",
            "model.decoder.embed_tokens.weight",
            "lm_head.weight",
        ]
        .iter()
        .find_map(|k| map.get(*k).cloned())
        .ok_or("nei pesi mancano gli embedding del modello")?;
        map.insert("model.shared.weight".into(), alt);
    }
    Ok(map)
}

impl Translator {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let cfg = read_config(dir)?;
        let device = Device::Cpu;
        let mut weights = read_weights(dir, &device)?;
        if !weights.contains_key("final_logits_bias") {
            let n = cfg.decoder_vocab_size.unwrap_or(cfg.vocab_size);
            weights.insert(
                "final_logits_bias".into(),
                Tensor::zeros((1, n), DType::F32, &device).map_err(err)?,
            );
        }
        let vb = VarBuilder::from_tensors(weights, DType::F32, &device);
        let model = MTModel::new(&cfg, vb)
            .map_err(|e| format!("pesi incompatibili con config.json: {e}"))?;
        let tok = Tok::load(
            dir,
            cfg.eos_token_id,
            cfg.pad_token_id,
            cfg.max_position_embeddings.min(512),
        )?;
        Ok(Self {
            model,
            cfg,
            tok,
            device,
        })
    }

    #[cfg(test)]
    pub fn from_parts(model: MTModel, cfg: Config, tok: Tok) -> Self {
        Self {
            model,
            cfg,
            tok,
            device: Device::Cpu,
        }
    }

    /// Traduce una frase (o un breve paragrafo).
    pub fn translate(&mut self, text: &str) -> Result<String, String> {
        let ids = self.tok.encode(text);
        if ids.len() <= 1 {
            return Ok(String::new());
        }
        self.model.reset_kv_cache();
        let input = Tensor::new(ids.as_slice(), &self.device)
            .map_err(err)?
            .unsqueeze(0)
            .map_err(err)?;
        let enc = self.model.encoder().forward(&input, 0).map_err(err)?;
        let mut out = vec![self.cfg.decoder_start_token_id];
        let limit =
            (ids.len() * 3 + 20).min(self.cfg.max_position_embeddings.saturating_sub(2).max(8));
        for step in 0..limit {
            let ctx = if step >= 1 { 1 } else { out.len() };
            let start = out.len().saturating_sub(ctx);
            let tail = out.get(start..).unwrap_or(&[]);
            let inp = Tensor::new(tail, &self.device)
                .map_err(err)?
                .unsqueeze(0)
                .map_err(err)?;
            let logits = self
                .model
                .decode(&inp, &enc, start)
                .map_err(err)?
                .squeeze(0)
                .map_err(err)?;
            let last = logits
                .get(logits.dim(0).map_err(err)?.saturating_sub(1))
                .map_err(err)?;
            let mut v: Vec<f32> = last
                .to_dtype(DType::F32)
                .map_err(err)?
                .to_vec1()
                .map_err(err)?;
            if let Some(p) = v.get_mut(self.cfg.pad_token_id as usize) {
                *p = f32::NEG_INFINITY;
            }
            ban_repeats(&mut v, &out[1..], 4);
            let next = argmax(&v);
            if next == self.cfg.eos_token_id || next == self.cfg.forced_eos_token_id {
                break;
            }
            out.push(next);
        }
        Ok(self.tok.decode(out.get(1..).unwrap_or(&[])))
    }
}

fn argmax(v: &[f32]) -> u32 {
    let mut best = 0usize;
    for (i, x) in v.iter().enumerate() {
        if *x > v.get(best).copied().unwrap_or(f32::NEG_INFINITY) {
            best = i;
        }
    }
    best as u32
}

/// Vieta i token che ripeterebbero un gruppo di `n` già generato (evita i cicli del greedy).
fn ban_repeats(logits: &mut [f32], generated: &[u32], n: usize) {
    if n < 2 || generated.len() + 1 < n {
        return;
    }
    let prefix = &generated[generated.len() + 1 - n..];
    for w in generated.windows(n) {
        if w[..n - 1] == *prefix {
            if let Some(l) = logits.get_mut(w[n - 1] as usize) {
                *l = f32::NEG_INFINITY;
            }
        }
    }
}

#[cfg(test)]
pub fn test_translator() -> Translator {
    let cfg = tiny_config();
    let vb = VarBuilder::zeros(DType::F32, &Device::Cpu);
    let model = MTModel::new(&cfg, vb).unwrap();
    Translator::from_parts(model, cfg, crate::translate::tok::test_tok())
}

#[cfg(test)]
fn tiny_config() -> Config {
    Config {
        vocab_size: 8,
        decoder_vocab_size: Some(8),
        max_position_embeddings: 32,
        encoder_layers: 1,
        encoder_ffn_dim: 16,
        encoder_attention_heads: 2,
        decoder_layers: 1,
        decoder_ffn_dim: 16,
        decoder_attention_heads: 2,
        use_cache: true,
        is_encoder_decoder: true,
        activation_function: candle_nn::Activation::Swish,
        d_model: 16,
        decoder_start_token_id: 2,
        scale_embedding: true,
        pad_token_id: 2,
        eos_token_id: 0,
        forced_eos_token_id: 0,
        share_encoder_decoder_embeddings: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argmax_and_repeat_ban() {
        assert_eq!(argmax(&[0.1, 0.9, 0.3]), 1);
        assert_eq!(argmax(&[]), 0);
        let mut l = vec![0.0; 6];
        // generato: 1 2 3 1 2 -> dopo «1 2» il 3 è già stato visto (gruppo 1 2 3)
        ban_repeats(&mut l, &[1, 2, 3, 1, 2], 3);
        assert_eq!(l[3], f32::NEG_INFINITY);
        assert_eq!(l[1], 0.0);
        ban_repeats(&mut l, &[], 4);
        ban_repeats(&mut l, &[1], 4);
    }

    #[test]
    fn a_model_with_blank_weights_runs_to_the_end_without_panicking() {
        let mut t = test_translator();
        let out = t.translate("hello world").unwrap();
        assert!(out.chars().count() < 200);
        assert_eq!(t.translate("   ").unwrap(), "");
        // seconda frase: la cache va azzerata, nessun errore di dimensioni
        assert!(t.translate("hello").is_ok());
    }

    #[test]
    fn missing_model_files_give_readable_errors() {
        let dir = std::env::temp_dir().join(format!("trad_nomodel_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let e = Translator::load(&dir).err().unwrap();
        assert!(e.contains("config.json"), "{e}");
        std::fs::write(dir.join("config.json"), "{}").unwrap();
        assert!(Translator::load(&dir).err().unwrap().contains("Marian"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
