//! Periodic heap instrumentation.
//!
//! The number worth watching is not the free total but the **largest free
//! block**: a heap that fragments keeps its total roughly flat while the
//! largest contiguous run shrinks, and the first symptom is an allocation
//! failing with plenty of memory nominally free. ESP-IDF never compacts, so
//! that gap only widens.
//!
//! `minimum_free` is the low-water mark since boot, which catches a transient
//! spike (the WiFi scan, a talk reload) that has already been released by the
//! time the next sample lands.

use std::time::{Duration, Instant};

use esp_idf_svc::sys::{
    heap_caps_get_free_size, heap_caps_get_largest_free_block, heap_caps_get_minimum_free_size,
    MALLOC_CAP_INTERNAL, MALLOC_CAP_SPIRAM,
};
use log::info;

/// How often to sample. Long enough to stay out of the way of the 100 ms main
/// loop, short enough that a leak shows a slope within a talk.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(30);

/// One region's figures, in bytes.
struct Usage {
    free: usize,
    largest_block: usize,
    minimum_free: usize,
}

impl Usage {
    /// # Safety
    /// `caps` must be a valid `MALLOC_CAP_*` mask. The three calls only read
    /// allocator bookkeeping.
    fn sample(caps: u32) -> Self {
        unsafe {
            Self {
                free: heap_caps_get_free_size(caps),
                largest_block: heap_caps_get_largest_free_block(caps),
                minimum_free: heap_caps_get_minimum_free_size(caps),
            }
        }
    }

    /// Free memory that is not in the largest contiguous run - the part an
    /// allocator can only hand out in pieces.
    fn fragmented(&self) -> usize {
        self.free.saturating_sub(self.largest_block)
    }
}

/// Samples the heap on a fixed interval, driven from the main loop.
pub struct HeapProbe {
    last_sample: Instant,
}

impl Default for HeapProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl HeapProbe {
    #[must_use]
    pub fn new() -> Self {
        // Report once at construction rather than waiting out the first
        // interval, so a boot log always carries a baseline.
        let probe = Self {
            last_sample: Instant::now(),
        };
        probe.report();
        probe
    }

    /// Logs a sample if the interval has elapsed. Cheap enough to call from the
    /// main loop's timeout branch on every pass.
    pub fn tick(&mut self) {
        if self.last_sample.elapsed() < SAMPLE_INTERVAL {
            return;
        }
        self.last_sample = Instant::now();
        self.report();
    }

    fn report(&self) {
        let internal = Usage::sample(MALLOC_CAP_INTERNAL);
        info!(
            "🧠 heap internal: free {} B, largest block {} B, fragmented {} B, min free since boot {} B",
            internal.free,
            internal.largest_block,
            internal.fragmented(),
            internal.minimum_free,
        );

        // Zero across the board when SPIRAM is not configured in; logging it
        // unconditionally makes enabling PSRAM visible in the same line.
        let spiram = Usage::sample(MALLOC_CAP_SPIRAM);
        if spiram.free > 0 {
            info!(
                "🧠 heap spiram:   free {} B, largest block {} B, fragmented {} B, min free since boot {} B",
                spiram.free,
                spiram.largest_block,
                spiram.fragmented(),
                spiram.minimum_free,
            );
        }
    }
}
