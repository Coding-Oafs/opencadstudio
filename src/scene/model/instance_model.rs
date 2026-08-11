use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SOURCE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderInstance {
    pub source_id: u64,
    pub translation: [f64; 3],
}

pub fn next_source_id() -> u64 {
    NEXT_SOURCE_ID.fetch_add(1, Ordering::Relaxed)
}
