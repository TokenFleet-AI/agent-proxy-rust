//! Contract tests for the [`Storage`] trait using the `SQLite` implementation.
//!
//! Each test uses `SqliteStorage::new_in_memory()` for isolation.
//! Every trait method has at least 2 test cases: normal path + error/edge case.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use agent_proxy_rust_storage::{
    Channel, CostFilter, CostGroupBy, CostRecord, ModelMapping, Storage, StorageError,
    SubscriptionFee, TimeRange,
};
use agent_proxy_rust_storage_sqlite::SqliteStorage;
use chrono::{Duration, Utc};
use secrecy::{ExposeSecret, SecretString};

fn test_channel(id: &str) -> Channel {
    Channel {
        id: id.to_string(),
        name: format!("Test {id}"),
        url: "https://api.example.com".to_string(),
        api_key: SecretString::new("sk-test".to_string().into_boxed_str()),
        protocol: "anthropic_messages".to_string(),
        is_builtin: false,
        enabled: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn test_mapping(id: &str, channel_id: &str) -> ModelMapping {
    ModelMapping {
        id: id.to_string(),
        channel_id: channel_id.to_string(),
        client_name: "claude-sonnet".to_string(),
        upstream_name: "claude-sonnet-4-7".to_string(),
        billing: "metered".to_string(),
        pricing_json: r#"{"mode":"per_token","input_per_mtok":3.0}"#.to_string(),
        weight: 130,
        enabled: true,
    }
}

fn test_cost_record(project_path: &str, model_name: &str, cost: f64) -> CostRecord {
    CostRecord {
        id: 0,
        timestamp: Utc::now(),
        user_name: "test-user".to_string(),
        project_path: project_path.to_string(),
        project_name: "test-project".to_string(),
        agent_type: "claude".to_string(),
        agent_role: None,
        channel_name: "test-channel".to_string(),
        channel_kind: "metered".to_string(),
        model_name: model_name.to_string(),
        input_tokens: 1300,
        output_tokens: 500,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        thinking_tokens: 0,
        actual_cost: cost,
        unit: "USD".to_string(),
        pre_compress_tokens: 2000,
        post_compress_tokens: 1500,
        compression_tokens_saved: 500,
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-13
}

async fn setup() -> SqliteStorage {
    let storage = SqliteStorage::new_in_memory().expect("failed to create in-memory storage");
    storage.migrate().await.expect("migration failed");
    storage
}

// ── Channel ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_channels_after_seed() {
    let storage = setup().await;
    let channels = storage.list_channels().await.expect("list_channels failed");
    assert_eq!(channels.len(), 13, "should have 13 seeded channels");
    let ids: Vec<&str> = channels.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&"anthropic-official"));
    assert!(ids.contains(&"openai-official"));

    assert!(ids.contains(&"deepseek-openai"));
}

#[tokio::test]
async fn test_list_channels_after_insert() {
    let storage = setup().await;
    let ch = test_channel("custom-channel");
    storage.upsert_channel(&ch).await.expect("upsert failed");
    let channels = storage.list_channels().await.expect("list_channels failed");
    assert_eq!(channels.len(), 14);
}

#[tokio::test]
async fn test_get_channel_existing() {
    let storage = setup().await;
    let ch = storage
        .get_channel("anthropic-official")
        .await
        .expect("get_channel failed");
    assert!(ch.is_some());
    let ch = ch.unwrap();
    assert_eq!(ch.id, "anthropic-official");
    assert_eq!(ch.protocol, "anthropic_messages");
    assert!(ch.is_builtin);
}

#[tokio::test]
async fn test_get_channel_not_found() {
    let storage = setup().await;
    let ch = storage
        .get_channel("nonexistent")
        .await
        .expect("get_channel failed");
    assert!(ch.is_none());
}

#[tokio::test]
async fn test_upsert_channel_insert() {
    let storage = setup().await;
    let ch = test_channel("new-channel");
    storage.upsert_channel(&ch).await.expect("upsert failed");
    let got = storage
        .get_channel("new-channel")
        .await
        .expect("get failed");
    assert!(got.is_some());
    assert_eq!(got.unwrap().name, "Test new-channel");
}

#[tokio::test]
async fn test_upsert_channel_update() {
    let storage = setup().await;
    let mut ch = test_channel("update-me");
    storage.upsert_channel(&ch).await.expect("upsert failed");
    ch.name = "Updated Name".to_string();
    storage.upsert_channel(&ch).await.expect("upsert failed");
    let got = storage.get_channel("update-me").await.expect("get failed");
    assert_eq!(got.unwrap().name, "Updated Name");
}

#[tokio::test]
async fn test_set_channel_enabled_normal() {
    let storage = setup().await;
    let ch = test_channel("toggle-me");
    storage.upsert_channel(&ch).await.expect("upsert failed");
    storage
        .set_channel_enabled("toggle-me", false)
        .await
        .expect("set_enabled failed");
    let got = storage.get_channel("toggle-me").await.expect("get failed");
    assert!(!got.unwrap().enabled);

    storage
        .set_channel_enabled("toggle-me", true)
        .await
        .expect("set_enabled failed");
    let got = storage.get_channel("toggle-me").await.expect("get failed");
    assert!(got.unwrap().enabled);
}

#[tokio::test]
async fn test_set_channel_enabled_not_found() {
    let storage = setup().await;
    let result = storage.set_channel_enabled("no-such-channel", false).await;
    assert!(matches!(result, Err(StorageError::NotFound(_))));
}

#[tokio::test]
async fn test_set_channel_api_key_normal() {
    let storage = setup().await;
    let ch = test_channel("key-test");
    storage.upsert_channel(&ch).await.expect("upsert failed");
    let new_key = SecretString::new("sk-new-key".to_string().into_boxed_str());
    storage
        .set_channel_api_key("key-test", &new_key)
        .await
        .expect("set_api_key failed");
    let got = storage.get_channel("key-test").await.expect("get failed");
    assert_eq!(got.unwrap().api_key.expose_secret(), "sk-new-key");
}

#[tokio::test]
async fn test_set_channel_api_key_not_found() {
    let storage = setup().await;
    let key = SecretString::new("sk-test".to_string().into_boxed_str());
    let result = storage.set_channel_api_key("no-such-channel", &key).await;
    assert!(matches!(result, Err(StorageError::NotFound(_))));
}

#[tokio::test]
async fn test_delete_channel_normal() {
    let storage = setup().await;
    let ch = test_channel("delete-me");
    storage.upsert_channel(&ch).await.expect("upsert failed");
    storage
        .delete_channel("delete-me")
        .await
        .expect("delete failed");
    let got = storage.get_channel("delete-me").await.expect("get failed");
    assert!(got.is_none());
}

#[tokio::test]
async fn test_delete_channel_not_found() {
    let storage = setup().await;
    let result = storage.delete_channel("no-such-channel").await;
    assert!(matches!(result, Err(StorageError::NotFound(_))));
}

#[tokio::test]
async fn test_delete_channel_cascades_mappings() {
    let storage = setup().await;
    let ch = test_channel("cascade-test");
    storage.upsert_channel(&ch).await.expect("upsert failed");
    let mapping = test_mapping("m1", "cascade-test");
    storage
        .upsert_mapping(&mapping)
        .await
        .expect("upsert mapping failed");
    storage
        .delete_channel("cascade-test")
        .await
        .expect("delete failed");
    let mappings = storage
        .list_mappings("cascade-test")
        .await
        .expect("list failed");
    assert!(mappings.is_empty());
}

// ── Model Mapping ────────────────────────────────────────────────────

#[tokio::test]
async fn test_list_mappings_empty() {
    let storage = setup().await;
    let mappings = storage
        .list_mappings("anthropic-official")
        .await
        .expect("list_mappings failed");
    assert!(!mappings.is_empty(), "should have seeded mappings");
}

#[tokio::test]
async fn test_list_mappings_after_insert() {
    let storage = setup().await;
    let existing = storage
        .list_mappings("anthropic-official")
        .await
        .unwrap()
        .len();
    let mapping = test_mapping("m1", "anthropic-official");
    storage
        .upsert_mapping(&mapping)
        .await
        .expect("upsert failed");
    let mappings = storage
        .list_mappings("anthropic-official")
        .await
        .expect("list failed");
    assert_eq!(mappings.len(), existing + 1);
    assert!(mappings.iter().any(|m| m.id == "m1"));
}

#[tokio::test]
async fn test_upsert_mapping_insert() {
    let storage = setup().await;
    let channel = test_channel("test-upsert-channel");
    storage.upsert_channel(&channel).await.unwrap();
    let mapping = test_mapping("map-insert", "test-upsert-channel");
    storage
        .upsert_mapping(&mapping)
        .await
        .expect("upsert failed");
    let mappings = storage
        .list_mappings("test-upsert-channel")
        .await
        .expect("list failed");
    assert_eq!(mappings.len(), 1);
}

#[tokio::test]
async fn test_upsert_mapping_update() {
    let storage = setup().await;
    let mut mapping = test_mapping("map-update", "anthropic-official");
    storage
        .upsert_mapping(&mapping)
        .await
        .expect("upsert failed");
    mapping.client_name = "claude-opus".to_string();
    storage
        .upsert_mapping(&mapping)
        .await
        .expect("upsert failed");
    let mappings = storage
        .list_mappings("anthropic-official")
        .await
        .expect("list failed");
    let updated = mappings
        .iter()
        .find(|m| m.id == "map-update")
        .expect("mapping not found");
    assert_eq!(updated.client_name, "claude-opus");
}

#[tokio::test]
async fn test_upsert_mapping_nonexistent_channel() {
    let storage = setup().await;
    let mapping = test_mapping("orphan", "no-such-channel");
    let result = storage.upsert_mapping(&mapping).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_set_mapping_enabled_normal() {
    let storage = setup().await;
    let mapping = test_mapping("toggle-mapping", "anthropic-official");
    storage
        .upsert_mapping(&mapping)
        .await
        .expect("upsert failed");
    storage
        .set_mapping_enabled("toggle-mapping", false)
        .await
        .expect("set_enabled failed");
    let mappings = storage
        .list_mappings("anthropic-official")
        .await
        .expect("list failed");
    let toggled = mappings
        .iter()
        .find(|m| m.id == "toggle-mapping")
        .expect("mapping not found");
    assert!(!toggled.enabled);
}

#[tokio::test]
async fn test_set_mapping_enabled_not_found() {
    let storage = setup().await;
    let result = storage.set_mapping_enabled("no-such-mapping", false).await;
    assert!(matches!(result, Err(StorageError::NotFound(_))));
}

#[tokio::test]
async fn test_delete_mapping_normal() {
    let storage = setup().await;
    let mapping = test_mapping("delete-map", "anthropic-official");
    storage
        .upsert_mapping(&mapping)
        .await
        .expect("upsert failed");
    storage
        .delete_mapping("delete-map")
        .await
        .expect("delete failed");
    let mappings = storage
        .list_mappings("anthropic-official")
        .await
        .expect("list failed");
    assert!(
        mappings.iter().all(|m| m.id != "delete-map"),
        "deleted mapping should not appear"
    );
}

#[tokio::test]
async fn test_delete_mapping_not_found() {
    let storage = setup().await;
    let result = storage.delete_mapping("no-such-mapping").await;
    assert!(matches!(result, Err(StorageError::NotFound(_))));
}

// ── Cost Records ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_insert_and_query_cost_record() {
    let storage = setup().await;
    let record = test_cost_record("/projects/app", "claude-sonnet", 0.015);
    storage
        .insert_cost_record(&record)
        .await
        .expect("insert failed");
    let records = storage
        .query_cost_records(CostFilter {
            project_path: None,
            model_name: None,
            channel_name: None,
            time_range: None,
            limit: None,
            offset: None,
        })
        .await
        .expect("query failed");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].project_path, "/projects/app");
    assert!(approx_eq(records[0].actual_cost, 0.015));
}

#[tokio::test]
async fn test_insert_multiple_cost_records() {
    let storage = setup().await;
    for i in 0..5 {
        let record = test_cost_record(
            &format!("/projects/app{i}"),
            "claude-sonnet",
            f64::from(i) * 0.01,
        );
        storage
            .insert_cost_record(&record)
            .await
            .expect("insert failed");
    }
    let records = storage
        .query_cost_records(CostFilter {
            project_path: None,
            model_name: None,
            channel_name: None,
            time_range: None,
            limit: None,
            offset: None,
        })
        .await
        .expect("query failed");
    assert_eq!(records.len(), 5);
}

#[tokio::test]
async fn test_query_cost_records_filter_project_path() {
    let storage = setup().await;
    storage
        .insert_cost_record(&test_cost_record("/projects/alpha", "gpt-4", 0.01))
        .await
        .expect("insert failed");
    storage
        .insert_cost_record(&test_cost_record("/projects/beta", "gpt-4", 0.02))
        .await
        .expect("insert failed");

    let records = storage
        .query_cost_records(CostFilter {
            project_path: Some("/projects/alpha".to_string()),
            model_name: None,
            channel_name: None,
            time_range: None,
            limit: None,
            offset: None,
        })
        .await
        .expect("query failed");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].project_path, "/projects/alpha");
}

#[tokio::test]
async fn test_query_cost_records_filter_model_name() {
    let storage = setup().await;
    storage
        .insert_cost_record(&test_cost_record("/p/a", "model-a", 0.01))
        .await
        .expect("insert failed");
    storage
        .insert_cost_record(&test_cost_record("/p/b", "model-b", 0.02))
        .await
        .expect("insert failed");

    let records = storage
        .query_cost_records(CostFilter {
            project_path: None,
            model_name: Some("model-a".to_string()),
            channel_name: None,
            time_range: None,
            limit: None,
            offset: None,
        })
        .await
        .expect("query failed");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model_name, "model-a");
}

#[tokio::test]
async fn test_query_cost_records_filter_time_range() {
    let storage = setup().await;
    let old = Utc::now() - Duration::days(13);
    let recent = Utc::now();

    let mut old_record = test_cost_record("/old", "claude", 0.01);
    old_record.timestamp = old;
    storage
        .insert_cost_record(&old_record)
        .await
        .expect("insert failed");

    let mut recent_record = test_cost_record("/recent", "claude", 0.02);
    recent_record.timestamp = recent;
    storage
        .insert_cost_record(&recent_record)
        .await
        .expect("insert failed");

    let range = TimeRange {
        start: Utc::now() - Duration::days(5),
        end: Utc::now() + Duration::days(1),
    };
    let records = storage
        .query_cost_records(CostFilter {
            project_path: None,
            model_name: None,
            channel_name: None,
            time_range: Some(range),
            limit: None,
            offset: None,
        })
        .await
        .expect("query failed");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].project_path, "/recent");
}

#[tokio::test]
async fn test_query_cost_records_with_limit() {
    let storage = setup().await;
    for i in 0..13 {
        storage
            .insert_cost_record(&test_cost_record(
                &format!("/p/{i}"),
                "m",
                f64::from(i) * 0.01,
            ))
            .await
            .expect("insert failed");
    }

    let records = storage
        .query_cost_records(CostFilter {
            project_path: None,
            model_name: None,
            channel_name: None,
            time_range: None,
            limit: Some(3),
            offset: None,
        })
        .await
        .expect("query failed");
    assert_eq!(records.len(), 3);
}

#[tokio::test]
async fn test_query_cost_records_empty_result() {
    let storage = setup().await;
    let records = storage
        .query_cost_records(CostFilter {
            project_path: Some("/no/such/path".to_string()),
            model_name: None,
            channel_name: None,
            time_range: None,
            limit: None,
            offset: None,
        })
        .await
        .expect("query failed");
    assert!(records.is_empty());
}

// ── Cost Aggregation ─────────────────────────────────────────────────

#[tokio::test]
async fn test_aggregate_costs_by_project() {
    let storage = setup().await;
    storage
        .insert_cost_record(&test_cost_record("/proj/a", "m1", 13.0))
        .await
        .expect("insert failed");
    storage
        .insert_cost_record(&test_cost_record("/proj/a", "m2", 20.0))
        .await
        .expect("insert failed");
    storage
        .insert_cost_record(&test_cost_record("/proj/b", "m1", 5.0))
        .await
        .expect("insert failed");

    let range = TimeRange {
        start: Utc::now() - Duration::days(1),
        end: Utc::now() + Duration::days(1),
    };
    let agg = storage
        .aggregate_costs(CostGroupBy::Project, range)
        .await
        .expect("aggregate failed");
    assert_eq!(agg.len(), 2, "should have 2 project groups");
    // Sorted by total_actual_cost DESC
    assert!(approx_eq(agg[0].total_actual_cost, 33.0));
    assert!(approx_eq(agg[1].total_actual_cost, 5.0));
}

#[tokio::test]
async fn test_aggregate_costs_by_model() {
    let storage = setup().await;
    storage
        .insert_cost_record(&test_cost_record("/p/a", "sonnet", 13.0))
        .await
        .expect("insert failed");
    storage
        .insert_cost_record(&test_cost_record("/p/b", "sonnet", 15.0))
        .await
        .expect("insert failed");
    storage
        .insert_cost_record(&test_cost_record("/p/c", "opus", 50.0))
        .await
        .expect("insert failed");

    let range = TimeRange {
        start: Utc::now() - Duration::days(1),
        end: Utc::now() + Duration::days(1),
    };
    let agg = storage
        .aggregate_costs(CostGroupBy::Model, range)
        .await
        .expect("aggregate failed");
    assert_eq!(agg.len(), 2);
    assert_eq!(agg[0].group_key, "opus");
    assert!(approx_eq(agg[0].total_actual_cost, 50.0));
}

#[tokio::test]
async fn test_aggregate_costs_empty() {
    let storage = setup().await;
    let range = TimeRange {
        start: Utc::now() - Duration::days(1),
        end: Utc::now() + Duration::days(1),
    };
    let agg = storage
        .aggregate_costs(CostGroupBy::Project, range)
        .await
        .expect("aggregate failed");
    assert!(agg.is_empty());
}

#[tokio::test]
async fn test_aggregate_costs_by_project_model_month() {
    let storage = setup().await;
    storage
        .insert_cost_record(&test_cost_record("/p/a", "m1", 5.0))
        .await
        .expect("insert failed");
    storage
        .insert_cost_record(&test_cost_record("/p/a", "m1", 5.0))
        .await
        .expect("insert failed");

    let range = TimeRange {
        start: Utc::now() - Duration::days(1),
        end: Utc::now() + Duration::days(1),
    };
    let agg = storage
        .aggregate_costs(CostGroupBy::ProjectModelMonth, range)
        .await
        .expect("aggregate failed");
    assert_eq!(agg.len(), 1);
    assert_eq!(agg[0].request_count, 2);
    assert!(approx_eq(agg[0].total_actual_cost, 10.0));
}

// ── Prune ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_prune_cost_records_deletes_old() {
    let storage = setup().await;
    let mut old = test_cost_record("/old", "m", 1.0);
    old.timestamp = Utc::now() - Duration::days(130);
    storage
        .insert_cost_record(&old)
        .await
        .expect("insert failed");

    let deleted = storage.prune_cost_records(90).await.expect("prune failed");
    assert_eq!(deleted, 1);
}

#[tokio::test]
async fn test_prune_cost_records_keeps_recent() {
    let storage = setup().await;
    let mut recent = test_cost_record("/recent", "m", 1.0);
    recent.timestamp = Utc::now() - Duration::days(5);
    storage
        .insert_cost_record(&recent)
        .await
        .expect("insert failed");

    let deleted = storage.prune_cost_records(90).await.expect("prune failed");
    assert_eq!(deleted, 0);
}

#[tokio::test]
async fn test_prune_cost_records_empty() {
    let storage = setup().await;
    let deleted = storage.prune_cost_records(30).await.expect("prune failed");
    assert_eq!(deleted, 0);
}

// ── Subscription Fees ────────────────────────────────────────────────

#[tokio::test]
async fn test_insert_and_query_subscription_fee() {
    let storage = setup().await;
    let fee = SubscriptionFee {
        id: 0,
        channel_name: "anthropic-official".to_string(),
        month: "2027-05".to_string(),
        monthly_price: 20.0,
        currency: "USD".to_string(),
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
    assert!(approx_eq(fees[0].monthly_price, 20.0));
}

#[tokio::test]
async fn test_insert_multiple_subscription_fees() {
    let storage = setup().await;
    for month in &["2027-01", "2027-02", "2027-03"] {
        storage
            .insert_subscription_fee(&SubscriptionFee {
                id: 0,
                channel_name: "test".to_string(),
                month: (*month).to_string(),
                monthly_price: 20.0,
                currency: "USD".to_string(),
            })
            .await
            .expect("insert failed");
    }
    let fees = storage
        .query_subscription_fees(None, None)
        .await
        .expect("query failed");
    assert_eq!(fees.len(), 3);
}

#[tokio::test]
async fn test_query_subscription_fees_filter_by_channel() {
    let storage = setup().await;
    storage
        .insert_subscription_fee(&SubscriptionFee {
            id: 0,
            channel_name: "ch-a".to_string(),
            month: "2027-05".to_string(),
            monthly_price: 13.0,
            currency: "USD".to_string(),
        })
        .await
        .expect("insert failed");
    storage
        .insert_subscription_fee(&SubscriptionFee {
            id: 0,
            channel_name: "ch-b".to_string(),
            month: "2027-05".to_string(),
            monthly_price: 20.0,
            currency: "USD".to_string(),
        })
        .await
        .expect("insert failed");

    let fees = storage
        .query_subscription_fees(Some("ch-a"), None)
        .await
        .expect("query failed");
    assert_eq!(fees.len(), 1);
    assert_eq!(fees[0].channel_name, "ch-a");
}

#[tokio::test]
async fn test_query_subscription_fees_filter_by_month() {
    let storage = setup().await;
    storage
        .insert_subscription_fee(&SubscriptionFee {
            id: 0,
            channel_name: "test".to_string(),
            month: "2027-04".to_string(),
            monthly_price: 13.0,
            currency: "USD".to_string(),
        })
        .await
        .expect("insert failed");
    storage
        .insert_subscription_fee(&SubscriptionFee {
            id: 0,
            channel_name: "test".to_string(),
            month: "2027-05".to_string(),
            monthly_price: 20.0,
            currency: "USD".to_string(),
        })
        .await
        .expect("insert failed");

    let fees = storage
        .query_subscription_fees(None, Some("2027-05"))
        .await
        .expect("query failed");
    assert_eq!(fees.len(), 1);
    assert_eq!(fees[0].month, "2027-05");
}

#[tokio::test]
async fn test_query_subscription_fees_empty() {
    let storage = setup().await;
    let fees = storage
        .query_subscription_fees(None, None)
        .await
        .expect("query failed");
    assert!(fees.is_empty());
}

// ── Lifecycle ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_migrate_is_idempotent() {
    let storage = setup().await;
    // First migrate already happened in setup()
    // Second migrate should succeed without errors
    storage.migrate().await.expect("second migrate failed");
    // Tables should still exist
    let channels = storage.list_channels().await.expect("list failed");
    assert_eq!(channels.len(), 13);
}

#[tokio::test]
async fn test_migrate_creates_tables() {
    let storage = SqliteStorage::new_in_memory().expect("create failed");
    // Tables don't exist yet
    storage.migrate().await.expect("migrate failed");
    // Now tables exist
    let channels = storage.list_channels().await.expect("list failed");
    assert_eq!(
        channels.len(),
        13,
        "should have seeded channels after migration"
    );
}

#[tokio::test]
async fn test_health_check_ok() {
    let storage = setup().await;
    let healthy = storage.health_check().await.expect("health check failed");
    assert!(healthy);
}

#[tokio::test]
async fn test_max_connections() {
    let storage = setup().await;
    assert_eq!(storage.max_connections(), 1);
}

// ── Channel seed verification ────────────────────────────────────────

#[tokio::test]
async fn test_seeded_channels_are_builtin() {
    let storage = setup().await;
    let channels = storage.list_channels().await.expect("list failed");
    for ch in &channels {
        assert!(ch.is_builtin, "{} should be builtin", ch.id);
        assert!(ch.enabled, "{} should be enabled by default", ch.id);
    }
}
