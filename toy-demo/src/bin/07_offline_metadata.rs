fn main() -> Result<(), toy_sqlx::rusqlite::Error> {
    let url = std::env::var("TOY_DATABASE_URL").expect("TOY_DATABASE_URL is required");
    let connection = toy_sqlx::connect(&url)?;
    let users = toy_sqlx::query!(&connection, "SELECT id, email FROM users ORDER BY id")?;
    println!(
        "decoded {} users using a compile-time description",
        users.len()
    );
    Ok(())
}
