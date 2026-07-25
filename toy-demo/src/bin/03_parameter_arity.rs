fn main() {
    let deliberately_wrong_type = "SQLite accepts dynamic values";
    #[cfg(feature = "fail-arity")]
    let sql = toy_sqlx::checked_query!(
        "SELECT id FROM users WHERE active = ?1 AND id >= ?2",
        deliberately_wrong_type,
    );
    #[cfg(not(feature = "fail-arity"))]
    let sql = toy_sqlx::checked_query!(
        "SELECT id FROM users WHERE active = ?1",
        deliberately_wrong_type,
    );
    println!("arity checked, parameter type deliberately unchecked: {sql}");
}
