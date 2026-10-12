//! Connecting to Mongo, and making each collection's indexes once.

use std::future::Future;

use futures::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};
use mongodb::{Client, Collection, Database, IndexModel, options::IndexOptions};
use teeny_core::config::StoreConfig;
use tokio::sync::OnceCell;

use crate::Result;

/// Opens the database the configuration names. The driver connects lazily,
/// so this does not wait for the server.
pub async fn connect(config: &StoreConfig) -> Result<Database> {
    let client = Client::with_uri_str(&config.mongo_uri).await?;
    Ok(client.database(&config.mongo_database))
}

/// A collection's indexes, made on first use as the TypeScript service's
/// `ready()` did, and not again.
#[derive(Debug, Default)]
pub(crate) struct Indexes(OnceCell<()>);

impl Indexes {
    pub(crate) async fn ensure<F, Fut>(&self, make: F) -> Result<()>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        self.0.get_or_try_init(make).await.map(|_| ())
    }
}

/// An index on `keys`.
pub(crate) fn index(keys: Document) -> IndexModel {
    IndexModel::builder().keys(keys).build()
}

/// A unique index on `keys`.
pub(crate) fn unique(keys: Document) -> IndexModel {
    IndexModel::builder()
        .keys(keys)
        .options(IndexOptions::builder().unique(true).build())
        .build()
}

/// `{ $group: { _id: "$<field>", total: { $sum: 1 } } }`, as pairs.
pub(crate) async fn group_counts(
    rows: &Collection<Document>,
    field: &str,
) -> Result<Vec<(Bson, u64)>> {
    let mut cursor = rows
        .aggregate([doc! { "$group": { "_id": format!("${field}"), "total": { "$sum": 1 } } }])
        .await?;
    let mut counts = Vec::new();
    while let Some(row) = cursor.try_next().await? {
        let total = match row.get("total") {
            Some(Bson::Int32(n)) => u64::try_from(*n).unwrap_or_default(),
            Some(Bson::Int64(n)) => u64::try_from(*n).unwrap_or_default(),
            Some(Bson::Double(n)) => *n as u64,
            _ => 0,
        };
        counts.push((row.get("_id").cloned().unwrap_or(Bson::Null), total));
    }
    Ok(counts)
}

#[cfg(test)]
pub(crate) mod testing {
    //! A scratch database for the Mongo-backed tests, which run only when
    //! `TEENY_TEST_MONGO_URI` names a server.

    use mongodb::{Client, Database};

    pub(crate) async fn database() -> Option<Database> {
        let uri = std::env::var("TEENY_TEST_MONGO_URI").ok()?;
        let client = Client::with_uri_str(&uri)
            .await
            .expect("TEENY_TEST_MONGO_URI connects");
        Some(client.database("teeny_store_test"))
    }

    /// A collection name no other run uses.
    pub(crate) fn collection(prefix: &str) -> String {
        format!("{prefix}_{}", crate::support::random_u64())
    }
}
