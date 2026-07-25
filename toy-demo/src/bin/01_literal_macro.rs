fn main() {
    #[cfg(feature = "fail-literal")]
    {
        let sql = "SELECT 40 + 2";
        let _ = toy_sqlx::sql_literal!(sql);
    }
    #[cfg(not(feature = "fail-literal"))]
    println!("{}", toy_sqlx::sql_literal!("SELECT 40 + 2 AS answer"));
}
