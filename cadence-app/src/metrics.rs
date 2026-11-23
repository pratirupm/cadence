use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Metrics {
    pub requests: AtomicU64,
    pub errors: AtomicU64,
    pub logins: AtomicU64,
    pub login_failures: AtomicU64,
    pub reminders: AtomicU64,
    pub webhook_deliveries: AtomicU64,
}

impl Metrics {
    pub fn inc(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        format!(
            concat!(
                "# HELP cadence_requests_total HTTP requests served.\n",
                "# TYPE cadence_requests_total counter\n",
                "cadence_requests_total {}\n",
                "# HELP cadence_errors_total HTTP errors served.\n",
                "# TYPE cadence_errors_total counter\n",
                "cadence_errors_total {}\n",
                "# HELP cadence_logins_total Successful logins.\n",
                "# TYPE cadence_logins_total counter\n",
                "cadence_logins_total {}\n",
                "# HELP cadence_login_failures_total Failed logins.\n",
                "# TYPE cadence_login_failures_total counter\n",
                "cadence_login_failures_total {}\n",
                "# HELP cadence_reminders_total Reminders delivered.\n",
                "# TYPE cadence_reminders_total counter\n",
                "cadence_reminders_total {}\n",
                "# HELP cadence_webhook_deliveries_total Webhook deliveries attempted.\n",
                "# TYPE cadence_webhook_deliveries_total counter\n",
                "cadence_webhook_deliveries_total {}\n",
            ),
            self.requests.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
            self.logins.load(Ordering::Relaxed),
            self.login_failures.load(Ordering::Relaxed),
            self.reminders.load(Ordering::Relaxed),
            self.webhook_deliveries.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prometheus_text() {
        let metrics = Metrics::default();
        metrics.inc(&metrics.requests);
        metrics.inc(&metrics.errors);

        let text = metrics.render();

        assert!(text.contains("cadence_requests_total 1"));
        assert!(text.contains("cadence_errors_total 1"));
    }
}
