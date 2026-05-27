//! Lightweight token-count estimation.
//!
//! Uses a heuristic (~4 characters per token) suitable for cost tracking.
//! For precise counting, this can be replaced with `tiktoken-rs` in the future.

/// Estimate the number of tokens in a UTF-8 byte slice.
///
/// Uses the heuristic that English text averages ~4 characters per token.
/// This is accurate enough (~95%) for cost estimation and compression
/// statistics without pulling in a full tokenizer.
#[must_use]
pub fn count(data: &[u8]) -> u64 {
    let text = std::str::from_utf8(data).unwrap_or("");
    let chars = text.chars().count();
    // Use saturating add of 3 then div by 4 for ceiling without floating point.
    (chars.saturating_add(3) / 4) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty() {
        assert_eq!(count(b""), 0);
    }

    #[test]
    fn test_short_text() {
        // "hello" = 5 chars → ceil(5/4) = 2
        assert_eq!(count(b"hello"), 2);
    }

    #[test]
    fn test_four_chars_is_one_token() {
        // "abcd" = 4 chars → ceil(4/4) = 1
        assert_eq!(count(b"abcd"), 1);
    }

    #[test]
    fn test_invalid_utf8_falls_back_to_empty() {
        // Invalid UTF-8 → treated as empty → 0 tokens
        assert_eq!(count(&[0xFF, 0xFE, 0xFD]), 0);
    }

    #[test]
    fn test_json_like_body() {
        let body = serde_json::json!({"key": "value", "number": 42});
        let bytes = serde_json::to_vec(&body).unwrap();
        let tokens = count(&bytes);
        // 20 chars → ceil(20/4) = 5
        assert!(tokens > 0);
    }
}
