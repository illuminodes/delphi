use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use signal_hook::consts::{SIGINT, SIGTERM};

/// Sets the daemon's stop flag on SIGTERM or SIGINT, so systemd's stop and
/// Ctrl-C both end the hub loop and let every worker finish its write.
pub struct ShutdownSignal;

impl ShutdownSignal {
    /// # Errors
    /// Returns an I/O error if a handler cannot be installed.
    pub fn arm(stop: &Arc<AtomicBool>) -> io::Result<()> {
        for signal in [SIGTERM, SIGINT] {
            signal_hook::flag::register(signal, Arc::clone(stop))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use signal_hook::consts::SIGTERM;

    use super::ShutdownSignal;

    #[test]
    fn sigterm_sets_the_stop_flag() {
        let stop = Arc::new(AtomicBool::new(false));
        ShutdownSignal::arm(&stop).expect("arm");

        signal_hook::low_level::raise(SIGTERM).expect("raise");

        assert!(stop.load(Ordering::Relaxed));
    }
}
