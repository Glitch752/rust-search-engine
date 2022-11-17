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
use rocket_db_pools::sqlx::{self, Sqlite};

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
            // Then, check if the staging table exists.
        
            let staging_table_exists = sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name='staging';")
                .fetch_one(&mut connection)
                .await
                .is_ok();
        
            if !staging_table_exists {
                println!("Staging table doesn't exist. Creating it now.");
                sqlx::query("CREATE TABLE staging (url TEXT PRIMARY KEY);")
                    .execute(&mut connection)
                    .await
                    .unwrap();
            } else {
                println!("Staging table exists.");
            }
        })))
        .launch().await;
}
