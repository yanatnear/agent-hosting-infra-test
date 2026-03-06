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
