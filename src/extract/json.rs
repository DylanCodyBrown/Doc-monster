use anyhow::{Context, Result};

pub fn convert(bytes: &[u8], fallback: &str) -> Result<(String, String)> {
    let value: serde_json::Value = serde_json::from_slice(bytes).context("invalid JSON")?;
    let pretty = serde_json::to_string_pretty(&value)?;
    Ok((fallback.to_string(), format!("```json\n{pretty}\n```\n")))
}
