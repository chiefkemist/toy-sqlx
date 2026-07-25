fn main() {
    #[cfg(feature = "fail-invalid-sql")]
    let sql = toy_sqlx::checked_sql!("SELECT id, definitely_missing FROM users");
    #[cfg(not(feature = "fail-invalid-sql"))]
    let sql = toy_sqlx::checked_sql!("SELECT id, email FROM users");
    println!("SQLite accepted during compilation: {sql}");
}
