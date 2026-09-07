use axum::{
    body::Body,
    extract::Json,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedToolCall {
    pub name: String,
    pub arguments: String,
}

/// Robustly extracts tool calls from local model outputs.
pub fn extract_tool_calls(content: &str) -> Option<Vec<ExtractedToolCall>> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return None;
    }

    let mut tool_calls = Vec::new();

    // 1. XML tags: <tool_call> ... </tool_call>
    if let Ok(xml_re) = Regex::new(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>") {
        for cap in xml_re.captures_iter(content) {
            if let Some(inner) = cap.get(1) {
                let inner_str = inner.as_str().trim();
                if let Ok(val) = serde_json::from_str::<Value>(inner_str) {
                    if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                        let args = val
                            .get("arguments")
                            .or_else(|| val.get("parameters"))
                            .cloned()
                            .unwrap_or(json!({}));
                        let args_str = if args.is_string() {
                            args.as_str().unwrap().to_string()
                        } else {
                            serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
                        };
                        tool_calls.push(ExtractedToolCall {
                            name: name.to_string(),
                            arguments: args_str,
                        });
                    }
                }
            }
        }
    }
    if !tool_calls.is_empty() {
        return Some(tool_calls);
    }

    // 2. Markdown code blocks: ```(?:json|text|xml)? ... ```
    if let Ok(code_block_re) = Regex::new(r"(?s)```(?:json|text|xml)?\s*(\{.*?\})\s*```") {
        for cap in code_block_re.captures_iter(content) {
            if let Some(inner) = cap.get(1) {
                let inner_str = inner.as_str().trim();
                if let Ok(val) = serde_json::from_str::<Value>(inner_str) {
                    if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                        if val.get("arguments").is_some() || val.get("parameters").is_some() {
                            let args = val
                                .get("arguments")
                                .or_else(|| val.get("parameters"))
                                .cloned()
                                .unwrap_or(json!({}));
                            let args_str = if args.is_string() {
                                args.as_str().unwrap().to_string()
                            } else {
                                serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
                            };
                            tool_calls.push(ExtractedToolCall {
                                name: name.to_string(),
                                arguments: args_str,
                            });
                        }
                    }
                }
            }
        }
    }
    if !tool_calls.is_empty() {
        return Some(tool_calls);
    }

    // 3. Custom XML tags: <function=name><parameter=key>value</parameter></function>
    if let Ok(fn_re) = Regex::new(r"(?s)<function=([a-zA-Z0-9_\.-]+)>\s*(.*?)\s*</function>") {
        for cap in fn_re.captures_iter(content) {
            let fn_name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
            let body = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            if let Ok(param_re) = Regex::new(r"(?s)<parameter=([a-zA-Z0-9_\.-]+)>\s*(.*?)\s*</parameter>") {
                let mut params_map = serde_json::Map::new();
                for pcap in param_re.captures_iter(body) {
                    if let (Some(k), Some(v)) = (pcap.get(1), pcap.get(2)) {
                        params_map.insert(k.as_str().to_string(), json!(v.as_str().trim()));
                    }
                }
                let args_str = serde_json::to_string(&params_map).unwrap_or_else(|_| "{}".to_string());
                if !fn_name.is_empty() {
                    tool_calls.push(ExtractedToolCall {
                        name: fn_name,
                        arguments: args_str,
                    });
                }
            }
        }
    }
    if !tool_calls.is_empty() {
        return Some(tool_calls);
    }

    // 4. Bare JSON object: {\"name\": \"...\", \"arguments\": { ... }}
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        if let Ok(val) = serde_json::from_str::<Value>(trimmed) {
            if let Some(name) = val.get("name").and_then(|n| n.as_str()) {
                if val.get("arguments").is_some() || val.get("parameters").is_some() {
                    let args = val
                        .get("arguments")
                        .or_else(|| val.get("parameters"))
                        .cloned()
                        .unwrap_or(json!({}));
                    let args_str = if args.is_string() {
                        args.as_str().unwrap().to_string()
                    } else {
                        serde_json::to_string(&args).unwrap_or_else(|_| "{}".to_string())
                    };
                    return Some(vec![ExtractedToolCall {
                        name: name.to_string(),
                        arguments: args_str,
                    }]);
                }
            }
        }
    }

    None
}

/// Axum route handler: POST /v1/chat/completions and /api/local-llm/v1/chat/completions
pub async fn handle_chat_completions(
    Json(mut payload): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    let is_stream = payload
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    // Always fetch non-streaming from upstream llama-server so we can normalize tool calls reliably
    payload["stream"] = json!(false);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let upstream_url = format!(
        "http://127.0.0.1:{}/v1/chat/completions",
        crate::sandbox::llama_server::LLAMA_PORT
    );

    let resp = client
        .post(&upstream_url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Failed to reach llama-server: {}", e),
            )
        })?;

    if !resp.status().is_success() {
        let status = StatusCode::from_u16(resp.status().as_u16())
            .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let text = resp.text().await.unwrap_or_default();
        return Err((status, text));
    }

    let mut llama_resp = resp.json::<Value>().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid JSON from llama-server: {}", e),
        )
    })?;

    let choice = llama_resp.get_mut("choices").and_then(|c| c.get_mut(0));
    let mut extracted_tc = None;

    if let Some(c) = choice {
        if let Some(msg) = c.get_mut("message") {
            let has_existing_tc = msg
                .get("tool_calls")
                .and_then(|tc| tc.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);

            if !has_existing_tc {
                let content = msg.get("content").and_then(|s| s.as_str()).unwrap_or("");
                if let Some(tools) = extract_tool_calls(content) {
                    info!(
                        "Normalized {} tool call(s) from local model output: {:?}",
                        tools.len(),
                        tools.iter().map(|t| &t.name).collect::<Vec<_>>()
                    );
                    extracted_tc = Some(tools);
                }
            }
        }
    }

    let now_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let resp_id = llama_resp
        .get("id")
        .and_then(|s| s.as_str())
        .unwrap_or("chatcmpl-local")
        .to_string();
    let model_name = llama_resp
        .get("model")
        .and_then(|s| s.as_str())
        .unwrap_or("qwen2.5-coder-7b")
        .to_string();

    if !is_stream {
        if let Some(tools) = extracted_tc {
            if let Some(c) = llama_resp.get_mut("choices").and_then(|c| c.get_mut(0)) {
                c["finish_reason"] = json!("tool_calls");
                if let Some(msg) = c.get_mut("message") {
                    let tool_calls_json: Vec<Value> = tools
                        .into_iter()
                        .enumerate()
                        .map(|(idx, tc)| {
                            json!({
                                "id": format!("call_{}_{}", now_ts, idx),
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments,
                                }
                            })
                        })
                        .collect();
                    msg["tool_calls"] = json!(tool_calls_json);
                    msg["content"] = Value::Null;
                }
            }
        }
        return Ok((
            [(header::CONTENT_TYPE, "application/json")],
            Json(llama_resp),
        )
            .into_response());
    }

    // Stream SSE response formatting
    let mut sse_data = String::new();

    if let Some(tools) = extracted_tc {
        for (idx, tc) in tools.iter().enumerate() {
            let call_id = format!("call_{}_{}", now_ts, idx);
            // Chunk 1: Header with tool call metadata
            let chunk1 = json!({
                "id": resp_id,
                "object": "chat.completion.chunk",
                "created": now_ts,
                "model": model_name,
                "choices": [{
                    "index": 0,
                    "delta": {
                        "role": "assistant",
                        "content": Value::Null,
                        "tool_calls": [{
                            "index": idx,
                            "id": call_id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": "",
                            }
                        }]
                    },
                    "finish_reason": Value::Null
                }]
            });
            sse_data.push_str(&format!("data: {}\n\n", chunk1));

            // Chunk 2: Arguments payload
            let chunk2 = json!({
                "id": resp_id,
                "object": "chat.completion.chunk",
                "created": now_ts,
                "model": model_name,
                "choices": [{
                    "index": 0,
                    "delta": {
                        "tool_calls": [{
                            "index": idx,
                            "function": {
                                "arguments": tc.arguments,
                            }
                        }]
                    },
                    "finish_reason": Value::Null
                }]
            });
            sse_data.push_str(&format!("data: {}\n\n", chunk2));
        }

        // Chunk 3: finish_reason tool_calls
        let chunk3 = json!({
            "id": resp_id,
            "object": "chat.completion.chunk",
            "created": now_ts,
            "model": model_name,
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "tool_calls"
            }]
        });
        sse_data.push_str(&format!("data: {}\n\n", chunk3));
    } else {
        // Pass-through existing tool_calls or plain text content
        let choice_obj = llama_resp.get("choices").and_then(|c| c.get(0));
        let msg_obj = choice_obj.and_then(|c| c.get("message"));
        let existing_tool_calls = msg_obj.and_then(|m| m.get("tool_calls"));
        let content = msg_obj.and_then(|m| m.get("content")).and_then(|s| s.as_str());

        if let Some(tcs) = existing_tool_calls.and_then(|t| t.as_array()) {
            for (idx, tc) in tcs.iter().enumerate() {
                let call_id = tc.get("id").and_then(|s| s.as_str()).unwrap_or("call_0");
                let fn_obj = tc.get("function");
                let name = fn_obj.and_then(|f| f.get("name")).and_then(|s| s.as_str()).unwrap_or("");
                let args = fn_obj.and_then(|f| f.get("arguments")).and_then(|s| s.as_str()).unwrap_or("{}");

                let chunk1 = json!({
                    "id": resp_id,
                    "object": "chat.completion.chunk",
                    "created": now_ts,
                    "model": model_name,
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "role": "assistant",
                            "content": Value::Null,
                            "tool_calls": [{
                                "index": idx,
                                "id": call_id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": args,
                                }
                            }]
                        },
                        "finish_reason": Value::Null
                    }]
                });
                sse_data.push_str(&format!("data: {}\n\n", chunk1));
            }
            let chunk_fin = json!({
                "id": resp_id,
                "object": "chat.completion.chunk",
                "created": now_ts,
                "model": model_name,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "tool_calls"
                }]
            });
            sse_data.push_str(&format!("data: {}\n\n", chunk_fin));
        } else {
            let chunk1 = json!({
                "id": resp_id,
                "object": "chat.completion.chunk",
                "created": now_ts,
                "model": model_name,
                "choices": [{
                    "index": 0,
                    "delta": {
                        "role": "assistant",
                        "content": content.unwrap_or("")
                    },
                    "finish_reason": Value::Null
                }]
            });
            sse_data.push_str(&format!("data: {}\n\n", chunk1));

            let chunk2 = json!({
                "id": resp_id,
                "object": "chat.completion.chunk",
                "created": now_ts,
                "model": model_name,
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": "stop"
                }]
            });
            sse_data.push_str(&format!("data: {}\n\n", chunk2));
        }
    }

    sse_data.push_str("data: [DONE]\n\n");

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "text/event-stream".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    headers.insert(header::CONNECTION, "keep-alive".parse().unwrap());

    Ok((headers, Body::from(sse_data)).into_response())
}

/// Axum route handler: GET /v1/models and /api/local-llm/v1/models
pub async fn handle_models() -> Response {
    let client = reqwest::Client::new();
    let upstream_url = format!(
        "http://127.0.0.1:{}/v1/models",
        crate::sandbox::llama_server::LLAMA_PORT
    );

    if let Ok(resp) = client.get(&upstream_url).send().await {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<Value>().await {
                return ([(header::CONTENT_TYPE, "application/json")], Json(json))
                    .into_response();
            }
        }
    }

    let fallback = json!({
        "object": "list",
        "data": [
            {"id": "qwen2.5-coder-7b", "object": "model", "owned_by": "subdollar"},
            {"id": "qwen2.5-coder-1.5b", "object": "model", "owned_by": "subdollar"},
            {"id": "llama-3.2-3b", "object": "model", "owned_by": "subdollar"}
        ]
    });
    ([(header::CONTENT_TYPE, "application/json")], Json(fallback)).into_response()
}

/// Axum route handler: GET /v1/health or /api/local-llm/health
pub async fn handle_health() -> Response {
    ([(header::CONTENT_TYPE, "application/json")], Json(json!({"status": "ok"}))).into_response()
}


pub async fn ensure_normalizer_listener() {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(300))
        .build()
        .unwrap_or_default();
    if let Ok(resp) = client.get("http://127.0.0.1:3000/api/local-llm/health").send().await {
        if resp.status().is_success() {
            return;
        }
    }

    tokio::spawn(async move {
        let app = axum::Router::new()
            .route("/v1/models", axum::routing::get(handle_models))
            .route("/v1/chat/completions", axum::routing::post(handle_chat_completions))
            .route("/api/local-llm/v1/models", axum::routing::get(handle_models))
            .route("/api/local-llm/v1/chat/completions", axum::routing::post(handle_chat_completions))
            .route("/api/local-llm/health", axum::routing::get(handle_health));

        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], 3000));
        if let Ok(listener) = tokio::net::TcpListener::bind(addr).await {
            info!("Spawned standalone tool normalizer listener on 0.0.0.0:3000");
            let _ = axum::serve(listener, app).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_qwen_markdown_json() {
        let content = r#"```json
{
  "name": "write",
  "arguments": {
    "i": "Create Dockerfile",
    "path": "Dockerfile",
    "content": "FROM python:3.9-slim\nCMD [\"python\", \"app.py\"]"
  }
}
```"#;
        let extracted = extract_tool_calls(content).expect("Should extract tool call");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "write");
        let parsed_args: Value = serde_json::from_str(&extracted[0].arguments).unwrap();
        assert_eq!(parsed_args["path"], "Dockerfile");
    }

    #[test]
    fn test_extract_markdown_text() {
        let content = r#"```text
{"name":"write","arguments":{"i":"Writing hello.txt with SUCCESS","path":"/workspace/hello.txt","content":"SUCCESS"}}
```"#;
        let extracted = extract_tool_calls(content).expect("Should extract tool call");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "write");
    }

    #[test]
    fn test_extract_xml_tool_call() {
        let content = r#"<tool_call>
{"name": "bash", "arguments": {"command": "ls -la /workspace"}}
</tool_call>"#;
        let extracted = extract_tool_calls(content).expect("Should extract tool call");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "bash");
        let parsed_args: Value = serde_json::from_str(&extracted[0].arguments).unwrap();
        assert_eq!(parsed_args["command"], "ls -la /workspace");
    }

    #[test]
    fn test_extract_function_xml() {
        let content = r#"<function=write>
<parameter=path>
start.sh
</parameter>
<parameter=content>
echo hello
</parameter>
</function>"#;
        let extracted = extract_tool_calls(content).expect("Should extract function xml");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "write");
        let parsed: Value = serde_json::from_str(&extracted[0].arguments).unwrap();
        assert_eq!(parsed["path"], "start.sh");
        assert_eq!(parsed["content"], "echo hello");
    }

    #[test]
    fn test_extract_bare_json() {
        let content = r#"{"name": "write", "arguments": {"path": "test.txt", "content": "hi"}}"#;
        let extracted = extract_tool_calls(content).expect("Should extract bare json");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].name, "write");
    }

    #[test]
    fn test_extract_ignores_normal_conversation() {
        let content = "I have finished implementing the HTTP server. It listens on port 8080.";
        assert!(extract_tool_calls(content).is_none());
    }
}
