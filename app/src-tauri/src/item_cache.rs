//! Built pull-request list rows, reused until the store revision moves.
//!
//! A sync poll that did not write, and every extra `list_page` in the same
//! revision (counts, paging, search), shares one build.

use std::sync::{Arc, Mutex, PoisonError};

use chrono::{DateTime, Utc};

use crate::listing::Item;

struct Entry {
    /// Address of the [`pr_nexus_core::store::Store`] the rows were read from.
    store: usize,
    revision: u64,
    /// Next time-based snooze expiry. `None` when no snooze will end by the clock.
    fresh_until: Option<DateTime<Utc>>,
    items: Arc<Vec<Item>>,
}

/// One cached `list_items` result for the active profile.
#[derive(Default)]
pub struct ListItemCache {
    entry: Mutex<Option<Entry>>,
}

impl ListItemCache {
    pub fn get(
        &self,
        store: usize,
        revision: u64,
        fresh_until: Option<DateTime<Utc>>,
    ) -> Option<Arc<Vec<Item>>> {
        let guard = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = guard.as_ref()?;
        if entry.store == store && entry.revision == revision && entry.fresh_until == fresh_until {
            Some(Arc::clone(&entry.items))
        } else {
            None
        }
    }

    pub fn put(
        &self,
        store: usize,
        revision: u64,
        fresh_until: Option<DateTime<Utc>>,
        items: Arc<Vec<Item>>,
    ) {
        let mut guard = self.entry.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = Some(Entry {
            store,
            revision,
            fresh_until,
            items,
        });
    }

    pub fn clear(&self) {
        *self.entry.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}
