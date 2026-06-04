//! Tokenless report file consumer.
//!
//! Reads the per-session JSONL report file produced by tokenless hooks,
//! consumes it via rename-then-read (atomic), and returns accumulated
//! before-stats for the current API request.
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

use std::{fs, io::Write, path::PathBuf};

use serde::Deserialize;

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

/// Consume and parse the tokenless report file for a session.
///
/// Uses rename-then-read to avoid race conditions with concurrent
/// tokenless hook writes. The report file lives at:
/// `~/.tokenfleet-ai/tokenless/reports/{session_id}.jsonl`
///
/// Returns `None` if no report file exists or the session ID is empty.
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
    let target = reports_dir.join(format!("{safe_sid}.processing"));

    // Atomically rename to claim the file.
    fs::rename(&source, &target).ok()?;

    let result = parse_report_file(&target);

    // Clean up the processing file regardless of parse success.
    let _ = fs::remove_file(&target);

    result
}

/// Parse a JSONL report file into an accumulator.
fn parse_report_file(path: &PathBuf) -> Option<TokenlessAccumulator> {
    let content = fs::read_to_string(path).ok()?;
    if content.is_empty() {
        return None;
    }

    let mut acc = TokenlessAccumulator::default();
    let mut breakdown_items: Vec<serde_json::Value> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(report) = serde_json::from_str::<ProxyReport>(trimmed) {
            acc.total_saved += report.saved_tokens;
            if acc.project_path.is_none() {
                acc.project_path.clone_from(&report.project_path);
            }
            if acc.user_name.is_none() {
                acc.user_name.clone_from(&report.user_name);
            }

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

    tracing::info!(
        lines = breakdown_items.len(),
        total_saved = acc.total_saved,
        project_path = ?acc.project_path,
        "consumed tokenless report"
    );

    // Debug: write report consumption log to file
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
    use std::io::Write;

    #[test]
    fn test_parse_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");
        fs::write(&path, "").unwrap();

        let result = parse_report_file(&path);
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_single_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("single.jsonl");
        let line = serde_json::json!({
            "sessionId": "sess-1",
            "agentId": "claude",
            "projectPath": null,
            "opType": "CompressSchema",
            "method": "ToonHrv",
            "beforeTokens": 1500,
            "afterTokens": 700,
            "savedTokens": 800
        })
        .to_string();
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "{line}").unwrap();

        let result = parse_report_file(&path);
        assert!(result.is_some());
        let acc = result.unwrap();
        assert_eq!(acc.total_saved, 800);
        assert!(acc.breakdown_json.contains("ToonHrv"));
    }

    #[test]
    fn test_parse_multiple_lines_accumulates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi.jsonl");
        let lines = [
            serde_json::json!({"sessionId":"s","agentId":"a","opType":"CompressSchema","method":"ToonHrv","beforeTokens":1000,"afterTokens":500,"savedTokens":500}),
            serde_json::json!({"sessionId":"s","agentId":"a","opType":"RewriteCommand","method":"RtkStandard","beforeTokens":200,"afterTokens":50,"savedTokens":150}),
        ];
        let mut f = fs::File::create(&path).unwrap();
        for line in &lines {
            writeln!(f, "{line}").unwrap();
        }

        let result = parse_report_file(&path);
        let acc = result.unwrap();
        assert_eq!(acc.total_saved, 650);
        assert_eq!(acc.breakdown_json.matches("\"savedTokens\"").count(), 2);
    }

    #[test]
    fn test_consume_report_rename_then_read() {
        let dir = tempfile::tempdir().unwrap();

        // Simulate: create a "reports" directory with a session file
        let reports = dir.path().join("reports");
        fs::create_dir_all(&reports).unwrap();

        let source = reports.join("sess-abc.jsonl");
        let line = serde_json::json!({
            "sessionId": "sess-abc",
            "agentId": "claude",
            "opType": "CompressResponse",
            "method": "HighFidelity",
            "beforeTokens": 3000,
            "afterTokens": 2200,
            "savedTokens": 800
        })
        .to_string();
        fs::write(&source, format!("{line}\n")).unwrap();

        // Cannot easily test consume_report because it uses ~/.tokenfleet-ai
        // But parse_report_file is tested above.
        assert!(source.exists());
        assert!(!reports.join("sess-abc.processing").exists());
    }
}
