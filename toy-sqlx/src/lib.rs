use std::path::Path;

pub use rusqlite;
pub use toy_sqlx_macros::{checked_query, checked_sql, query, query_as, sql_literal};

pub fn database_path(url: &str) -> &Path {
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    Path::new(path)
}

pub fn connect(url: &str) -> rusqlite::Result<rusqlite::Connection> {
    let connection = rusqlite::Connection::open(database_path(url))?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(connection)
}

#[doc(hidden)]
pub fn map_rows<T, F>(
    connection: &rusqlite::Connection,
    sql: &str,
    parameters: &[&dyn rusqlite::ToSql],
    mut map: F,
) -> rusqlite::Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map(parameters, |row| map(row))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_urls_and_paths() {
        assert_eq!(database_path("sqlite://db/toy.db"), Path::new("db/toy.db"));
        assert_eq!(database_path("sqlite::memory:"), Path::new(":memory:"));
        assert_eq!(database_path("db/toy.db"), Path::new("db/toy.db"));
    }

    #[test]
    fn enables_foreign_keys() {
        let connection = connect("sqlite::memory:").unwrap();
        let enabled: bool = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert!(enabled);
    }
}
