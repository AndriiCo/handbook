//! Staging-only wire proof for the Responses client.
//!
//! Two tiny calls with the staging key (run on plant only, where the
//! key lives):
//!
//! ```sh
//! set -a; . ~/.config/apostille/meta-staging.env; set +a
//! cargo test -p atlas --test live_responses -- --ignored --nocapture
//! ```
//!
//! 1. A structured-output call pins the flat `text.format` shape: a 200
//!    whose output parses as the requested schema proves the server
//!    accepted it.
//! 2. A 16-token call forces `status: "incomplete"`, proving truncation
//!    surfaces as an error instead of silent partial data.

use atlas::model::{
    BASE_URL, InputBlock, JsonSchema, MetaClient, ModelPort, ResponsesRequest, UreqTransport,
};

fn client() -> MetaClient<UreqTransport> {
    let key = std::env::var("META_API_KEY").expect("META_API_KEY must be set (staging key)");
    MetaClient::new(UreqTransport::default(), BASE_URL, key)
}

fn request(
    schema: Option<JsonSchema>,
    max_output_tokens: Option<u32>,
    text: &str,
) -> ResponsesRequest {
    ResponsesRequest {
        model: "muse-spark-1.3".to_string(),
        instructions: "Reply with exactly what is asked, nothing else.".to_string(),
        input: vec![InputBlock::Text(text.to_string())],
        schema,
        max_output_tokens,
        reasoning_effort: None,
        background: false,
        previous_response_id: None,
    }
}

#[test]
#[ignore = "needs the staging META_API_KEY on plant"]
fn live_flat_text_format_round_trip() {
    let schema = JsonSchema {
        name: "ok".to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {"ok": {"type": "boolean"}},
            "required": ["ok"],
            "additionalProperties": false,
        }),
        strict: true,
    };
    let created = client()
        .create_response(&request(
            Some(schema),
            Some(64),
            "Reply with {\"ok\": true}.",
        ))
        .expect("live structured-output call");
    let text = created.completion.expect("completed with text").text;
    println!("live structured output: {text}");
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("output is JSON");
    assert_eq!(
        parsed.get("ok").and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

#[test]
#[ignore = "needs the staging META_API_KEY on plant"]
fn live_truncation_reports_incomplete() {
    let err = format!(
        "{}",
        client()
            .create_response(&request(
                None,
                Some(16),
                "Write a 300-word essay about lighthouses, in full detail.",
            ))
            .unwrap_err()
    );
    println!("live truncation error: {err}");
    assert!(err.contains("incomplete"), "{err}");
}
