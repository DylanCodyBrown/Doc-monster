//! The optional language model behind `--summarizer model`: Qwen2.5 1.5B Instruct (Alibaba
//! Cloud, Apache 2.0), compressed to about 4.5 bits per weight, run on this computer's
//! processor with candle.
//!
//! The files are downloaded once from Hugging Face into Doc Monster's cache folder (or read
//! from `--model-dir`), checked against known fingerprints, and reused offline afterwards.

use crate::transcript::analyze::Rewriter;
use anyhow::{Context, Result, anyhow, bail};
use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen2::ModelWeights;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tokenizers::Tokenizer;

/// Shown in summaries and help.
pub const NAME: &str = "Qwen2.5 1.5B Instruct";
/// About how much is downloaded, once.
pub const MEGABYTES: u32 = 1124;

/// (repository, exact version, file, SHA-256): always the same bytes.
const FILES: [(&str, &str, &str, &str); 2] = [
    (
        "Qwen/Qwen2.5-1.5B-Instruct-GGUF",
        "91cad51170dc346986eccefdc2dd33a9da36ead9",
        "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        "6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e",
    ),
    (
        "Qwen/Qwen2.5-1.5B-Instruct",
        "989aa7980e4cf806f80c7fef2b1adb7bc71aa306",
        "tokenizer.json",
        "c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539",
    ),
];

const SYSTEM: &str = "You summarize meeting transcripts accurately and briefly. Use only facts \
that are in the transcript, and never present something still unknown as a fact.";

pub struct Qwen {
    tokenizer: Tokenizer,
    end: u32,
    model: Mutex<ModelWeights>,
}

impl Qwen {
    /// Load the model, downloading it first if needed. `dir`: a folder that already holds the
    /// files (for computers without internet access).
    pub fn load(dir: Option<&Path>) -> Result<Qwen> {
        let dir = match dir {
            Some(d) => {
                for (repo, _, name, _) in FILES {
                    if !d.join(name).is_file() {
                        bail!(
                            "{name} is missing from {} (download it from https://huggingface.co/{repo})",
                            d.display()
                        );
                    }
                }
                d.to_path_buf()
            }
            None => fetch()?,
        };
        let weights = dir.join(FILES[0].2);
        let mut file = std::fs::File::open(&weights)
            .with_context(|| format!("opening {}", weights.display()))?;
        let content = gguf_file::Content::read(&mut file).context("reading the model")?;
        let model = ModelWeights::from_gguf(content, &mut file, &Device::Cpu)
            .context("loading the model")?;
        let tokenizer = Tokenizer::from_file(dir.join(FILES[1].2))
            .map_err(|e| anyhow!("reading the model's word list: {e}"))?;
        let end = tokenizer
            .token_to_id("<|im_end|>")
            .ok_or_else(|| anyhow!("the model's word list has no end marker"))?;
        Ok(Qwen {
            tokenizer,
            end,
            model: Mutex::new(model),
        })
    }
}

impl Rewriter for Qwen {
    fn complete(&self, prompt: &str) -> Result<String> {
        let chat = format!(
            "<|im_start|>system\n{SYSTEM}<|im_end|>\n<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n"
        );
        let ids = self
            .tokenizer
            .encode(chat, false)
            .map_err(|e| anyhow!("{e}"))?
            .get_ids()
            .to_vec();
        let device = Device::Cpu;
        let mut model = self
            .model
            .lock()
            .map_err(|_| anyhow!("the model failed earlier"))?;
        // Always the most likely next word: the same transcript gives the same summary.
        let mut pick = LogitsProcessor::from_sampling(0, Sampling::ArgMax);
        // Position 0 starts a fresh conversation (the model forgets the previous one).
        let input = Tensor::new(ids.as_slice(), &device)?.unsqueeze(0)?;
        let logits = model.forward(&input, 0)?.squeeze(0)?;
        let mut next = pick.sample(&logits)?;
        let mut out = vec![next];
        for i in 0..220 {
            if next == self.end {
                break;
            }
            let input = Tensor::new(&[next], &device)?.unsqueeze(0)?;
            let logits = model.forward(&input, ids.len() + i)?.squeeze(0)?;
            let recent = &out[out.len().saturating_sub(64)..];
            let logits = candle_transformers::utils::apply_repeat_penalty(&logits, 1.1, recent)?;
            next = pick.sample(&logits)?;
            out.push(next);
        }
        self.tokenizer
            .decode(&out, true)
            .map(|s| s.trim().to_string())
            .map_err(|e| anyhow!("{e}"))
    }

    fn name(&self) -> String {
        NAME.to_string()
    }

    fn max_words(&self) -> usize {
        // Reading is about 100 words a second on four cores; longer topics are summarised in
        // pieces first.
        1200
    }
}

/// The folder holding the model files, downloading any that are missing.
fn fetch() -> Result<PathBuf> {
    let mut last_err = anyhow!("no folder could be written");
    for base in crate::tools::cache_dirs() {
        let dir = base.join("models").join("qwen2.5-1.5b-instruct");
        if FILES.iter().all(|(_, _, name, _)| dir.join(name).is_file()) {
            return Ok(dir);
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            last_err = e.into();
            continue;
        }
        eprintln!(
            "Downloading the summarizing model ({NAME}, about {MEGABYTES} MB, once) into {}",
            dir.display()
        );
        for (repo, rev, name, sha) in FILES {
            let target = dir.join(name);
            if !target.is_file() {
                download(repo, rev, name, sha, &target)?;
            }
        }
        return Ok(dir);
    }
    Err(last_err).context("finding a folder for the model")
}

fn download(repo: &str, rev: &str, name: &str, sha: &str, target: &Path) -> Result<()> {
    let url = format!("https://huggingface.co/{repo}/resolve/{rev}/{name}");
    // The computer's own certificates, so company networks that inspect traffic still work.
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder().tls_config(tls).build().into();
    let response = agent
        .get(&url)
        .call()
        .with_context(|| format!("downloading {name} from huggingface.co"))?;
    let mut reader = response.into_body().into_reader();
    // Downloaded under a private name, checked, then renamed: never a half-written model.
    let temp = target.with_extension(format!("part{}", std::process::id()));
    let mut file = std::fs::File::create(&temp)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("downloading {name}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])?;
    }
    file.flush()?;
    drop(file);
    let got: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != sha {
        let _ = std::fs::remove_file(&temp);
        bail!("{name} did not download correctly (its fingerprint does not match); try again");
    }
    std::fs::rename(&temp, target)?;
    Ok(())
}
