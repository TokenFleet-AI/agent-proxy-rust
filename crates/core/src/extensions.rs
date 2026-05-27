//! Well-known extension keys for
//! [`ConnectionContext::extensions`](crate::types::ConnectionContext::extensions).
//!
//! Middleware crates use these constants to read and write data in the
//! context extensions type-map. Using constants avoids typos and documents
//! the inter-middleware contract.

/// Key for the [`StatsRecord`] written by the compress middleware.
///
/// The value type is `StatsRecord` (to be defined by the compress crate).
/// Contains `pre_compress_tokens`, `post_compress_tokens`, and `compression_tokens_saved`.
pub const EXT_STATS_RECORD: &str = "stats_record";

/// Key for the selected [`ChannelConfig`](crate::types::ChannelConfig) written by the model-router
/// middleware.
pub const EXT_SELECTED_CHANNEL: &str = "selected_channel";

/// Key for the selected model mapping written by the model-router middleware.
///
/// The value type is the model mapping struct defined in the storage or model-router crate.
pub const EXT_SELECTED_MAPPING: &str = "selected_mapping";

/// Key for the bridge reverse flag.
///
/// Set by the bridge middleware to indicate that the response needs reverse
/// protocol conversion.
pub const EXT_BRIDGE_REVERSE: &str = "bridge_reverse";
