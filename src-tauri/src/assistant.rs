use serde_json::{json, Value};

const MODEL: &str = "claude-sonnet-5";

/// Sends a spoken instruction plus optional on-screen context (e.g. selected
/// text) to Claude and returns the text to inject in place of the instruction.
pub async fn run(instruction: &str, context: &str) -> Result<String, String> {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").map_err(|_| "ANTHROPIC_API_KEY not set".to_string())?;

    let prompt = if context.trim().is_empty() {
        instruction.to_string()
    } else {
        format!(
            "Context (selected text from the screen):\n{context}\n\n\
             Instruction: {instruction}\n\n\
             Respond with ONLY the resulting text to insert. No preamble, no explanation, no markdown fences."
        )
    };

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": prompt}]
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {status}: {body}"));
    }

    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    body["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("unexpected response shape: {body}"))
}
