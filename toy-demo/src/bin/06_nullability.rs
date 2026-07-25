fn main() -> Result<(), toy_sqlx::rusqlite::Error> {
    let url = std::env::var("TOY_DATABASE_URL").expect("TOY_DATABASE_URL is required");
    let connection = toy_sqlx::connect(&url)?;
    #[cfg(feature = "fail-expression-metadata")]
    let _ = toy_sqlx::query!(&connection, "SELECT COUNT(*) AS user_count FROM users")?;
    #[cfg(feature = "fail-unsupported-shape")]
    let _ = toy_sqlx::query!(
        &connection,
        "SELECT a.id FROM users AS a JOIN users AS b ON a.id = b.id",
    )?;
    #[cfg(feature = "fail-view-source")]
    let _ = toy_sqlx::query!(&connection, "SELECT bid FROM hidden_join")?;
    #[cfg(feature = "fail-subquery-source")]
    let _ = toy_sqlx::query!(
        &connection,
        "SELECT (VALUES(NULL),(email)) AS email FROM users"
    )?;
    #[cfg(not(any(
        feature = "fail-expression-metadata",
        feature = "fail-unsupported-shape",
        feature = "fail-view-source",
        feature = "fail-subquery-source"
    )))]
    for user in toy_sqlx::query!(
        &connection,
        "SELECT email, display_name FROM users ORDER BY id"
    )? {
        println!("{} {:?}", user.email, user.display_name);
    }
    Ok(())
}
