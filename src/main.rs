use rocket::fairing::AdHoc;
use rocket::fs::FileServer;
use rocket::serde::{Serialize, json::Json};

#[macro_use] extern crate rocket;

#[cfg(test)] mod tests;

#[derive(Serialize)]
struct Recommendation<String> {
    text: String
}

impl Recommendation<String> {
    fn new(text: &str) -> Recommendation<String> {
        Recommendation { text: text.to_string() }
    }
}

use rocket_db_pools::sqlx::pool::PoolConnection;
use rocket_db_pools::{Database};
use rocket_db_pools::sqlx::{self, Sqlite, Row};

#[derive(Database)]
#[database("sites")]
struct Sites(sqlx::SqlitePool);

#[get("/search?<q>")]
async fn search(/*mut db: Connection<Sites>,*/ q: i64) -> Json<Vec<String>> {
    // TODO: Implement search
    Json(vec![q.to_string()])
}

#[get("/recommended?<query>")]
fn recommended(query: Option<&str>) -> Json<Vec<Recommendation<String>>> {
    // Return JSON array of recommended search terms for the query
    // For now, just use the query and "bar"
    let results = vec![Recommendation::new(query.unwrap_or("foo")), Recommendation::new("bar")];
    Json(results)
}

#[rocket::main]
async fn main() {
    let _ = rocket::build()
        .mount("/", FileServer::from("./public"))
        .mount("/api", routes![recommended, search])
        .attach(Sites::init())
        .attach(AdHoc::on_liftoff("Liftoff Printer", |rocket| Box::pin(async move {
            println!("Liftoff!");

            // Look into the database 'sites' and find the staging table.
            // If it's empty, then create it.
            let db: &Sites = Sites::fetch(&rocket).unwrap();
        
            let mut connection: PoolConnection<Sqlite> = db.acquire().await.unwrap();
        
            // First, get the database connection from rocket.
            // Then, make sure the staging table exists.
        
            sqlx::query("CREATE TABLE IF NOT EXISTS staging (url TEXT PRIMARY KEY);")
                .execute(&mut connection)
                .await
                .unwrap();
            println!("Staging table created if it didn't exist.");

            // Create the sitedata table if it doesn't exist.
            // This table has a url (text) as the primary key, a title (text), an array of text for keywords, and a rank (integer).
            sqlx::query("CREATE TABLE IF NOT EXISTS sitedata (url TEXT PRIMARY KEY, title TEXT, keywords TEXT[], rank INTEGER);")
                .execute(&mut connection)
                .await
                .unwrap();
            println!("Sitedata table created if it didn't exist.");

            // Check if there are any sites in the sitedata table. If not, this is the first time we've started to index the web.
            // If there are sites, then we can skip adding wikipedia to the staging table.
            let mut rows = sqlx::query("SELECT * FROM sitedata;")
                .fetch_all(&mut connection)
                .await
                .unwrap();

            if rows.is_empty() {
                // Add wikipedia to the staging table.
                sqlx::query("INSERT INTO staging (url) VALUES (?);")
                    .bind("https://en.wikipedia.org/wiki/Main_Page")
                    .execute(&mut connection)
                    .await
                    .unwrap();
                println!("Added wikipedia to the staging table.");
            }

            // Awesome! We can start indexing the web.
            index_staged_sites(connection).await;
        })))
        .launch().await;
}

async fn index_staged_sites(mut connection: PoolConnection<Sqlite>) {
    // Get the first 100 sites from the staging table and remove them from the staging table.
    // Then, index them.
    let rows = sqlx::query("SELECT * FROM staging LIMIT 100;")
        .fetch_all(&mut connection)
        .await
        .unwrap();
    
    // Remove the sites from the staging table.
    for row in &rows {
        let url: String = row.get("url");
        sqlx::query("DELETE FROM staging WHERE url = ?;")
            .bind(url.clone())
            .execute(&mut connection)
            .await
            .unwrap();

        index_site(url.as_str());
    }
}

fn index_site(url: &str) {
    // TODO: Implement indexing
}