//! Test-only row construction probe. Production rows call it under `cfg(test)`; no
//! production build carries the counter.

use std::time::Instant;

thread_local! {
    pub(in crate::app) static ROW_COST: std::cell::Cell<(u64, u128)> = const { std::cell::Cell::new((0, 0)) };
}

pub(in crate::app) struct RowTimer(Instant);

impl RowTimer {
    pub(in crate::app) fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for RowTimer {
    fn drop(&mut self) {
        ROW_COST.with(|cost| {
            let (count, nanos) = cost.get();
            cost.set((count + 1, nanos + self.0.elapsed().as_nanos()));
        });
    }
}
