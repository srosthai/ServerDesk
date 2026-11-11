//! Bounded results and server-side pagination.
//!
//! Metadata and data browsing return a page at a time. No adapter may
//! materialize an entire table by default, and every page carries enough
//! information for the UI to request the next one without guessing.

use serde::{Deserialize, Serialize};

use super::errors::DatabaseError;

pub const DEFAULT_PAGE_LIMIT: u32 = 100;
pub const MAX_PAGE_LIMIT: u32 = 500;
pub const MAX_PAGE_OFFSET: u64 = 1_000_000;

/// A bounded request for one page. User input is clamped so a command cannot
/// ask for an unbounded result set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub limit: u32,
    pub offset: u64,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_LIMIT,
            offset: 0,
        }
    }
}

impl PageRequest {
    pub fn new(limit: u32, offset: u64) -> Self {
        Self {
            limit: limit.clamp(1, MAX_PAGE_LIMIT),
            offset: offset.min(MAX_PAGE_OFFSET),
        }
    }

    /// Strict constructor for internal callers that should reject rather than
    /// clamp an invalid page request.
    pub fn try_new(limit: u32, offset: u64) -> Result<Self, DatabaseError> {
        if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
            return Err(DatabaseError::validation(format!(
                "A page limit must be between 1 and {MAX_PAGE_LIMIT}."
            )));
        }
        if offset > MAX_PAGE_OFFSET {
            return Err(DatabaseError::validation(format!(
                "A page offset cannot exceed {MAX_PAGE_OFFSET}."
            )));
        }
        Ok(Self { limit, offset })
    }

    pub fn clamped(limit: Option<u32>, offset: Option<u64>) -> Self {
        Self::new(
            limit.unwrap_or(DEFAULT_PAGE_LIMIT),
            offset.unwrap_or_default(),
        )
    }
}

/// A collection that never contains more than its configured bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounded<T> {
    pub items: Vec<T>,
    pub limit: usize,
    pub truncated: bool,
}

impl<T> Bounded<T> {
    pub fn new(mut items: Vec<T>, limit: usize) -> Self {
        let limit = limit.max(1);
        let truncated = items.len() > limit;
        items.truncate(limit);
        Self {
            items,
            limit,
            truncated,
        }
    }

    /// Collects at most `limit + 1` items so the caller can tell whether the
    /// source had more without reading the rest of it.
    pub fn from_iter(items: impl IntoIterator<Item = T>, limit: usize) -> Self {
        let limit = limit.max(1);
        let mut iterator = items.into_iter();
        let mut items = Vec::with_capacity(limit);
        let mut truncated = false;

        for _ in 0..limit {
            let Some(item) = iterator.next() else {
                return Self {
                    items,
                    limit,
                    truncated,
                };
            };
            items.push(item);
        }
        truncated = iterator.next().is_some();

        Self {
            items,
            limit,
            truncated,
        }
    }

    pub fn into_items(self) -> Vec<T> {
        self.items
    }
}

/// One page of results plus the request and optional exact total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page<T> {
    pub items: Vec<T>,
    pub limit: u32,
    pub offset: u64,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl<T> Page<T> {
    pub fn from_items(items: Vec<T>, request: PageRequest, total: Option<u64>) -> Self {
        let bounded = Bounded::new(items, request.limit as usize);
        Self::from_bounded(bounded, request, total)
    }

    pub fn from_bounded(bounded: Bounded<T>, request: PageRequest, total: Option<u64>) -> Self {
        let has_more = bounded.truncated
            || total.is_some_and(|total| {
                request.offset.saturating_add(bounded.items.len() as u64) < total
            });

        Self {
            items: bounded.items,
            limit: request.limit,
            offset: request.offset,
            has_more,
            total,
        }
    }

    pub fn map<U>(self, map: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(map).collect(),
            limit: self.limit,
            offset: self.offset,
            has_more: self.has_more,
            total: self.total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_page_requests_to_the_shared_bounds() {
        assert_eq!(PageRequest::new(0, 0).limit, 1);
        assert_eq!(PageRequest::new(u32::MAX, u64::MAX).limit, MAX_PAGE_LIMIT);
        assert_eq!(PageRequest::new(u32::MAX, u64::MAX).offset, MAX_PAGE_OFFSET);
        assert_eq!(PageRequest::clamped(None, None), PageRequest::default());
    }

    #[test]
    fn strict_page_requests_reject_unbounded_values() {
        assert!(PageRequest::try_new(0, 0).is_err());
        assert!(PageRequest::try_new(MAX_PAGE_LIMIT + 1, 0).is_err());
        assert!(PageRequest::try_new(1, MAX_PAGE_OFFSET + 1).is_err());
        assert!(PageRequest::try_new(MAX_PAGE_LIMIT, MAX_PAGE_OFFSET).is_ok());
    }

    #[test]
    fn bounded_collections_truncate_and_disclose_it() {
        let bounded = Bounded::from_iter(1..=10, 3);

        assert_eq!(bounded.items, [1, 2, 3]);
        assert!(bounded.truncated);

        let exact = Bounded::from_iter(1..=3, 3);
        assert!(!exact.truncated);
    }

    #[test]
    fn pages_report_more_results_from_truncation_or_total() {
        let request = PageRequest::try_new(2, 0).expect("page request");
        let page = Page::from_items(vec![1, 2, 3], request, None);
        assert_eq!(page.items, [1, 2]);
        assert!(page.has_more);

        let page = Page::from_items(vec![1, 2], PageRequest::try_new(2, 2).unwrap(), Some(3));
        assert!(!page.has_more);
        assert_eq!(page.total, Some(3));
    }
}
