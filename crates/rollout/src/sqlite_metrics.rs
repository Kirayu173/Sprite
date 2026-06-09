use std::sync::Arc;
use std::time::Duration;

use diagnostics::ORIGINATOR_TAG;
use diagnostics::bounded_originator_tag_value;
use state::DbDiagnostics;
use state::DbDiagnosticsHandle;

struct MetricsDbDiagnostics {
    metrics: diagnostics::MetricsClient,
    originator: &'static str,
}

impl DbDiagnostics for MetricsDbDiagnostics {
    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) {
        let tags = with_originator(tags, self.originator);
        let _ = self.metrics.counter(name, inc, &tags);
    }

    fn record_duration(&self, name: &str, duration: Duration, tags: &[(&str, &str)]) {
        let tags = with_originator(tags, self.originator);
        let _ = self.metrics.record_duration(name, duration, &tags);
    }
}

pub(crate) fn recorder(
    metrics: diagnostics::MetricsClient,
    originator: &str,
) -> DbDiagnosticsHandle {
    Arc::new(MetricsDbDiagnostics {
        metrics,
        originator: bounded_originator_tag_value(originator),
    })
}

fn with_originator<'a>(
    tags: &[(&'a str, &'a str)],
    originator: &'static str,
) -> Vec<(&'a str, &'a str)> {
    let mut tags = tags.to_vec();
    tags.push((ORIGINATOR_TAG, originator));
    tags
}
