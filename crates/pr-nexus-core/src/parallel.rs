//! Bounded fan-out for blocking lookups (SPEC §6: at most 4 in flight per host).

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `f` over `items` on at most `limit` scoped threads, results in input
/// order. One item (or `limit` 1) runs inline without spawning.
pub fn bounded_map<T: Sync, R: Send>(
    items: &[T],
    limit: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let workers = limit.max(1).min(items.len());
    if workers <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(index) else { break };
                    let result = f(item);
                    *slots[index]
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(result);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .expect("every slot is filled")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    #[test]
    fn keeps_order_and_bounds_concurrency() {
        let peak = AtomicUsize::new(0);
        let gate = Mutex::new(0u32);
        let cv = Condvar::new();
        let items: Vec<usize> = (0..6).collect();
        let out = bounded_map(&items, 3, |&i| {
            let mut in_flight = gate.lock().unwrap();
            *in_flight += 1;
            peak.fetch_max(*in_flight as usize, Ordering::SeqCst);
            if *in_flight >= 2 {
                cv.notify_all();
            }
            let started = Instant::now();
            while *in_flight < 2 && started.elapsed() < Duration::from_secs(5) {
                let (guard, timeout) = cv.wait_timeout(in_flight, Duration::from_secs(5)).unwrap();
                in_flight = guard;
                if timeout.timed_out() {
                    break;
                }
            }
            *in_flight = in_flight.saturating_sub(1);
            drop(in_flight);
            i * 2
        });
        assert_eq!(out, (0..6).map(|i| i * 2).collect::<Vec<_>>());
        let peak = peak.load(Ordering::SeqCst);
        assert!(peak > 1 && peak <= 3, "peak {peak}");
    }
}
