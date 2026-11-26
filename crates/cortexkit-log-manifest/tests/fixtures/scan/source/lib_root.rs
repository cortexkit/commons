mod disk;
mod plain;
#[path = "odd/wire.rs"]
mod renamed;
#[cfg(test)]
mod not_present;
#[cfg(feature = "extra")]
mod inline {
    fn run() { tracing::info!("inline"); }
}

fn run() {
    tracing::info!(n_bytes, has_file = %v, path = ?v, "state" = ?v, a.b = v, %self.session_id, "merged");
    warn!(n_bytes = v, "state" = v, ok = v, "merged");
    ::tracing::event!(name: "named", target: "fixed.target", parent: None, tracing::Level::INFO, { ?elapsed_ms = v, %path, ?ok, "path" = %v }, r"escaped {{braces}} \ one");
    event!(level, "variable level");
    debug!("formatted {}", value);
    debug!("capture {value}");
    warn!("[ambiguous]");
    trace!(target: TARGET, "unknown target");
    trace!(target: "", "invalid target");
    trace!("new\nline");
    trace!(message = "manual");
    trace!(n_bytes = 1);
    macro_rules! defer {
        () => { error!("inside definition"); }
    }
    tokio::select! {
        _ = wake() => { tracing::warn!("inside invocation"); }
    }
    custom::warn!("other qualifier");
    warn!("secret-shaped field",);
    error!("n_0123456789abcdef0123456789abcdef" = v, a.n_bytes = v, %self.session_id, "filtered field");
    #[cfg(test)]
    error!("test statement");
    #[cfg(test,)]
    {
        error!("test block");
    }
}

#[cfg(test)]
fn test_only() { info!("test item"); }
