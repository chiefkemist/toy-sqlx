fn main() -> Result<(), toy_sqlx::rusqlite::Error> {
    let url = std::env::var("TOY_DATABASE_URL").expect("TOY_DATABASE_URL is required");
    let connection = toy_sqlx::connect(&url)?;
    let result = connection.prepare("SELECT id, definitely_missing FROM users");
    match result {
        Ok(_) => println!("unexpected success"),
        Err(error) => println!("runtime SQLite error: {error}"),
    }
    Ok(())
}
