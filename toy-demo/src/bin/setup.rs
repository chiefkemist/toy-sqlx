fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("TOY_DATABASE_URL").expect("TOY_DATABASE_URL is required");
    let connection = toy_sqlx::connect(&url)?;
    connection.execute_batch(include_str!("../../schema.sql"))?;
    println!("initialized {}", toy_sqlx::database_path(&url).display());
    Ok(())
}
