//! Tokenless report file consumer.
//!
//! Reads the per-session JSONL report file produced by tokenless hooks
//! incrementally: each API request reads only the new lines since the last
//! consumption, keeping the file on disk so every request gets its fair
//! share of project/user context and incremental savings.
//!
//! # Report file format (JSONL)
//!
//! Each line is a JSON object with these fields:
//!
//! | Field | Type | Example | Description |
//! |---|---|---|---|
//! | `sessionId` | string | `"abc123"` | Claude Code session ID |
//! | `agentId` | string | `"claude"` | Agent identifier |
//! | `projectPath` | string\|null | `"my-project"` | Project path |
//! | `opType` | string | `"compress-schema"` | Operation type (see below) |
//! | `method` | string\|null | `"ToonHrv"` | Compression strategy (see below) |
//! | `beforeTokens` | u64 | `1500` | Estimated tokens before compression |
//! | `afterTokens` | u64 | `700` | Estimated tokens after compression |
//! | `savedTokens` | u64 | `800` | Tokens saved |
//! | `beforeBytes` | u64 | `6000` | Bytes before compression |
//! | `afterBytes` | u64 | `2800` | Bytes after compression |
//! | `savedBytes` | u64 | `3200` | Bytes saved |
//! | `timestamp` | string | RFC 3339 | When the hook ran |
//!
//! ## opType values
//!
//! | Value | Meaning |
//! |---|---|
//! | `compress-schema` | Tool schema definition compression (`BeforeModel` hook) |
//! | `compress-response` | Tool response output compression (`PostToolUse` hook) |
//! | `rewrite-command` | Shell command rewriting via RTK (`PreToolUse` hook) |
//! | `compress-toon` | TOON format encoding |
//!
//! ## method values (by opType)
//!
//! **compress-schema:**
//!
//! | Value | Meaning |
//! |---|---|
//! | `CompressorOnly` | Basic schema compressor (truncate descriptions, drop titles) |
//! | `ToonHrv` | Uniform object arrays → HRV encoding (>= 5 items) |
//! | `EnhancedToon` | Deep nesting or enum constraints → enhanced TOON |
//! | `CjsonCompact` | CJSON compact encoding (fallback) |
//!
//! **compress-response:**
//!
//! | Value | Meaning |
//! |---|---|
//! | `Standard` | Standard response compression (truncate strings/arrays, drop nulls) |
//! | `HighFidelity` | Bash output compression (wider truncation limits) |
//! | `Semantic` | Semantic-aware field filtering (`--semantic` flag) |
//!
//! **rewrite-command:**
//!
//! | Value | Meaning |
//! |---|---|
//! | `RtkStandard` | RTK command rewriting |
//!
//! **compress-toon:**
//!
//! | Value | Meaning |
//! |---|---|
//! | `ToonDefault` | Basic TOON encoding |

// File I/O via std::fs is intentional here: these are fast, small-file
// operations that don't benefit from async I/O.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::{fs, io::Write, sync::LazyLock};

use dashmap::DashMap;
use serde::Deserialize;

/// Tracks the last consumed byte offset per session for incremental
/// tokenless report reading. Kept in-memory; resets on proxy restart.
static REPORT_CURSORS: LazyLock<DashMap<String, u64>> = LazyLock::new(DashMap::new);

/// A single compression event reported by a tokenless hook.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProxyReport {
    #[allow(dead_code)]
    session_id: String,
    #[allow(dead_code)]
    agent_id: String,
    #[allow(dead_code)]
    project_path: Option<String>,
    #[serde(default)]
    user_name: Option<String>,
    op_type: String,
    #[allow(dead_code)]
    method: Option<String>,
    #[allow(dead_code)]
    before_tokens: u64,
    #[allow(dead_code)]
    after_tokens: u64,
    saved_tokens: u64,
    #[allow(dead_code)]
    #[serde(default)]
    before_bytes: u64,
    #[allow(dead_code)]
    #[serde(default)]
    after_bytes: u64,
    #[allow(dead_code)]
    #[serde(default)]
    saved_bytes: u64,
    #[allow(dead_code)]
    #[serde(default)]
    timestamp: String,
}

/// Accumulated tokenless stats for one session.
#[derive(Debug, Default)]
pub(crate) struct TokenlessAccumulator {
    /// Total tokens saved by all tokenless hook operations.
    pub total_saved: u64,
    /// Raw breakdown as a JSON array of objects.
    pub breakdown_json: String,
    /// Project path extracted from the first report line that has one.
    pub project_path: Option<String>,
    /// User name extracted from the first report line that has one.
    pub user_name: Option<String>,
}

/// Reads the tokenless report file for a session, returning only the
/// incremental savings since the last read.
///
/// Unlike the original rename-then-delete design, this reads the file in
/// place and tracks consumption via a byte-offset cursor. Every API request
/// in the same session gets its share of savings, and `project_path` /
/// `user_name` are always returned if present in the file.
///
/// The report file lives at:
/// `~/.tokenfleet-ai/tokenless/reports/{session_id}.jsonl`
///
/// Returns `None` if no report file exists, the session ID is empty, or
/// there are no new lines since the last consumption (and no metadata to
/// propagate).
pub(crate) fn consume_report(session_id: &str) -> Option<TokenlessAccumulator> {
    if session_id.is_empty() {
        return None;
    }

    // Sanitize session_id for use as filename
    let safe_sid: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(128)
        .collect();

    if safe_sid.is_empty() {
        return None;
    }

    let home = dirs::home_dir()?;
    let reports_dir = home
        .join(".tokenfleet-ai")
        .join("tokenless")
        .join("reports");
    let source = reports_dir.join(format!("{safe_sid}.jsonl"));

    // Read in place — no rename, no delete.
    let content = fs::read_to_string(&source).ok()?;
    if content.is_empty() {
        return None;
    }

    // Cursor tracks the byte offset of the last consumed byte.
    let mut cursor = REPORT_CURSORS.entry(safe_sid).or_insert(0u64);
    let start_byte = usize::try_from(*cursor).unwrap_or(usize::MAX);

    // If the file shrank (shouldn't normally happen), reset the cursor.
    let start_byte = if start_byte > content.len() {
        *cursor = 0;
        0
    } else {
        start_byte
    };

    let mut acc = TokenlessAccumulator::default();
    let mut breakdown_items: Vec<serde_json::Value> = Vec::new();
    let mut has_new_lines = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Determine whether this line is new using byte position.
        let line_start = line.as_ptr() as usize - content.as_ptr() as usize;
        let is_new = line_start >= start_byte;

        let Ok(report) = serde_json::from_str::<ProxyReport>(trimmed) else {
            continue;
        };

        // Always extract project/user from the first line that carries them.
        if acc.project_path.is_none() {
            acc.project_path.clone_from(&report.project_path);
        }
        if acc.user_name.is_none() {
            acc.user_name.clone_from(&report.user_name);
        }

        // Only accumulate savings from new lines.
        if is_new {
            acc.total_saved += report.saved_tokens;
            has_new_lines = true;

            breakdown_items.push(serde_json::json!({
                "op": report.op_type,
                "method": report.method,
                "beforeTokens": report.before_tokens,
                "afterTokens": report.after_tokens,
                "savedTokens": report.saved_tokens,
                "beforeBytes": report.before_bytes,
                "afterBytes": report.after_bytes,
                "savedBytes": report.saved_bytes,
            }));
        }
    }

    // Advance cursor past all content we just scanned, so the next read
    // only picks up newly appended lines.
    if has_new_lines {
        *cursor = content.len() as u64;
    }

    let has_new_data = has_new_lines;
    let has_meta = acc.project_path.is_some() || acc.user_name.is_some();

    if !has_new_data && !has_meta {
        return None;
    }

    tracing::info!(
        new_lines = breakdown_items.len(),
        total_new_saved = acc.total_saved,
        project_path = ?acc.project_path,
        "consumed tokenless report (incremental)"
    );

    // Debug log.
    let _ = write_consume_log(
        breakdown_items.len(),
        acc.total_saved,
        acc.project_path.as_deref(),
    );

    if !breakdown_items.is_empty() {
        acc.breakdown_json = serde_json::to_string(&breakdown_items).unwrap_or_default();
    }

    Some(acc)
}

/// Debug helper: write report consumption event to log file.
///
/// Log file: `~/.tokenfleet-ai/agent-proxy/report-consume.log` (JSON Lines).
fn write_consume_log(lines: usize, total_saved: u64, project_path: Option<&str>) -> Result<(), ()> {
    let home = dirs::home_dir().ok_or(())?;
    let log_dir = home.join(".tokenfleet-ai").join("agent-proxy");
    #[allow(clippy::disallowed_methods)]
    std::fs::create_dir_all(&log_dir).map_err(|_| ())?;
    let log_path = log_dir.join("report-consume.log");

    let entry = serde_json::json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "lines": lines,
        "total_saved": total_saved,
        "project_path": project_path,
    });

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|_| ())?;

    writeln!(f, "{}", serde_json::to_string(&entry).unwrap_or_default()).map_err(|_| ())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Helper: parse JSONL content incrementally with a cursor.
    fn parse(content: &str, cursor: &mut u64) -> Option<TokenlessAccumulator> {
        let start_byte = usize::try_from(*cursor).unwrap_or(usize::MAX);
        let start = if start_byte > content.len() {
            0
        } else {
            start_byte
        };

        let mut acc = TokenlessAccumulator::default();
        let mut items: Vec<serde_json::Value> = Vec::new();
        let mut has_new_lines = false;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let line_start = line.as_ptr() as usize - content.as_ptr() as usize;
            let is_new = line_start >= start;

            let Ok(report) = serde_json::from_str::<super::ProxyReport>(trimmed) else {
                continue;
            };
            if acc.project_path.is_none() {
                acc.project_path.clone_from(&report.project_path);
            }
            if acc.user_name.is_none() {
                acc.user_name.clone_from(&report.user_name);
            }
            if is_new {
                acc.total_saved += report.saved_tokens;
                has_new_lines = true;
                items.push(serde_json::json!({
                    "op": report.op_type,
                    "savedTokens": report.saved_tokens,
                }));
            }
        }
        if has_new_lines {
            *cursor = content.len() as u64;
        }
        if items.is_empty() && acc.project_path.is_none() && acc.user_name.is_none() {
            return None;
        }
        if !items.is_empty() {
            acc.breakdown_json = serde_json::to_string(&items).unwrap_or_default();
        }
        Some(acc)
    }

    #[test]
    fn test_incremental_empty_returns_none() {
        let mut cursor = 0u64;
        assert!(parse("", &mut cursor).is_none());
    }

    #[test]
    fn test_incremental_first_read() {
        let jsonl = r#"{"sessionId":"s","agentId":"a","projectPath":"test-proj","userName":"byx","opType":"rewrite-command","method":"Rtk","beforeTokens":100,"afterTokens":50,"savedTokens":50,"beforeBytes":400,"afterBytes":200,"savedBytes":200}"#;
        let mut cursor = 0u64;
        let acc = parse(jsonl, &mut cursor).unwrap();
        assert_eq!(acc.total_saved, 50);
        assert_eq!(acc.project_path.as_deref(), Some("test-proj"));
        assert_eq!(acc.user_name.as_deref(), Some("byx"));
        assert!(cursor > 0);
    }

    #[test]
    fn test_incremental_second_read_only_new_lines() {
        let first = r#"{"sessionId":"s","agentId":"a","projectPath":"p","userName":"u","opType":"rewrite-command","method":"Rtk","beforeTokens":100,"afterTokens":50,"savedTokens":50,"beforeBytes":400,"afterBytes":200,"savedBytes":200}"#;
        let mut cursor = 0u64;
        // First read consumes first line
        let acc1 = parse(first, &mut cursor).unwrap();
        assert_eq!(acc1.total_saved, 50);

        // Append a second line
        let second_line = r#"{"sessionId":"s","agentId":"a","projectPath":"p","userName":"u","opType":"rewrite-command","method":"Rtk","beforeTokens":200,"afterTokens":100,"savedTokens":100,"beforeBytes":800,"afterBytes":400,"savedBytes":400}"#;
        let full = format!("{first}\n{second_line}\n");
        let acc2 = parse(&full, &mut cursor).unwrap();
        // Only the new line's savings
        assert_eq!(acc2.total_saved, 100);
        // Metadata still available even from old lines
        assert_eq!(acc2.project_path.as_deref(), Some("p"));
        assert_eq!(acc2.user_name.as_deref(), Some("u"));
    }

    #[test]
    fn test_incremental_no_new_data_returns_meta() {
        let first = r#"{"sessionId":"s","agentId":"a","projectPath":"p","userName":"u","opType":"rewrite-command","method":"Rtk","beforeTokens":100,"afterTokens":50,"savedTokens":50,"beforeBytes":400,"afterBytes":200,"savedBytes":200}"#;
        let mut cursor = 0u64;
        parse(first, &mut cursor);

        // Second read with no new data — still returns meta
        let acc = parse(first, &mut cursor).unwrap();
        assert_eq!(acc.total_saved, 0);
        assert_eq!(acc.project_path.as_deref(), Some("p"));
        assert_eq!(acc.user_name.as_deref(), Some("u"));
        assert!(acc.breakdown_json.is_empty());
    }
}
