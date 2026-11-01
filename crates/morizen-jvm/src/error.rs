use thiserror::Error;

pub type Result<T> = std::result::Result<T, BridgeError>;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("invalid Java-mod configuration: {0}")]
    Config(String),
    #[error("could not discover JDK 25: {0}")]
    Jdk(String),
    #[error("could not start the embedded JVM: {0}")]
    JvmStart(String),
    #[error("JNI operation failed: {0}")]
    Jni(String),
    #[error("Java exception {class}: {message}\n{stack}")]
    JavaException {
        class: String,
        message: String,
        stack: String,
    },
    #[error("Java mod thread failed: {0}")]
    Thread(String),
    #[error("compatibility report failed: {0}")]
    Report(String),
}

impl From<jni::errors::Error> for BridgeError {
    fn from(error: jni::errors::Error) -> Self {
        match error {
            jni::errors::Error::CaughtJavaException {
                name, msg, stack, ..
            } => Self::JavaException {
                class: name,
                message: msg,
                stack,
            },
            other => Self::Jni(other.to_string()),
        }
    }
}
