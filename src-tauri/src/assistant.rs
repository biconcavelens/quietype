use serde_json::{json, Value};

const MODEL: &str = "claude-sonnet-5";

/// Sends a spoken instruction plus on-screen context (the user's current
/// selection) to Claude and returns the text to type in.
pub async fn run(instruction: &str, context: &str, api_key: &str) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("No API key. Add one in Settings to use assistant mode.".to_string());
    }

    let prompt = if context.trim().is_empty() {
        format!(
            "{instruction}\n\n\
             Respond with ONLY the resulting text, ready to be typed into whatever \
             the user is working in. No preamble, no explanation, no markdown fences."
        )
    } else {
        format!(
            "Selected text:\n{context}\n\n\
             Instruction: {instruction}\n\n\
             Respond with ONLY the resulting text, which will replace the selection. \
             No preamble, no explanation, no markdown fences."
        )
    };

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&json!({
            "model": MODEL,
            "max_tokens": 2048,
            "messages": [{"role": "user", "content": prompt}]
        }))
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        // Surface the API's own message rather than a wall of JSON.
        let detail = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
            .unwrap_or(body);
        return Err(format!("Claude API {status}: {detail}"));
    }

    let body: Value = resp.json().await.map_err(|e| e.to_string())?;
    body["content"][0]["text"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "Unexpected response from Claude.".to_string())
}
