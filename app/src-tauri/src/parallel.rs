//! Bounded fan-out for blocking network lookups (§7.9 fast preview).

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `f` over `items` on at most `limit` scoped threads, results in input
/// order. One item (or `limit` 1) runs inline without spawning.
pub(crate) fn bounded_map<T: Sync, R: Send>(
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
    use std::time::Duration;

    #[test]
    fn keeps_order_and_bounds_concurrency() {
        let running = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let items: Vec<usize> = (0..10).collect();
        let out = bounded_map(&items, 3, |&i| {
            let now = running.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            running.fetch_sub(1, Ordering::SeqCst);
            i * 2
        });
        assert_eq!(out, (0..10).map(|i| i * 2).collect::<Vec<_>>());
        let peak = peak.load(Ordering::SeqCst);
        assert!(peak > 1 && peak <= 3, "peak {peak}");
    }
}
