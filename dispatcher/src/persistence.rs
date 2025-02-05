use chrono::prelude::*;
use log::error;
use postgres::tls::{MakeTlsConnect, TlsConnect};
use r2d2_postgres::PostgresConnectionManager;
use tokio_postgres::Socket;

use crate::base_types::FileInfo;

#[derive(thiserror::Error, Debug)]
pub enum PersistenceError {
    #[error("{message}: {source}")]
    DatabaseConnection {
        source: r2d2::Error,
        message: String,
    },
    #[error("{message}: {source}")]
    DatabasePool {
        source: bb8::RunError<tokio_postgres::Error>,
        message: String,
    },
    #[error("{message}: {source}")]
    Query {
        source: tokio_postgres::Error,
        message: String,
    },
    #[error("{message}")]
    Logical { message: String },
}

pub trait Persistence {
    fn delete_sftp_download_file(&self, id: i64) -> Result<(), PersistenceError>;
    fn set_sftp_download_file(&self, id: i64, file_id: i64) -> Result<(), PersistenceError>;
    fn insert_file(
        &self,
        source: &str,
        path: &str,
        modified: &DateTime<Utc>,
        size: i64,
        hash: Option<String>,
    ) -> Result<i64, PersistenceError>;
    fn get_file(&self, source: &str, path: &str) -> Result<Option<FileInfo>, PersistenceError>;
}

#[derive(Clone)]
pub struct PostgresPersistence<T>
where
    T: MakeTlsConnect<Socket>
        + Clone
        + 'static
        + Sync
        + Send
        + postgres::tls::MakeTlsConnect<postgres::Socket>,
    T::TlsConnect: Send,
    T::Stream: Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    conn_pool: r2d2::Pool<PostgresConnectionManager<T>>,
}

impl<T> PostgresPersistence<T>
where
    T: MakeTlsConnect<Socket>
        + Clone
        + 'static
        + Sync
        + Send
        + postgres::tls::MakeTlsConnect<postgres::Socket>,
    T::TlsConnect: Send,
    T::Stream: Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    pub fn new(
        connection_manager: PostgresConnectionManager<T>,
    ) -> Result<PostgresPersistence<T>, String> {
        let pool = r2d2::Pool::new(connection_manager)
            .map_err(|e| format!("Error connecting to database: {}", e))?;

        Ok(PostgresPersistence { conn_pool: pool })
    }
}

impl<T> Persistence for PostgresPersistence<T>
where
    T: MakeTlsConnect<Socket> + Clone + 'static + Sync + Send,
    T::TlsConnect: Send,
    T::Stream: Send,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    fn set_sftp_download_file(&self, id: i64, file_id: i64) -> Result<(), PersistenceError> {
        let mut client = self.conn_pool.get().unwrap();

        let execute_result = client.execute(
            "update dispatcher.sftp_download set file_id = $2 where id = $1",
            &[&id, &file_id],
        );

        match execute_result {
            Ok(_) => Ok(()),
            Err(e) => Err(PersistenceError::Query {
                source: e,
                message: String::from("Error updating sftp_download record into database"),
            }),
        }
    }

    fn delete_sftp_download_file(&self, id: i64) -> Result<(), PersistenceError> {
        let mut client = self.conn_pool.get().unwrap();

        let execute_result =
            client.execute("delete from dispatcher.sftp_download where id = $1", &[&id]);

        match execute_result {
            Ok(_) => Ok(()),
            Err(e) => Err(PersistenceError::Query {
                source: e,
                message: String::from("Error deleting sftp_download record from database"),
            }),
        }
    }

    fn insert_file(
        &self,
        source: &str,
        path: &str,
        modified: &DateTime<Utc>,
        size: i64,
        hash: Option<String>,
    ) -> Result<i64, PersistenceError> {
        let mut client = self.conn_pool.get().unwrap();

        let insert_result = client.query_one(
            concat!(
                "insert into dispatcher.file (source, path, modified, size, hash) ",
                "values ($1, $2, $3, $4, $5) ",
                "on conflict (source, path) do update ",
                "set modified=EXCLUDED.modified, size=EXCLUDED.size, hash=EXCLUDED.hash ",
                "returning id",
            ),
            &[&source, &path, &modified, &size, &hash],
        );

        match insert_result {
            Ok(row) => Ok(row.get(0)),
            Err(e) => Err(PersistenceError::Query {
                source: e,
                message: String::from("Error inserting file record into database"),
            }),
        }
    }

    fn get_file(&self, source: &str, path: &str) -> Result<Option<FileInfo>, PersistenceError> {
        let mut client =
            self.conn_pool
                .get()
                .map_err(|e| PersistenceError::DatabaseConnection {
                    source: e,
                    message: "Could not get database connection".to_string(),
                })?;

        let rows = client.query(
            "select source, path, modified, size, hash from dispatcher.file where source = $1 and path = $2",
            &[&source, &path]
        ).map_err(|e| PersistenceError::Query {
            source: e,
            message: String::from("Error reading file record from database"),
        })?;

        if rows.is_empty() {
            Ok(None)
        } else if rows.len() == 1 {
            let row = &rows[0];

            Ok(Some(FileInfo {
                modified: row.get(2),
                size: row.get(3),
                hash: row.get(4),
            }))
        } else {
            Err(PersistenceError::Logical {
                message: String::from("More than one file matching criteria"),
            })
        }
    }
}

#[derive(Clone)]
pub struct PostgresAsyncPersistence<T>
where
    T: MakeTlsConnect<Socket> + Clone + 'static + Sync + Send,
    T::TlsConnect: Send,
    T::Stream: Send + Sync,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    conn_pool: bb8::Pool<bb8_postgres::PostgresConnectionManager<T>>,
}

impl<T> PostgresAsyncPersistence<T>
where
    T: MakeTlsConnect<Socket> + Clone + 'static + Sync + Send,
    T::TlsConnect: Send,
    T::Stream: Send + Sync,
    <T::TlsConnect as TlsConnect<Socket>>::Future: Send,
{
    pub async fn new(
        connection_manager: bb8_postgres::PostgresConnectionManager<T>,
    ) -> PostgresAsyncPersistence<T> {
        let pool = bb8::Pool::builder()
            .build(connection_manager)
            .await
            .unwrap();

        PostgresAsyncPersistence { conn_pool: pool }
    }

    pub async fn insert_dispatched(
        &self,
        dest: &str,
        file_id: i64,
    ) -> Result<(), PersistenceError> {
        let get_result = self.conn_pool.get().await;

        let client = match get_result {
            Ok(c) => c,
            Err(e) => {
                let message = format!("Error getting PostgreSQL conection from pool: {}", &e);
                error!("{}", &message);

                return Err(PersistenceError::DatabasePool { source: e, message });
            }
        };

        let insert_result = client.execute(
            "insert into dispatcher.dispatched (file_id, target, timestamp) values ($1, $2, now())",
            &[&file_id, &dest]
        ).await;

        match insert_result {
            Ok(_) => Ok(()),
            Err(e) => Err(PersistenceError::Query {
                source: e,
                message: String::from("Error inserting dispatched record into database"),
            }),
        }
    }
}
