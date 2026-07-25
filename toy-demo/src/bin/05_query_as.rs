struct UserSummary {
    id: i64,
    email: String,
}

#[cfg(feature = "fail-query-as-name")]
struct WrongName {
    id: i64,
    address: String,
}

#[cfg(feature = "fail-query-as-type")]
struct WrongType {
    id: i32,
    email: String,
}

fn main() -> Result<(), toy_sqlx::rusqlite::Error> {
    let url = std::env::var("TOY_DATABASE_URL").expect("TOY_DATABASE_URL is required");
    let connection = toy_sqlx::connect(&url)?;
    #[cfg(feature = "fail-query-as-name")]
    let _ = toy_sqlx::query_as!(
        WrongName,
        &connection,
        "SELECT id, email FROM users ORDER BY id"
    )?;
    #[cfg(feature = "fail-query-as-type")]
    let _ = toy_sqlx::query_as!(
        WrongType,
        &connection,
        "SELECT id, email FROM users ORDER BY id"
    )?;
    #[cfg(not(any(feature = "fail-query-as-name", feature = "fail-query-as-type")))]
    for user in toy_sqlx::query_as!(
        UserSummary,
        &connection,
        "SELECT id, email FROM users ORDER BY id"
    )? {
        println!("{} {}", user.id, user.email);
    }
    Ok(())
}
