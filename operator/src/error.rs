use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Missing object key: {0}")]
    MissingObjectKey(&'static str),

    #[error("Invalid agent spec: {0}")]
    InvalidSpec(String),

    #[error("Finalizer error: {0}")]
    Finalizer(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn metric_label(&self) -> String {
        match self {
            Error::Kube(_) => "kube_error".to_string(),
            Error::Serialization(_) => "serialization_error".to_string(),
            Error::MissingObjectKey(_) => "missing_key_error".to_string(),
            Error::InvalidSpec(_) => "invalid_spec_error".to_string(),
            Error::Finalizer(_) => "finalizer_error".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Test Case: General quality — metric labels for Prometheus observability**
    ///
    /// WHY THIS MATTERS:
    /// The operator exposes error metrics to Prometheus for alerting and dashboards.
    /// Each error variant must produce a stable, unique label string so that alert
    /// rules and Grafana queries don't silently break when error types change.
    ///
    /// WHAT THIS TEST DOES:
    /// 1. Constructs one instance of every Error variant
    /// 2. Calls metric_label() on each
    /// 3. Asserts the returned string matches the expected Prometheus label
    ///
    /// IF THIS FAILS:
    /// A metric label changed or a new variant was added without a corresponding
    /// label — Prometheus alert rules referencing the old label will stop matching.
    ///
    /// WHAT IS BEING TESTED:
    /// The Error::metric_label() method — a pure function with no dependencies.
    #[test]
    fn p2_metric_labels_match_expected_prometheus_strings() {
        // Arrange — construct one of each variant using minimal valid data
        let serde_err = serde_json::from_str::<()>("invalid").unwrap_err();
        let kube_err = kube::Error::Api(kube::error::ErrorResponse {
            code: 500,
            message: "test".into(),
            reason: "test".into(),
            status: "Failure".into(),
        });

        let cases: Vec<(Error, &str)> = vec![
            (Error::Kube(kube_err), "kube_error"),
            (Error::Serialization(serde_err), "serialization_error"),
            (Error::MissingObjectKey("test_key"), "missing_key_error"),
            (Error::InvalidSpec("bad spec".into()), "invalid_spec_error"),
            (Error::Finalizer("cleanup failed".into()), "finalizer_error"),
        ];

        for (error, expected_label) in cases {
            // Act
            let label = error.metric_label();

            // Assert
            assert_eq!(
                label, expected_label,
                "Error variant {:?} must produce metric label '{}'",
                error, expected_label
            );
        }
    }
}
