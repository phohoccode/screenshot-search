pub mod hybrid;
pub mod normalize;
pub mod query;

#[cfg(test)]
pub mod search_tests;

pub use hybrid::search_hybrid;
pub use normalize::{normalize_search_query, normalize_search_text};
pub use query::{
    build_safe_fts_query, search_screenshots, SearchRequest, SearchResultItem, SearchResultPage,
};
