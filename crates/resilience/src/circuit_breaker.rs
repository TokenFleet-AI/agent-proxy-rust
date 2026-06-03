//! Circuit Breaker middleware.
//!
//! Three-state circuit breaker (Closed → Open → Half-Open) per channel.
//! After `failure_threshold` consecutive failures, the circuit opens and
//! immediately rejects requests for `cooldown` duration. After cooldown,
//! one probe request is allowed (Half-Open); success closes the circuit,
//! failure re-opens it.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use agent_proxy_rust_core::{
    ProxyError,
    extensions::EXT_SELECTED_CHANNEL,
    middleware::ProxyMiddleware,
    types::{ChannelConfig, ConnectionContext, ProxyRequest, ProxyResponse},
};
use async_trait::async_trait;
use http::StatusCode;

/// Circuit breaker configuration.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures to trip the circuit.
    pub failure_threshold: u32,
    /// Cooldown period before attempting a probe request.
    pub cooldown: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            cooldown: Duration::from_secs(60),
        }
    }
}

/// Circuit state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Circuit is closed — requests flow normally.
    Closed,
    /// Circuit is open — requests are rejected immediately.
    Open,
    /// One probe request is allowed to test if the upstream recovered.
    HalfOpen,
}

/// Per-channel circuit breaker state.
#[derive(Debug)]
struct ChannelCircuit {
    state: State,
    failure_count: u32,
    opened_at: Option<Instant>,
}

impl ChannelCircuit {
    fn new() -> Self {
        Self {
            state: State::Closed,
            failure_count: 0,
            opened_at: None,
        }
    }

    /// Returns `true` if a request should be allowed through.
    fn allow_request(&mut self, cooldown: Duration) -> bool {
        match self.state {
            State::Closed | State::HalfOpen => true,
            State::Open => {
                // Check if cooldown has expired
                if let Some(opened) = self.opened_at
                    && opened.elapsed() >= cooldown
                {
                    self.state = State::HalfOpen;
                    return true; // Allow one probe
                }
                false
            }
        }
    }

    /// Record a successful request.
    fn record_success(&mut self) {
        self.state = State::Closed;
        self.failure_count = 0;
        self.opened_at = None;
    }

    /// Record a failed request. Returns `true` if the circuit just tripped.
    fn record_failure(&mut self, threshold: u32) {
        self.failure_count += 1;
        if self.failure_count >= threshold {
            self.state = State::Open;
            self.opened_at = Some(Instant::now());
        }
    }
}

/// Circuit Breaker middleware.
#[derive(Debug)]
pub struct CircuitBreakerMiddleware {
    circuits: Mutex<HashMap<String, ChannelCircuit>>,
    config: CircuitBreakerConfig,
}

impl CircuitBreakerMiddleware {
    /// Creates a new circuit breaker.
    #[must_use]
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            circuits: Mutex::new(HashMap::new()),
            config,
        }
    }

    fn is_retryable(status: StatusCode) -> bool {
        status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS
    }
}

#[async_trait]
impl ProxyMiddleware for CircuitBreakerMiddleware {
    async fn on_request(
        &self,
        _req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError> {
        let channel_id = ctx
            .get::<ChannelConfig>(EXT_SELECTED_CHANNEL)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        if channel_id.is_empty() {
            return Ok(());
        }

        let mut circuits = self
            .circuits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let circuit = circuits
            .entry(channel_id.clone())
            .or_insert_with(ChannelCircuit::new);

        if !circuit.allow_request(self.config.cooldown) {
            return Err(ProxyError::CircuitOpen {
                channel: channel_id,
            });
        }
        Ok(())
    }

    async fn on_response(
        &self,
        res: &mut ProxyResponse,
        ctx: &ConnectionContext,
    ) -> Result<(), ProxyError> {
        let channel_id = ctx
            .get::<ChannelConfig>(EXT_SELECTED_CHANNEL)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        if channel_id.is_empty() {
            return Ok(());
        }

        let mut circuits = self
            .circuits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let circuit = circuits
            .entry(channel_id)
            .or_insert_with(ChannelCircuit::new);

        if Self::is_retryable(res.status) {
            circuit.record_failure(self.config.failure_threshold);
        } else {
            circuit.record_success();
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "circuit-breaker"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_circuit_starts_closed() {
        let cb = CircuitBreakerMiddleware::new(CircuitBreakerConfig {
            failure_threshold: 3,
            cooldown: Duration::from_millis(50),
        });
        let mut circuits = cb.circuits.lock().unwrap();
        let circuit = circuits
            .entry("ch-1".into())
            .or_insert_with(ChannelCircuit::new);
        assert_eq!(circuit.state, State::Closed);
    }

    #[test]
    fn test_circuit_opens_after_threshold() {
        let cb = CircuitBreakerMiddleware::new(CircuitBreakerConfig {
            failure_threshold: 2,
            cooldown: Duration::from_millis(50),
        });

        let channel = "ch-test";
        let mut circuits = cb.circuits.lock().unwrap();
        let circuit = circuits
            .entry(channel.into())
            .or_insert_with(ChannelCircuit::new);

        // First failure
        circuit.record_failure(2);
        assert_eq!(circuit.state, State::Closed);
        assert_eq!(circuit.failure_count, 1);

        // Second failure → trip
        circuit.record_failure(2);
        assert_eq!(circuit.state, State::Open);
        assert_eq!(circuit.failure_count, 2);
    }

    #[test]
    fn test_open_circuit_rejects_requests() {
        let cb = CircuitBreakerMiddleware::new(CircuitBreakerConfig {
            failure_threshold: 1,
            cooldown: Duration::from_secs(60),
        });

        let mut circuits = cb.circuits.lock().unwrap();
        let circuit = circuits
            .entry("ch-2".into())
            .or_insert_with(ChannelCircuit::new);
        circuit.record_failure(1); // Trip immediately

        // Request should be rejected
        assert!(!circuit.allow_request(Duration::from_secs(60)));
    }

    #[test]
    fn test_circuit_allows_probe_after_cooldown() {
        let cb = CircuitBreakerMiddleware::new(CircuitBreakerConfig {
            failure_threshold: 1,
            cooldown: Duration::from_millis(1), // Very short cooldown for testing
        });

        let mut circuits = cb.circuits.lock().unwrap();
        let circuit = circuits
            .entry("ch-3".into())
            .or_insert_with(ChannelCircuit::new);

        // Trip the circuit
        circuit.record_failure(1);
        assert_eq!(circuit.state, State::Open);

        // After cooldown, allow probe (Half-Open)
        std::thread::sleep(Duration::from_millis(2));
        assert!(circuit.allow_request(Duration::from_millis(1)));
        assert_eq!(circuit.state, State::HalfOpen);
    }

    #[test]
    fn test_success_closes_half_open_circuit() {
        let cb = CircuitBreakerMiddleware::new(CircuitBreakerConfig {
            failure_threshold: 1,
            cooldown: Duration::from_millis(1),
        });

        let mut circuits = cb.circuits.lock().unwrap();
        let circuit = circuits
            .entry("ch-4".into())
            .or_insert_with(ChannelCircuit::new);

        // Trip + cooldown + probe
        circuit.record_failure(1);
        std::thread::sleep(Duration::from_millis(2));
        circuit.allow_request(Duration::from_millis(1));
        assert_eq!(circuit.state, State::HalfOpen);

        // Success → close
        circuit.record_success();
        assert_eq!(circuit.state, State::Closed);
        assert_eq!(circuit.failure_count, 0);
    }
}
