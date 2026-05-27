//! Shared test helpers for E2E tests.

#![allow(clippy::disallowed_methods)]
#![allow(clippy::panic)]

use std::path::PathBuf;

/// Path to the fixtures directory.
#[must_use]
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Load a fixture file as a string.
///
/// # Panics
///
/// Panics if the fixture file does not exist or cannot be read as UTF-8.
#[must_use]
pub fn load_fixture(name: &str) -> String {
    std::fs::read_to_string(fixtures_dir().join(name))
        .unwrap_or_else(|e| panic!("failed to load fixture {name}: {e}"))
}

/// Load a fixture and parse as JSON.
///
/// # Panics
///
/// Panics if the fixture file does not exist or contains invalid JSON.
#[must_use]
pub fn load_fixture_json(name: &str) -> serde_json::Value {
    let content = load_fixture(name);
    serde_json::from_str(&content).unwrap_or_else(|e| panic!("failed to parse fixture {name}: {e}"))
}

/// Build a basic Anthropic Messages API response.
#[must_use]
pub fn anthropic_response(
    id: &str,
    text: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": text}],
        "model": "claude-sonnet-4-6",
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": input_tokens,
            "output_tokens": output_tokens
        }
    })
}

/// Build a basic `OpenAI` Chat API response.
#[must_use]
pub fn openai_response(
    id: &str,
    text: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": text
            },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": input_tokens,
            "completion_tokens": output_tokens,
            "total_tokens": input_tokens + output_tokens
        }
    })
}
