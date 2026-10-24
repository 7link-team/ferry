//! 41 timestamp / 10 worker / 12 sequence bits; IDs cross JSON as strings.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    fmt,
    str::FromStr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const EPOCH: i64 = 1_767_225_600_000;
pub const TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Id(i64);
impl From<i64> for Id {
    fn from(value: i64) -> Self {
        Self(value)
    }
}
impl From<Id> for i64 {
    fn from(value: Id) -> Self {
        value.0
    }
}
impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl FromStr for Id {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse().map(Self)
    }
}
impl Serialize for Id {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for Id {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Input {
            Text(String),
            Number(i64),
        }
        match Input::deserialize(d)? {
            Input::Text(s) => s.parse().map_err(serde::de::Error::custom),
            Input::Number(n) => Ok(Self(n)),
        }
    }
}

struct State {
    last_ms: i64,
    sequence: u16,
    heartbeat: Option<Instant>,
}
pub struct Generator {
    worker: u16,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    state: Mutex<State>,
}
impl Generator {
    pub fn new(worker: u16, heartbeat: Instant) -> anyhow::Result<Self> {
        Self::with_clock(worker, heartbeat, Arc::new(crate::db::now))
    }
    fn with_clock(
        worker: u16,
        heartbeat: Instant,
        clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(worker < 1024, "worker id must be in 0..1023");
        Ok(Self {
            worker,
            clock,
            state: Mutex::new(State {
                last_ms: -1,
                sequence: 0,
                heartbeat: Some(heartbeat),
            }),
        })
    }
    pub fn heartbeat(&self, started: Instant) {
        self.state
            .lock()
            .expect("generator mutex poisoned")
            .heartbeat = Some(started);
    }
    pub fn stop(&self) {
        self.state
            .lock()
            .expect("generator mutex poisoned")
            .heartbeat = None;
    }
    pub fn next_id(&self) -> anyhow::Result<Id> {
        loop {
            // Release between waits so a successful heartbeat can restore the lease.
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("generator mutex poisoned"))?;
            anyhow::ensure!(
                state.heartbeat.is_some_and(|t| t.elapsed() <= TTL),
                "worker heartbeat expired; refusing to issue ID"
            );
            let now = (self.clock)() - EPOCH;
            anyhow::ensure!(
                (0..(1_i64 << 41)).contains(&now),
                "clock outside snowflake epoch range"
            );
            anyhow::ensure!(
                state.last_ms - now <= 5000,
                "clock rolled back more than 5 seconds"
            );
            if now < state.last_ms || (now == state.last_ms && state.sequence == 4095) {
                drop(state);
                std::thread::yield_now();
                continue;
            }
            state.sequence = if now == state.last_ms {
                state.sequence + 1
            } else {
                0
            };
            state.last_ms = now;
            return Ok(Id((now << 22)
                | ((self.worker as i64) << 12)
                | state.sequence as i64));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::HashSet,
        sync::atomic::{AtomicI64, Ordering},
    };
    fn controlled() -> (Arc<Generator>, Arc<AtomicI64>) {
        let time = Arc::new(AtomicI64::new(EPOCH + 10_000_000_000));
        let clock = time.clone();
        (
            Arc::new(
                Generator::with_clock(
                    37,
                    Instant::now(),
                    Arc::new(move || clock.load(Ordering::SeqCst)),
                )
                .unwrap(),
            ),
            time,
        )
    }
    #[test]
    fn monotonic_and_unique_100000() {
        let g = Generator::new(1, Instant::now()).unwrap();
        let ids: Vec<_> = (0..100_000).map(|_| g.next_id().unwrap()).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]));
    }
    #[test]
    fn concurrent_100000_unique() {
        let g = Arc::new(Generator::new(2, Instant::now()).unwrap());
        let threads: Vec<_> = (0..10)
            .map(|_| {
                let g = g.clone();
                std::thread::spawn(move || {
                    (0..10_000)
                        .map(|_| g.next_id().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: HashSet<_> = threads
            .into_iter()
            .flat_map(|t| t.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 100_000);
    }
    /// Acceptance check: shows the failure this design exists to prevent.
    /// If an id were emitted as a JSON number, the browser would parse it as
    /// an f64 and silently hand back a different id — no error anywhere.
    #[test]
    fn a_json_number_would_corrupt_an_id_which_is_why_it_is_a_string() {
        // A snowflake minted one year into the epoch, worker 37, sequence 1.
        // Ids pass f64's exact range about 25 days after the epoch, so every
        // id this product ever issues in production is past it.
        let year_ms: i64 = 365 * 86_400_000;
        let value: i64 = (year_ms << 22) + (37 << 12) + 1;
        assert!(
            value > (1 << 53),
            "the premise: ids exceed f64's exact range"
        );

        // What a browser does with a JSON number.
        let through_f64 = value as f64 as i64;
        assert_ne!(through_f64, value, "f64 must lose this value");

        // What this type actually emits, and what survives the round trip.
        let id = Id::from(value);
        assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{value}\""));
        assert_eq!(
            serde_json::from_str::<Id>(&format!("\"{value}\"")).unwrap(),
            id
        );
    }

    #[test]
    fn bits_json_and_conversions() {
        let (g, clock) = controlled();
        for sequence in 0..3 {
            let id = g.next_id().unwrap();
            let value = i64::from(id);
            assert_eq!((value >> 22) + EPOCH, clock.load(Ordering::SeqCst));
            assert_eq!((value >> 12) & 1023, 37);
            assert_eq!(value & 4095, sequence);
            assert!(value > (1 << 53));
            let json = serde_json::to_string(&id).unwrap();
            assert_eq!(json, format!("\"{value}\""));
            assert_eq!(serde_json::from_str::<Id>(&json).unwrap(), id);
            assert_eq!(serde_json::from_str::<Id>(&value.to_string()).unwrap(), id);
            assert_eq!(Id::from(value).to_string().parse::<Id>().unwrap(), id);
        }
        assert!(serde_json::from_str::<Id>("1.5").is_err());
        assert!(Generator::new(1024, Instant::now()).is_err());
    }
    #[test]
    fn rollback_waits_and_large_rollback_fails() {
        let (g, clock) = controlled();
        let first = g.next_id().unwrap();
        clock.fetch_sub(1000, Ordering::SeqCst);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = g.clone();
        let thread = std::thread::spawn(move || tx.send(worker.next_id()).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        clock.fetch_add(1000, Ordering::SeqCst);
        assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap() > first);
        thread.join().unwrap();
        clock.fetch_sub(10_000, Ordering::SeqCst);
        assert!(g.next_id().unwrap_err().to_string().contains("rolled back"));
    }
    #[test]
    fn sequence_exhaustion_waits_without_borrowing() {
        let (g, clock) = controlled();
        for sequence in 0..4096 {
            assert_eq!(i64::from(g.next_id().unwrap()) & 4095, sequence);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = g.clone();
        let thread = std::thread::spawn(move || tx.send(worker.next_id()).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(30)).is_err());
        clock.fetch_add(1, Ordering::SeqCst);
        let value = i64::from(rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap());
        assert_eq!(value & 4095, 0);
        assert_eq!((value >> 22) + EPOCH, clock.load(Ordering::SeqCst));
        thread.join().unwrap();
    }
    #[test]
    fn expired_heartbeat_stops_ids_and_recovers() {
        let (g, _) = controlled();
        let first = g.next_id().unwrap();
        g.heartbeat(Instant::now() - TTL - Duration::from_secs(1));
        assert!(
            g.next_id()
                .unwrap_err()
                .to_string()
                .contains("heartbeat expired")
        );
        g.heartbeat(Instant::now());
        assert!(g.next_id().unwrap() > first);
        g.stop();
        assert!(g.next_id().is_err());
    }
    #[test]
    fn lease_is_checked_while_waiting() {
        let (g, clock) = controlled();
        g.next_id().unwrap();
        clock.fetch_sub(1000, Ordering::SeqCst);
        g.heartbeat(Instant::now() - TTL + Duration::from_millis(20));
        assert!(
            g.next_id()
                .unwrap_err()
                .to_string()
                .contains("heartbeat expired")
        );
    }
}
