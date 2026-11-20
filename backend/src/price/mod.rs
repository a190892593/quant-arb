//! 标价与价格源。

pub mod oracle;

pub use oracle::{Marks, chainlink_feeds, derive_pool_marks, merge, parse_chainlink, stable_marks};
