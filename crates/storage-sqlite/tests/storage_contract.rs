//! Contract tests for the `SQLite` storage backend — aligned with token-fleet-switch schema.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use agent_proxy_rust_storage::{CostFilter, CostRecord, Storage, SubscriptionFee, SwitchLog};
use agent_proxy_rust_storage_sqlite::SqliteStorage;
use chrono::Utc;
use serial_test::serial;

async fn setup() -> SqliteStorage {
    let storage = SqliteStorage::new_in_memory().expect("failed to create storage");
    storage.migrate().await.expect("migration failed");
    storage
}

// ── Provider ──────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_list_providers_after_migration() {
    let storage = setup().await;
    let providers = storage
        .list_providers()
        .await
        .expect("list_providers failed");
    assert_eq!(providers.len(), 7, "should have 7 seed providers");
    let anthropic = providers.iter().find(|p| p.name == "Anthropic").unwrap();
    assert_eq!(anthropic.id, "019a0000-0000-7000-0000-000000000001");
}

#[tokio::test]
#[serial]
async fn test_get_provider_exists() {
    let storage = setup().await;
    let result = storage
        .get_provider("019a0000-0000-7000-0000-000000000001")
        .await
        .expect("get_provider failed");
    assert!(result.is_some());
    assert_eq!(result.unwrap().name, "Anthropic");
}

#[tokio::test]
#[serial]
async fn test_get_provider_not_found() {
    let storage = setup().await;
    let result = storage
        .get_provider("nonexistent")
        .await
        .expect("get_provider failed");
    assert!(result.is_none());
}

// ── Model ─────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_list_models_all() {
    let storage = setup().await;
    let models = storage.list_models(None).await.expect("list_models failed");
    assert!(models.len() >= 21, "should have at least 21 seed models");
}

#[tokio::test]
#[serial]
async fn test_list_models_filtered_by_provider() {
    let storage = setup().await;
    // Filter by channel id (providers share channel namespace in V1 schema)
    let models = storage
        .list_models(Some("anthropic-official"))
        .await
        .expect("list_models failed");
    assert_eq!(models.len(), 3);
    assert!(models.iter().all(|m| m.provider_id == "anthropic-official"));
}

#[tokio::test]
#[serial]
async fn test_get_model_exists() {
    let storage = setup().await;
    let model = storage
        .get_model("anthropic-official:claude-sonnet-4-6")
        .await
        .expect("get_model failed")
        .unwrap();
    assert_eq!(model.client_name, "claude-sonnet-4-6");
    assert_eq!(model.currency, "USD");
}

// ── Channel ───────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_list_channels_all() {
    let storage = setup().await;
    let channels = storage
        .list_channels(None)
        .await
        .expect("list_channels failed");
    assert_eq!(channels.len(), 13, "should have 13 seed channels");
}

#[tokio::test]
#[serial]
async fn test_get_channel_fields() {
    let storage = setup().await;
    let channel = storage
        .get_channel("anthropic-official")
        .await
        .expect("get_channel failed")
        .unwrap();
    assert_eq!(channel.name, "Anthropic Official");
    assert_eq!(channel.base_url, "https://api.anthropic.com");
    assert_eq!(channel.billing_type, "metered");
    assert_eq!(channel.health_status, "Healthy");
    assert!(channel.enabled);
}

#[tokio::test]
#[serial]
async fn test_list_channels_filtered_by_model() {
    let storage = setup().await;
    let channels = storage
        .list_channels(Some("qwen3.5-plus"))
        .await
        .expect("list_channels failed");
    assert!(
        channels.len() >= 4,
        "qwen3.5-plus should have at least 4 channels"
    );
}

#[tokio::test]
#[serial]
async fn test_update_channel() {
    let storage = setup().await;
    let updated = storage
        .update_channel(
            "anthropic-official",
            Some("Updated Name"),
            None,
            Some(99),
            Some(500_000),
            Some("Block"),
        )
        .await
        .expect("update_channel failed");
    assert_eq!(updated.name, "Updated Name");
    assert_eq!(updated.priority, 99);
    assert_eq!(updated.monthly_quota, Some(500_000));
    assert_eq!(updated.quota_policy, "Block");
}

#[tokio::test]
#[serial]
async fn test_mark_channel_healthy() {
    let storage = setup().await;
    storage
        .mark_channel_healthy("anthropic-official")
        .await
        .expect("mark_channel_healthy failed");
    let channel = storage
        .get_channel("anthropic-official")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(channel.health_status, "Healthy");
    assert_eq!(channel.consecutive_failures, 0);
}

#[tokio::test]
#[serial]
async fn test_record_channel_failure_sequence() {
    let storage = setup().await;
    // 1st failure → Degraded
    storage
        .record_channel_failure("anthropic-official")
        .await
        .unwrap();
    let ch = storage
        .get_channel("anthropic-official")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ch.health_status, "Degraded");
    assert_eq!(ch.consecutive_failures, 1);

    // 2 more failures → Cooldown
    storage
        .record_channel_failure("anthropic-official")
        .await
        .unwrap();
    storage
        .record_channel_failure("anthropic-official")
        .await
        .unwrap();
    let ch = storage
        .get_channel("anthropic-official")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ch.health_status, "Cooldown");
    assert_eq!(ch.consecutive_failures, 3);
}

#[tokio::test]
#[serial]
async fn test_delete_channel() {
    let storage = setup().await;
    storage
        .delete_channel("anthropic-official")
        .await
        .expect("delete failed");
    assert!(
        storage
            .get_channel("anthropic-official")
            .await
            .unwrap()
            .is_none()
    );
}

// ── Cost Record ───────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_insert_and_query_cost_records() {
    let storage = setup().await;
    let record = CostRecord {
        id: uuid::Uuid::now_v7().to_string(),
        channel_id: "anthropic-official".into(),
        project: "/test/project".into(),
        user_id: "test-user".into(),
        agent_type: "ClaudeCode".into(),
        input_tokens: 1000,
        output_tokens: 500,
        cache_write_tokens: 0,
        cache_read_tokens: 50,
        thinking_tokens: 100,
        cost: 0.015,
        schema_saved_tokens: 0,
        response_saved_tokens: 0,
        rtk_saved_tokens: 0,
        pre_compress_tokens: 1500,
        post_compress_tokens: 1400,
        compression_tokens_saved: 100,
        unit: "USD".into(),
        timestamp: Utc::now().to_rfc3339(),
    };
    storage
        .insert_cost_record(&record)
        .await
        .expect("insert failed");
    let records = storage
        .query_cost_records(CostFilter {
            project_path: Some("/test/project".into()),
            model_name: None,
            channel_name: None,
            time_range: None,
            limit: Some(10),
            offset: None,
        })
        .await
        .expect("query failed");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].input_tokens, 1000);
}

// ── Switch Log ────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_insert_switch_log() {
    let storage = setup().await;
    let log = SwitchLog {
        id: uuid::Uuid::now_v7().to_string(),
        from_channel_id: "anthropic-official".into(),
        to_channel_id: "openai-official".into(),
        reason: "health check failed".into(),
        cost_record_id: None,
        created_at: Utc::now().to_rfc3339(),
    };
    storage
        .insert_switch_log(&log)
        .await
        .expect("insert failed");
}

// ── Subscription Fee ──────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_insert_and_query_subscription_fees() {
    let storage = setup().await;
    let fee = SubscriptionFee {
        id: 0,
        channel_name: "DashScope Coding Plan".into(),
        month: "2026-05".into(),
        monthly_price: 200.0,
        currency: "CNY".into(),
    };
    storage
        .insert_subscription_fee(&fee)
        .await
        .expect("insert failed");
    let fees = storage
        .query_subscription_fees(None, None)
        .await
        .expect("query failed");
    assert_eq!(fees.len(), 1);
}

// ── Health & Lifecycle ────────────────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_health_check_returns_true() {
    let storage = setup().await;
    assert!(storage.health_check().await.expect("health_check failed"));
}

#[tokio::test]
#[serial]
async fn test_max_connections_is_one() {
    assert_eq!(setup().await.max_connections(), 1);
}

#[tokio::test]
#[serial]
async fn test_migrate_is_idempotent() {
    let storage = setup().await;
    storage.migrate().await.expect("second migrate failed");
    let channels = storage.list_channels(None).await.unwrap();
    assert_eq!(channels.len(), 13, "seed data must not be duplicated");
}
