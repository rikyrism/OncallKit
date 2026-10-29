use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Deserialize)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl PageQuery {
    pub fn limit_offset(&self) -> (i64, i64) {
        (self.limit.unwrap_or(50).clamp(1, 200), self.offset.unwrap_or(0).max(0))
    }
}

/// A page of results. Queries fetch `limit + 1` rows so `has_more` needs no extra count query.
#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub limit: i64,
    pub offset: i64,
    pub has_more: bool,
}

impl<T> Page<T> {
    pub fn new(mut items: Vec<T>, limit: i64, offset: i64) -> Self {
        let has_more = items.len() as i64 > limit;
        items.truncate(limit as usize);
        Page { items, limit, offset, has_more }
    }
}
