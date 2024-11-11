use opentelemetry::global;
use std::io::stdout;
use tracing_appender::non_blocking::NonBlocking;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_core::Subscriber;
use tracing_subscriber::fmt::format::{DefaultFields, Format};
use tracing_subscriber::fmt::layer;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, layer::SubscriberExt, registry};

use crate::logs::tracing_guard::TracingGuard;
use crate::logs::LogFormat;
use crate::logs::{GlobalErrorHandler, LoggingConfiguration};

pub struct LoggingTracing;

impl LoggingTracing {
    /// Setup logging and tracing
    /// The app name is used to set an attribute on all events specifying if the event
    /// has been created by the cli or by a local node.
    ///
    /// The TracingGuard is used to flush all events when dropped.
    pub fn setup(logging_configuration: &LoggingConfiguration) -> TracingGuard {
        Self::setup_local_logging_only(logging_configuration)
    }

    /// Setup logging to the console or to a file
    pub fn setup_local_logging_only(logging_configuration: &LoggingConfiguration) -> TracingGuard {
        let (appender, worker_guard) = make_logging_appender(logging_configuration);
        if logging_configuration.is_enabled() {
            let layers = registry().with(logging_configuration.env_filter());
            let result = match logging_configuration.format() {
                LogFormat::Pretty => layers.with(appender.pretty()).try_init(),
                LogFormat::Json => layers.with(appender.json()).try_init(),
                LogFormat::Default => layers.with(appender).try_init(),
            };
            result.expect("Failed to initialize tracing subscriber");
        };

        // the global error handler prints errors when exporting spans or log records fails
        set_global_error_handler(logging_configuration);

        TracingGuard::guard_only(worker_guard)
    }
}

/// Return either a console or a file appender for log messages
fn make_logging_appender<S>(
    logging_configuration: &LoggingConfiguration,
) -> (
    fmt::Layer<S, DefaultFields, Format, NonBlocking>,
    WorkerGuard,
)
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let layer = layer().with_ansi(logging_configuration.is_colored());
    let (writer, guard) = match logging_configuration.log_dir() {
        // If a node dir path is not provided, log to stdout.
        None => tracing_appender::non_blocking(stdout()),
        // If a log directory is provided, log to a rolling file appender.
        Some(log_dir) => {
            let r = RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .max_log_files(logging_configuration.max_files() as usize)
                .filename_prefix("stdout")
                .filename_suffix("log")
                .build(log_dir)
                .expect("Failed to create rolling file appender");
            tracing_appender::non_blocking(r)
        }
    };
    (layer.with_writer(writer), guard)
}

/// Set a global error handler to report logging/tracing errors.
/// They are either:
///
///  - printed on the console
///  - logged to a log file
///  - not printed at all
///
fn set_global_error_handler(logging_configuration: &LoggingConfiguration) {
    if let Err(e) = match logging_configuration.global_error_handler() {
        GlobalErrorHandler::Off => global::set_error_handler(|_| ()).map_err(|e| format!("{e:?}")),
        GlobalErrorHandler::Console => global::set_error_handler(|e| println!("{e}"))
            .map_err(|e| format!("logging error: {e:?}")),
        GlobalErrorHandler::LogFile => match logging_configuration.log_dir() {
            Some(log_dir) => {
                use flexi_logger::*;
                let file_spec = FileSpec::default()
                    .directory(log_dir)
                    .basename("logging_tracing_errors");
                match Logger::try_with_str("info") {
                    Ok(logger) => {
                        // make sure that the log file is rolled every 3 days to avoid
                        // accumulating error messages
                        match logger
                            .log_to_file(file_spec)
                            .append()
                            .rotate(
                                Criterion::Age(Age::Day),
                                Naming::Timestamps,
                                Cleanup::KeepLogFiles(3),
                            )
                            .build()
                        {
                            Ok((log, _logger_handle)) => global::set_error_handler(move |e| {
                                log.log(
                                    &Record::builder()
                                        .level(Level::Error)
                                        .module_path(Some("ockam_api::logs::setup"))
                                        .args(format_args!("{e:?}"))
                                        .build(),
                                )
                            })
                            .map_err(|e| format!("{e:?}")),
                            Err(e) => Err(format!("{e:?}")),
                        }
                    }
                    Err(e) => Err(format!("{e:?}")),
                }
            }
            None => {
                global::set_error_handler(|e| println!("ERROR! {e}")).map_err(|e| format!("{e:?}"))
            }
        },
    } {
        println!("cannot set a global error handler for logging: {e}");
    };
}
