//! Lanes: how many of each kind of work run at once, across every run.
//!
//! A drive reads one thing at a time; encoding uses every core; each
//! catalogue gets one request at a time; each destination one copy at a
//! time. Two drives are two runs sharing one `Lanes`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LaneKind {
    /// The run's own drive.
    Drive,
    Encode,
    Network(String),
    /// Copies into this folder.
    File(String),
}

pub struct Lanes {
    encode: Arc<Semaphore>,
    keyed: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl Default for Lanes {
    fn default() -> Self {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        Lanes::with_encoders(cores)
    }
}

impl Lanes {
    pub fn with_encoders(n: usize) -> Self {
        Lanes { encode: Arc::new(Semaphore::new(n.max(1))), keyed: Mutex::default() }
    }

    /// Waits for room in a lane. `drive` is the run's drive.
    pub async fn enter(&self, lane: &LaneKind, drive: &str) -> OwnedSemaphorePermit {
        let sem = match lane {
            LaneKind::Encode => self.encode.clone(),
            LaneKind::Drive => self.keyed(format!("drive:{drive}")),
            LaneKind::Network(s) => self.keyed(format!("net:{s}")),
            LaneKind::File(root) => self.keyed(format!("file:{root}")),
        };
        sem.acquire_owned().await.expect("lanes are never closed")
    }

    fn keyed(&self, key: String) -> Arc<Semaphore> {
        self.keyed.lock().unwrap().entry(key).or_insert_with(|| Arc::new(Semaphore::new(1))).clone()
    }
}
