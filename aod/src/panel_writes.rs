use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

/// Atomic generation counter adapted from NEESS-COS-AOD-FOD-FIX to drop stale panel writes.
pub struct PanelWrites {
    gen: AtomicU64,
}

impl PanelWrites {
    pub const fn new() -> Self {
        Self {
            gen: AtomicU64::new(0),
        }
    }

    pub fn invalidate(&self) -> u64 {
        self.gen.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn current(&self) -> u64 {
        self.gen.load(Ordering::SeqCst)
    }

    pub fn apply(&self, expected_gen: u64, path: &str, value: &str) -> bool {
        if self.current() != expected_gen {
            return false;
        }
        match OpenOptions::new().write(true).open(path) {
            Ok(mut f) => {
                if self.current() != expected_gen {
                    return false;
                }
                f.write_all(value.as_bytes()).is_ok()
            }
            Err(_) => false,
        }
    }
}
