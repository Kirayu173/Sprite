#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeMetricTotals {
    pub count: u64,
    pub duration_ms: u64,
}

impl RuntimeMetricTotals {
    pub fn is_empty(self) -> bool {
        self.count == 0 && self.duration_ms == 0
    }

    pub fn merge(&mut self, other: Self) {
        self.count = self.count.saturating_add(other.count);
        self.duration_ms = self.duration_ms.saturating_add(other.duration_ms);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeMetricsSummary {
    pub tool_calls: RuntimeMetricTotals,
    pub api_calls: RuntimeMetricTotals,
    pub streaming_events: RuntimeMetricTotals,
    pub websocket_calls: RuntimeMetricTotals,
    pub websocket_events: RuntimeMetricTotals,
    pub provider_overhead_ms: u64,
    pub provider_inference_time_ms: u64,
    pub provider_backend_ttft_ms: u64,
    pub provider_service_ttft_ms: u64,
    pub provider_backend_tbt_ms: u64,
    pub provider_service_tbt_ms: u64,
    pub turn_ttft_ms: u64,
    pub turn_ttfm_ms: u64,
}

impl RuntimeMetricsSummary {
    pub fn is_empty(self) -> bool {
        self.tool_calls.is_empty()
            && self.api_calls.is_empty()
            && self.streaming_events.is_empty()
            && self.websocket_calls.is_empty()
            && self.websocket_events.is_empty()
            && self.provider_overhead_ms == 0
            && self.provider_inference_time_ms == 0
            && self.provider_backend_ttft_ms == 0
            && self.provider_service_ttft_ms == 0
            && self.provider_backend_tbt_ms == 0
            && self.provider_service_tbt_ms == 0
            && self.turn_ttft_ms == 0
            && self.turn_ttfm_ms == 0
    }

    pub fn merge(&mut self, other: Self) {
        self.tool_calls.merge(other.tool_calls);
        self.api_calls.merge(other.api_calls);
        self.streaming_events.merge(other.streaming_events);
        self.websocket_calls.merge(other.websocket_calls);
        self.websocket_events.merge(other.websocket_events);
        if other.provider_overhead_ms > 0 {
            self.provider_overhead_ms = other.provider_overhead_ms;
        }
        if other.provider_inference_time_ms > 0 {
            self.provider_inference_time_ms = other.provider_inference_time_ms;
        }
        if other.provider_backend_ttft_ms > 0 {
            self.provider_backend_ttft_ms = other.provider_backend_ttft_ms;
        }
        if other.provider_service_ttft_ms > 0 {
            self.provider_service_ttft_ms = other.provider_service_ttft_ms;
        }
        if other.provider_backend_tbt_ms > 0 {
            self.provider_backend_tbt_ms = other.provider_backend_tbt_ms;
        }
        if other.provider_service_tbt_ms > 0 {
            self.provider_service_tbt_ms = other.provider_service_tbt_ms;
        }
        if other.turn_ttft_ms > 0 {
            self.turn_ttft_ms = other.turn_ttft_ms;
        }
        if other.turn_ttfm_ms > 0 {
            self.turn_ttfm_ms = other.turn_ttfm_ms;
        }
    }

    pub fn provider_summary(&self) -> RuntimeMetricsSummary {
        Self {
            provider_overhead_ms: self.provider_overhead_ms,
            provider_inference_time_ms: self.provider_inference_time_ms,
            provider_backend_ttft_ms: self.provider_backend_ttft_ms,
            provider_service_ttft_ms: self.provider_service_ttft_ms,
            provider_backend_tbt_ms: self.provider_backend_tbt_ms,
            provider_service_tbt_ms: self.provider_service_tbt_ms,
            ..RuntimeMetricsSummary::default()
        }
    }
}
