use super::Summarizer;
use anyhow::{Context, Result, bail};
use serde_json::json;
use std::time::Duration;

/// Talks to a local Ollama server (https://ollama.com), which runs small
/// models such as `qwen2.5:1.5b` or `llama3.2:3b` entirely on this machine.
pub struct Ollama {
    pub url: String,
    pub model: String,
}

const PROMPT: &str = "You are a precise technical summarizer. Summarize the document \
excerpt below in clear Markdown bullet points. Keep every concrete number, name, date, \
and decision. Do not invent anything that is not in the text. Output only the summary.\n\n\
--- EXCERPT ---\n";

impl Ollama {
    fn agent(timeout: Duration) -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .build()
            .into()
    }

    /// True when a server answers and has the requested model pulled.
    pub fn is_ready(&self) -> Result<bool> {
        let mut resp = Self::agent(Duration::from_secs(2))
            .get(&format!("{}/api/tags", self.url))
            .call()
            .context("Ollama server not reachable")?;
        let body: serde_json::Value = resp.body_mut().read_json()?;
        let found = body["models"].as_array().is_some_and(|ms| {
            ms.iter().any(|m| {
                let name = m["name"].as_str().unwrap_or("");
                name == self.model || name.strip_suffix(":latest") == Some(self.model.as_str())
            })
        });
        Ok(found)
    }
}

impl Summarizer for Ollama {
    fn name(&self) -> String {
        format!("ollama:{}", self.model)
    }

    fn summarize(&self, text: &str) -> Result<String> {
        let mut resp = Self::agent(Duration::from_secs(600))
            .post(&format!("{}/api/generate", self.url))
            .send_json(json!({
                "model": self.model,
                "prompt": format!("{PROMPT}{text}"),
                "stream": false,
                "options": { "temperature": 0.1 }
            }))
            .context("Ollama request failed")?;
        let body: serde_json::Value = resp.body_mut().read_json()?;
        match body["response"].as_str() {
            Some(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
            _ => bail!("Ollama returned an empty response"),
        }
    }
}
