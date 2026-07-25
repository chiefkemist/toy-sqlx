fn main() -> Result<(), toy_sqlx::rusqlite::Error> {
    let url = std::env::var("TOY_DATABASE_URL").expect("TOY_DATABASE_URL is required");
    let connection = toy_sqlx::connect(&url)?;
    let users = toy_sqlx::query!(
        &connection,
        "SELECT id, email, display_name FROM users WHERE active = ?1 ORDER BY id",
        true,
    )?;
    for user in users {
        println!("{} {} {:?}", user.id, user.email, user.display_name);
    }
    Ok(())
}
