//! A deliberately small, SQLite-only model of SQLx's compile-time query macros.
//!
//! # The central idea
//!
//! A procedural macro runs while the crate that called it is being compiled.  It
//! receives Rust *tokens*, asks SQLite what a SQL string means, and emits new Rust
//! tokens.  The emitted Rust then goes through the normal Rust compiler.
//!
//! The pipeline is:
//!
//! 1. [`syn`] parses the macro input into Rust data structures.
//! 2. [`rusqlite`] prepares the SQL and returns parameter/column metadata.
//! 3. This crate validates the deliberately small teaching subset.
//! 4. [`quote!`] builds ordinary Rust code from that metadata.
//! 5. `rustc` checks the generated fields, types, and struct literals.
//!
//! This is teaching code, not a general SQL analyzer.  Typed queries intentionally
//! accept only direct columns from one real SQLite table.

// Standard-library tools used for names, environment variables, cache files, and paths.
use std::{collections::HashSet, env, fs, path::PathBuf};

// `proc_macro::TokenStream` is the compiler-facing input and output type.
use proc_macro::TokenStream;
// `proc_macro2` provides token types that are easier to build and test.
use proc_macro2::{Ident, Span, TokenStream as Tokens};
// `quote!` turns Rust-like syntax into tokens; `format_ident!` creates identifiers.
use quote::{format_ident, quote};
// SQLite is both the runtime database and our compile-time SQL authority.
use rusqlite::{Connection, OpenFlags};
// Descriptions are serialized so the same evidence can be used offline.
use serde::{Deserialize, Serialize};
// `syn` parses literals, expressions, types, commas, and custom macro grammars.
#[rustfmt::skip]
use syn::{parse::{Parse, ParseStream}, parse_macro_input, punctuated::Punctuated, Expr, LitStr, Token, Type};

/// Require the macro input to be a string literal, then emit that literal unchanged.
///
/// This first teaching macro does not understand SQL.  Its only job is to show
/// that compile-time checking needs compile-time input: `sql_literal!(variable)`
/// fails because `variable` is not a [`LitStr`].
#[proc_macro]
pub fn sql_literal(input: TokenStream) -> TokenStream {
    // `parse_macro_input!` stops expansion and emits a compiler diagnostic on failure.
    let sql = parse_macro_input!(input as LitStr);
    // `#sql` interpolates the parsed literal into the token stream built by `quote!`.
    quote!(#sql).into()
}

/// Convert our internal `syn::Result` into the token stream required by rustc.
///
/// Procedural macros cannot return `Result`.  A `syn::Error` therefore becomes a
/// `compile_error!` invocation placed at the most useful source span.
fn result(result: syn::Result<Tokens>) -> TokenStream {
    result.unwrap_or_else(syn::Error::into_compile_error).into()
}

// Bump this whenever the meaning or shape of cached metadata changes.  Old cache
// entries then fail clearly instead of being interpreted under new rules.
const CACHE_VERSION: u8 = 3;

/// The small intermediate representation shared by online and offline checking.
///
/// Think of `Description` as a fact sheet about one SQL string:
///
/// - `parameter_count` says how many values SQLite expects;
/// - `columns` describes the result set;
/// - `source_is_table` records whether our narrow typed-query rule was proved;
/// - `version`, `database`, and `sql` protect offline cache loading.
///
/// Keeping one representation is important: code generation does not need to know
/// whether these facts came from live SQLite or from a cache file.
#[rustfmt::skip]
#[derive(Debug, Serialize, Deserialize)]
struct Description { version: u8, database: String, sql: String, parameter_count: usize, source_is_table: bool, columns: Vec<Column> }

/// The SQLite evidence needed to generate one Rust output field.
///
/// `declared_type` is the type written in `CREATE TABLE`; it is not the type of
/// every value SQLite could dynamically store.  `nullable` decides between `T`
/// and `Option<T>` in generated Rust.
#[rustfmt::skip]
#[derive(Debug, Serialize, Deserialize)]
struct Column { name: String, declared_type: Option<String>, nullable: bool }

/// Ask SQLite to validate a SQL literal during macro expansion.
///
/// The emitted value is still just the original string literal.  The improvement
/// is timing: invalid SQL becomes a compiler error instead of a runtime error.
#[proc_macro]
pub fn checked_sql(input: TokenStream) -> TokenStream {
    let sql = parse_macro_input!(input as LitStr);
    // Loading the description performs validation; this macro discards the facts.
    result(load_description(&sql).map(|_| quote!(#sql)))
}

/// Obtain query evidence from exactly one source.
///
/// Offline mode reads a previously saved description.  Online mode asks SQLite
/// directly and optionally saves the answer for a later offline build.
fn load_description(sql: &LitStr) -> syn::Result<Description> {
    if flag("TOY_SQLX_OFFLINE") {
        return load_cache(&sql.value(), sql.span());
    }
    let description = describe_online(&sql.value(), sql.span())?;
    if flag("TOY_SQLX_PREPARE") {
        save_cache(&description, sql.span())?;
    }
    Ok(description)
}

/// Recognize the deliberately tiny SQL shape supported by typed queries.
///
/// This is a *scope guard*, not a SQL parser.  It returns a table name only for a
/// projection of direct columns from one simple table.  Any uncertainty returns
/// `None`, which makes the typed macro reject the query rather than invent facts.
#[rustfmt::skip]
fn direct_source(sql: &str) -> Option<&str> {
    // Comments could hide extra `FROM` clauses, so this toy rejects them outright.
    if ["--", "/*", "*/"].iter().any(|marker| sql.contains(marker)) { return None; }
    // Whitespace tokenization is enough because every accepted shape is simple.
    let words = sql.split_whitespace().collect::<Vec<_>>();
    // Exactly one FROM excludes ordinary SELECT subqueries and ambiguous sources.
    let from = words.iter().enumerate().filter(|(_, word)| word.eq_ignore_ascii_case("FROM")).map(|(index, _)| index).collect::<Vec<_>>();
    let [from] = from.as_slice() else { return None };
    // The first word after FROM must be an unquoted, unqualified table identifier.
    let table = *words.get(from + 1)?;
    // Stop the source clause when ordinary filtering/ordering begins.
    let tail = words[from + 2..].iter().take_while(|word| !matches!(word.to_ascii_uppercase().as_str(), "WHERE" | "GROUP" | "ORDER" | "LIMIT")).copied().collect::<Vec<_>>();
    let ident = |word: &str| word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    // Every selected field must be `column`, `table.column`, or either form with `AS name`.
    let fields = words[1..*from].join(" ");
    let direct_field = |field: &str| { let parts = field.split_whitespace().collect::<Vec<_>>(); let path = parts.first()?.split('.').collect::<Vec<_>>(); Some((path.len() == 1 || path.len() == 2) && path.iter().all(|part| ident(part)) && (parts.len() == 1 || (parts.len() == 3 && parts[1].eq_ignore_ascii_case("AS") && ident(parts[2])))) };
    // After the table, allow no alias, `alias`, or `AS alias`—never a second source.
    let alias = tail.is_empty() || (tail.len() == 1 && ident(tail[0])) || (tail.len() == 2 && tail[0].eq_ignore_ascii_case("AS") && ident(tail[1]));
    (ident(table) && alias && fields.split(',').all(|field| direct_field(field) == Some(true))).then_some(table)
}

/// Ask a live SQLite database to describe a query during compilation.
///
/// Preparing validates syntax, table names, and column names without executing the
/// query.  `span` points diagnostics back at the SQL literal in the caller's code.
#[rustfmt::skip]
fn describe_online(sql: &str, span: Span) -> syn::Result<Description> {
    // Cargo runs this function inside the proc-macro process, not in the final program.
    let url = env::var("TOY_DATABASE_URL")
        .map_err(|_| syn::Error::new(span, "TOY_DATABASE_URL is required for online checking"))?;
    // Accept the same small set of SQLite URL forms as the runtime crate.
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(&url);
    // Read-only mode prevents a compile from modifying the teaching database.
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|error| syn::Error::new(span, format!("cannot open SQLite: {error}")))?;
    // This is the key compile-time check: let SQLite validate SQLite SQL.
    let statement = connection.prepare(sql).map_err(|error| {
        syn::Error::new(
            span,
            format!("SQLite rejected this query during compilation: {error}"),
        )
    })?;
    // Copy only the output facts needed by later Rust code generation.
    let columns = (0..statement.column_count())
        .map(|index| {
            let name = statement.column_name(index)?.to_owned();
            let metadata = statement.column_metadata(index)?;
            // rusqlite exposes several origin fields; this toy needs declaration and NOT NULL.
            let (declared_type, nullable) = match metadata {
                Some((_, _, _, declared, _, not_null, _, _)) => (
                    declared.map(|value| value.to_string_lossy().into_owned()),
                    !not_null,
                ),
                // Missing origin evidence must never be treated as non-null.
                None => (None, true),
            };
            Ok(Column { name, declared_type, nullable })
        })
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| syn::Error::new(span, format!("cannot describe output: {error}")))?;
    // Lexical recognition is not enough: sqlite_schema must confirm a real table, not a view.
    #[rustfmt::skip]
    let source_is_table = direct_source(sql).is_some_and(|table| connection.query_row(
        "SELECT type = 'table' FROM sqlite_schema WHERE name = ?1", [table], |row| row.get(0),
    ).unwrap_or(false));
    // The prepared statement itself supplies SQLite's authoritative placeholder count.
    Ok(Description {
        version: CACHE_VERSION,
        database: "SQLite".into(),
        sql: sql.into(),
        parameter_count: statement.parameter_count(),
        source_is_table,
        columns,
    })
}

/// Parsed input for `checked_query!("SQL", arg1, arg2, ...)`.
///
/// `Punctuated` is syn's representation of zero or more expressions separated by
/// commas.  Keeping expressions as syntax lets us count them without running them.
#[rustfmt::skip]
struct CheckedInput { sql: LitStr, args: Punctuated<Expr, Token![,]> }

/// Teach syn the little grammar accepted by `checked_query!`.
impl Parse for CheckedInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        // The first token must be the SQL string literal.
        let sql = input.parse()?;
        let args = if input.is_empty() {
            // A query without placeholders needs no comma and no arguments.
            Punctuated::new()
        } else {
            // Otherwise consume the comma after SQL, then all comma-separated expressions.
            input.parse::<Token![,]>()?;
            Punctuated::parse_terminated(input)?
        };
        Ok(Self { sql, args })
    }
}

/// Validate SQL plus parameter arity, but do not execute the query.
///
/// This transitional macro isolates one lesson: SQLite can tell us how many
/// parameters it expects, but not static SQLite parameter types.
#[proc_macro]
pub fn checked_query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as CheckedInput);
    result(expand_checked(input))
}

/// Implement `checked_query!` after its tokens have been parsed.
fn expand_checked(input: CheckedInput) -> syn::Result<Tokens> {
    let description = load_description(&input.sql)?;
    validate_arity(input.args.len(), &description, input.sql.span())?;
    let CheckedInput { sql, args } = input;
    let args = args.into_iter().collect::<Vec<_>>();
    Ok(quote! {{
        // This branch never runs, but rustc still resolves and type-checks each expression.
        // `#(...)*` is quote's repetition syntax: emit the body once per argument.
        if false {
            #(let _ = &(#args);)*
        }
        // The macro's runtime value remains the original SQL string.
        #sql
    }})
}

/// Parsed input for `query!(&connection, "SQL", arg1, arg2, ...)`.
///
/// The connection and every argument are full Rust expressions.  They are kept as
/// syntax until expansion emits code that evaluates each exactly once.
#[rustfmt::skip]
struct QueryInput { connection: Expr, sql: LitStr, args: Punctuated<Expr, Token![,]> }

/// Parse the connection first, then reuse `CheckedInput` for SQL and arguments.
#[rustfmt::skip]
impl Parse for QueryInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let connection = input.parse()?; input.parse::<Token![,]>()?;
        let checked = input.parse::<CheckedInput>()?;
        Ok(Self { connection, sql: checked.sql, args: checked.args })
    }
}

/// Validate and execute a query whose output record is generated by the macro.
///
/// The local generated type has one Rust field per selected SQL column.
#[proc_macro]
pub fn query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    result(expand_query(input, Output::Generated))
}

/// Choose which Rust value the row decoder constructs.
enum Output {
    /// `query!` asks us to define a local `Record` type.
    Generated,
    /// `query_as!` gives us an application-owned type to construct.
    Given(Box<Type>),
}

/// A checked SQL output column translated into a Rust field name and type tokens.
#[rustfmt::skip]
struct RustColumn { ident: Ident, ty: Tokens }

/// Build the Rust program emitted by `query!` and `query_as!`.
///
/// Everything before the final `quote!` runs during compilation.  The code inside
/// that `quote!` is what the caller's program will execute at runtime.
#[rustfmt::skip]
fn expand_query(input: QueryInput, output: Output) -> syn::Result<Tokens> {
    // Compile-time phase: gather evidence and reject unsupported input.
    let description = load_description(&input.sql)?;
    validate_arity(input.args.len(), &description, input.sql.span())?;
    let columns = typed_columns(&description, input.sql.span())?;

    // Turn each checked column into `field_name: RustType` tokens.
    let fields = columns.iter().map(|column| {
        let ident = &column.ident;
        let ty = &column.ty;
        quote!(#ident: #ty)
    });
    // Turn each checked column into `field_name: row.get::<_, RustType>(index)?`.
    let values: Vec<_> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let ident = &column.ident;
            let ty = &column.ty;
            quote!(#ident: __toy_row.get::<usize, #ty>(#index)?)
        })
        .collect();

    // Both public macros share one decoder; only their final constructor differs.
    let (definition, expression) = match output {
        Output::Generated => (
            quote!(#[derive(Debug)] struct Record { #(#fields),* }),
            quote!(Record { #(#values),* }),
        ),
        Output::Given(output) => (Tokens::new(), quote!(#output { #(#values),* })),
    };

    let QueryInput { connection, sql, args } = input;
    // Generated local names let us evaluate every caller expression exactly once.
    let names: Vec<_> = (0..args.len())
        .map(|index| format_ident!("__toy_arg_{index}"))
        .collect();
    let args = args.into_iter().collect::<Vec<_>>();

    // Runtime phase: this entire block is inserted at the macro call site.
    Ok(quote! {{
        #definition
        // Tuple right-hand sides are evaluated before the generated names are bound.
        let (__toy_connection, #(#names,)*) = (#connection, #(&(#args),)*);
        // rusqlite accepts a slice of references to values implementing `ToSql`.
        let __toy_parameters: &[&dyn ::toy_sqlx::rusqlite::ToSql] = &[#(#names),*];
        ::toy_sqlx::map_rows(
            __toy_connection,
            #sql,
            __toy_parameters,
            // Decode one SQLite row into the generated or caller-provided struct.
            |__toy_row| ::core::result::Result::Ok(#expression),
        )
    }})
}

/// Compare the number of Rust arguments with SQLite's placeholder count.
///
/// This intentionally checks only *how many* values exist.  SQLite does not give
/// this toy stable static parameter types, so claiming more would be misleading.
fn validate_arity(got: usize, description: &Description, span: Span) -> syn::Result<()> {
    if got == description.parameter_count {
        Ok(())
    } else {
        let expected = description.parameter_count;
        Err(syn::Error::new(
            span,
            format!("SQLite expects {expected} parameter(s), but the macro received {got}"),
        ))
    }
}

/// Turn SQLite columns into safe Rust field names and types.
///
/// The function first enforces the evidence boundary, then checks names, rejects
/// duplicate fields, maps SQLite declarations, and applies nullability.
fn typed_columns(description: &Description, span: Span) -> syn::Result<Vec<RustColumn>> {
    ensure_typed_shape(&description.sql, span)?;
    if !description.source_is_table {
        return Err(syn::Error::new(
            span,
            "typed query must select direct columns from one table; expressions, views, and subqueries are out of scope",
        ));
    }
    if description.columns.is_empty() {
        return Err(syn::Error::new(span, "typed query must return columns"));
    }
    let mut names = HashSet::new();
    description
        .columns
        .iter()
        .map(|column| {
            // A SQL output name must be usable in a generated Rust struct literal.
            let ident = rust_ident(&column.name, span)?;
            if !names.insert(ident.to_string()) {
                return Err(syn::Error::new(span, "duplicate output field name"));
            }
            // Expressions often have no table declaration; typed output rejects them.
            let declared = column.declared_type.as_deref().ok_or_else(|| {
                let name = &column.name;
                syn::Error::new(span, format!("SQLite has no declared type for {name:?}; typed queries only support direct table columns"))
            })?;
            let base = rust_type(declared, span)?;
            // Rust represents a possibly-NULL SQL value as `Option<T>`.
            let ty = if column.nullable {
                quote!(::core::option::Option<#base>)
            } else {
                base
            };
            Ok(RustColumn { ident, ty })
        })
        .collect()
}

/// Reject broad SQL shapes before generating typed Rust.
///
/// Joins and compound queries need nullability analysis that this workshop does
/// not implement.  Conservative rejection keeps the small guarantee honest.
fn ensure_typed_shape(sql: &str, span: Span) -> syn::Result<()> {
    let sql = sql.trim_start().to_ascii_uppercase();
    let words =
        || sql.split(|character: char| !character.is_ascii_alphanumeric() && character != '_');
    let unsupported = !sql.starts_with("SELECT ")
        || words().filter(|word| *word == "SELECT").count() != 1
        || words().any(|word| matches!(word, "JOIN" | "UNION" | "INTERSECT" | "EXCEPT"));
    if unsupported {
        Err(syn::Error::new(
            span,
            "typed queries support one direct-table SELECT; joins and compound queries are out of scope",
        ))
    } else {
        Ok(())
    }
}

/// Convert a SQLite output name into a Rust field identifier.
///
/// The raw form (`r#type`, for example) allows Rust keywords when legal.  The
/// fallback handles ordinary identifiers on compiler versions with differing
/// raw-identifier parsing behavior.
fn rust_ident(name: &str, span: Span) -> syn::Result<Ident> {
    syn::parse_str(&format!("r#{name}"))
        .or_else(|_| syn::parse_str(name))
        .map_err(|_| syn::Error::new(span, format!("{name:?} is not a Rust field name")))
}

/// Map a small set of SQLite declared types to Rust type tokens.
///
/// SQLite uses type affinity and permits dynamic stored values.  This table is a
/// teaching subset, not a claim that every stored value must have this Rust type.
///
/// | Declaration contains | Generated base type |
/// | --- | --- |
/// | `BOOL` or `BOOLEAN` | `bool` |
/// | `INT` | `i64` |
/// | `REAL`, `FLOA`, or `DOUB` | `f64` |
/// | `CHAR`, `CLOB`, or `TEXT` | `String` |
/// | `BLOB` | `Vec<u8>` |
/// | anything else | compiler error |
fn rust_type(declared: &str, span: Span) -> syn::Result<Tokens> {
    let name = declared.trim().to_ascii_uppercase();
    if matches!(name.as_str(), "BOOL" | "BOOLEAN") {
        Ok(quote!(bool))
    } else if name.contains("INT") {
        Ok(quote!(i64))
    } else if name.contains("REAL") || name.contains("FLOA") || name.contains("DOUB") {
        Ok(quote!(f64))
    } else if name.contains("CHAR") || name.contains("CLOB") || name.contains("TEXT") {
        Ok(quote!(::std::string::String))
    } else if name.contains("BLOB") {
        Ok(quote!(::std::vec::Vec<u8>))
    } else {
        Err(syn::Error::new(
            span,
            format!("no toy Rust mapping for SQLite declaration {declared:?}"),
        ))
    }
}

/// Parsed input for `query_as!(OutputType, &connection, "SQL", args...)`.
#[rustfmt::skip]
struct QueryAsInput { output: Type, query: QueryInput }

/// Parse the caller's Rust type, then reuse the complete `query!` grammar.
#[rustfmt::skip]
impl Parse for QueryAsInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let output = input.parse()?; input.parse::<Token![,]>()?;
        Ok(Self { output, query: input.parse()? })
    }
}

/// Validate and execute a query into a caller-provided Rust struct.
///
/// The macro emits an ordinary struct literal.  rustc therefore reports missing,
/// extra, or incorrectly typed fields without custom reflection machinery.
#[proc_macro]
#[rustfmt::skip]
pub fn query_as(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryAsInput);
    result(expand_query(input.query, Output::Given(Box::new(input.output))))
}

/// Read the workshop's Boolean environment flags.
fn flag(name: &str) -> bool {
    env::var(name)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE"))
}

/// Produce a short, deterministic cache filename from the exact SQL text.
///
/// FNV-1a is used because its loop is easy to teach.  A production cache would
/// normally use a collision-resistant hash and stronger concurrency guarantees.
fn hash(sql: &str) -> u64 {
    sql.as_bytes()
        .iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

/// Locate this query's cache file inside the crate that invoked the macro.
///
/// `CARGO_MANIFEST_DIR` belongs to the caller, so each crate gets its own
/// `.toy-sqlx` directory rather than sharing the proc-macro crate's directory.
fn cache_path(sql: &str, span: Span) -> syn::Result<PathBuf> {
    let manifest = env::var_os("CARGO_MANIFEST_DIR")
        .ok_or_else(|| syn::Error::new(span, "Cargo did not provide CARGO_MANIFEST_DIR"))?;
    Ok(PathBuf::from(manifest)
        .join(".toy-sqlx")
        .join(format!("query-{:016x}.json", hash(sql))))
}

/// Serialize a live SQLite description as readable JSON for offline compilation.
fn save_cache(description: &Description, span: Span) -> syn::Result<()> {
    let path = cache_path(&description.sql, span)?;
    fs::create_dir_all(path.parent().unwrap())
        .and_then(|_| fs::write(&path, serde_json::to_vec_pretty(description).unwrap()))
        .map_err(|error| syn::Error::new(span, format!("cannot write {}: {error}", path.display())))
}

/// Load cached evidence and reject anything that does not match this query.
///
/// Checking version, backend, and exact SQL prevents a cache entry created under
/// different assumptions from silently driving Rust code generation.
fn load_cache(sql: &str, span: Span) -> syn::Result<Description> {
    let path = cache_path(sql, span)?;
    let bytes = fs::read(&path).map_err(|error| {
        syn::Error::new(
            span,
            format!("offline metadata missing at {}: {error}", path.display()),
        )
    })?;
    let description: Description = serde_json::from_slice(&bytes)
        .map_err(|error| syn::Error::new(span, format!("invalid offline metadata: {error}")))?;
    // All three checks are part of the cache's trust boundary.
    if description.version != CACHE_VERSION
        || description.database != "SQLite"
        || description.sql != sql
    {
        Err(syn::Error::new(
            span,
            "offline metadata does not match this SQLite query",
        ))
    } else {
        Ok(description)
    }
}

// These tests target the small policies that are easiest to break while editing
// the workshop: type mapping, query-shape rejection, field names, and cache hashes.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[rustfmt::skip]
    fn maps_the_teaching_types() {
        // Compare emitted tokens as text because these helpers generate code, not values.
        assert_eq!(rust_type("INTEGER", Span::call_site()).unwrap().to_string(), "i64");
        assert_eq!(
            rust_type("TEXT", Span::call_site()).unwrap().to_string(),
            ":: std :: string :: String"
        );
        assert!(rust_type("NUMERIC", Span::call_site()).is_err());
    }

    #[test]
    #[rustfmt::skip]
    fn rejects_unsupported_sources_and_shapes() {
        // Every uncertain shape must fail closed rather than generate an unsound type.
        assert!(ensure_typed_shape("SELECT * FROM a LEFT JOIN b", Span::call_site()).is_err());
        assert!(ensure_typed_shape("DELETE FROM a RETURNING id", Span::call_site()).is_err());
        assert!(ensure_typed_shape("SELECT (SELECT id FROM a) FROM b", Span::call_site()).is_err());
        assert_eq!(direct_source("SELECT id FROM users"), Some("users"));
        assert_eq!(direct_source("SELECT id FROM users AS u WHERE u.id > 0"), Some("users"));
        assert_eq!(direct_source("SELECT bid /* FROM users */ FROM hidden_join"), None);
        assert_eq!(direct_source("SELECT u.id FROM users u , hidden_join v"), None);
        assert_eq!(direct_source("SELECT (VALUES(NULL),(email)) AS email FROM users"), None);
    }

    #[test]
    #[rustfmt::skip]
    fn rejects_bad_and_duplicate_fields() {
        // Invalid or repeated SQL names cannot form a valid Rust struct.
        assert!(rust_ident("bad name", Span::call_site()).is_err());
        let column = || Column { name: "id".into(), declared_type: Some("INTEGER".into()), nullable: false };
        let description = Description { version: CACHE_VERSION, database: "SQLite".into(), sql: "SELECT id, id FROM users".into(), parameter_count: 0, source_is_table: true, columns: vec![column(), column()] };
        assert!(typed_columns(&description, Span::call_site()).is_err());
    }

    #[test]
    fn hash_is_stable() {
        // Changing this value would move every cache file and requires an explicit decision.
        assert_eq!(hash("SELECT 1"), 0x199e7bca63ea84f2);
    }
}
