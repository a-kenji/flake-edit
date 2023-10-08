use std::{fs, path::PathBuf, sync::Mutex};

use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, FmtSubscriber};

// use crate::consts::{LOG_ENV, LOG_PATH};

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

    if let Ok(env_filter) = EnvFilter::try_from_env("RUST_LOG") {
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
    init_logging(Some("/tmp/flake-add/flk-add.log".into()))
}

pub fn log_err<S: Into<String> + std::fmt::Display>(err: S) {
    tracing::error!("Received {}", err.into());
}

pub(crate) fn log_node_enter_info(node: &rowan::NodeOrToken<rnix::SyntaxNode, rnix::SyntaxToken>) {
    tracing::debug!("Start Enter: {node}");
    log_node_debug_info(node);
    tracing::debug!("End Enter: {node}");
}

pub(crate) fn log_node_leave_info(node: &rowan::NodeOrToken<rnix::SyntaxNode, rnix::SyntaxToken>) {
    tracing::debug!("Start Leave: {node}");
    log_node_debug_info(node);
    tracing::debug!("End Leave: {node}");
}

pub(crate) fn log_node_debug_info(node: &rowan::NodeOrToken<rnix::SyntaxNode, rnix::SyntaxToken>) {
    tracing::debug!("Index: {:?}", node.index());
    tracing::debug!("Kind: {:?}", node.kind());
    tracing::debug!("Parent: {:?}", node.parent());
    if let Some(parent) = node.parent() {
        tracing::debug!("Parent Node: {:?}", parent);
        tracing::debug!("Parent Node Kind: {:?}", parent.kind());
    }
    if let Some(node) = node.as_node() {
        tracing::debug!("Green Kind: {:?}", node.green().kind());
        for child in node.children() {
            tracing::debug!("Children: {:?}", child);
            tracing::debug!("Children Kind: {:?}", child.green().kind());
        }
        tracing::debug!("Node Next Sibling: {:?}", node.next_sibling());
        tracing::debug!("Node Prev Sibling: {:?}", node.prev_sibling());
    }
    if let Some(token) = node.as_token() {
        tracing::debug!("Token: {}", token);
    }
    tracing::debug!("Node Index: {}", node.index());
}
