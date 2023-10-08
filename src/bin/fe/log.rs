use std::{fs, path::PathBuf, sync::Mutex};

use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, FmtSubscriber};

const LOG_ENV: &str = "FE_LOG";

/// Configuration of logging
pub fn init_logging(log_file: Option<PathBuf>) -> Result<(), std::io::Error> {
    let log_file = match log_file {
        Some(path) => {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            Some(fs::File::create(path)?)
        }
        None => None,
    };

    let subscriber = FmtSubscriber::builder()
        // all spans/events with a level higher than TRACE (e.g, debug, info, warn, etc.)
        // will be written to output path.
        .with_max_level(Level::TRACE)
        .with_writer(Mutex::new(log_file.unwrap()))
        .with_thread_ids(true)
        .with_ansi(true)
        .with_line_number(true);

    if let Ok(env_filter) = EnvFilter::try_from_env(LOG_ENV) {
        let subscriber = subscriber.with_env_filter(env_filter).finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    } else {
        let subscriber = subscriber.finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("setting default subscriber failed");
    }

    Ok(())
}

pub fn init() -> Result<(), std::io::Error> {
    init_logging(Some("/tmp/fe/fe.log".into()))
}
