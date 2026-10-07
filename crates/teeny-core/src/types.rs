//! Identifiers shared across crates.
//!
//! X ids are 64-bit numbers that JSON clients mangle, so X sends them as
//! strings and so do we. Each kind is its own type so a post id cannot be
//! passed where a user id belongs.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// The id as X spells it.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_owned())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id! {
    /// An X post (a tweet), by id.
    PostId
}

string_id! {
    /// An X account, by numeric id (not the handle, which can change).
    UserId
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
