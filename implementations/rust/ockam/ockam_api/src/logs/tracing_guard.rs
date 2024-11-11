use opentelemetry::global;
use opentelemetry_sdk::logs::LoggerProvider;
use tracing_appender::non_blocking::WorkerGuard;

/// The Tracing guard contains a guard closing the logging appender
/// and optionally the logger/tracer providers which can be used to force the flushing
/// of spans and log records
#[derive(Debug)]
pub struct TracingGuard {
    _worker_guard: Option<WorkerGuard>,
    logger_provider: Option<LoggerProvider>,
}

impl TracingGuard {
    /// Create a new tracing guard
    pub fn new(worker_guard: WorkerGuard, logger_provider: LoggerProvider) -> TracingGuard {
        TracingGuard {
            _worker_guard: Some(worker_guard),
            logger_provider: Some(logger_provider),
        }
    }

    /// Create a Tracing guard when distributed tracing is deactivated
    pub fn guard_only(worker_guard: WorkerGuard) -> TracingGuard {
        TracingGuard {
            _worker_guard: Some(worker_guard),
            logger_provider: None,
        }
    }

    pub fn shutdown(&self) {
        global::shutdown_tracer_provider();
    }

    /// Export the current batches of spans and log records
    /// This is used right after a background node has started to get the first logs
    /// and in tests otherwise
    pub fn force_flush(&self) {
        if let Some(logger_provider) = self.logger_provider.as_ref() {
            logger_provider.force_flush();
        }
    }
}
