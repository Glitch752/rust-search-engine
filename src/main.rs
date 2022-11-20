use reqwest::Client;
use rocket::{tokio};
use rocket::fairing::AdHoc;
use rocket::fs::FileServer;
use rocket::serde::{Serialize, json::Json};

use scraper::{Html, Selector, ElementRef};

// Runs prinln!() with all the arguments if the debug flag is passed on the command line.
// This function can take as many arguments as needed.

// debugMessage! macro
macro_rules! debugMessage {
    ($($arg:tt)*) => {
        // Check if the --debug flag is passed on the command line
        if std::env::args().any(|x| x == "--debug") {
            println!($($arg)*);
        }
    }
}

// TODO: Implement concurrent requests to increase performance
// const CONCURRENT_INDEXING_AMOUNT: usize = 10; // How many network requests to make at once when indexing

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
use rocket_db_pools::sqlx::sqlite::SqliteRow;
use rocket_db_pools::{Database, Connection};
use rocket_db_pools::sqlx::{self, Sqlite, Row};

#[derive(Database)]
#[database("sites")]
struct Sites(sqlx::SqlitePool);

#[derive(Serialize)]
struct Site {
    title: String,
    url: String,
}

#[derive(Serialize)]
struct SearchResults {
    sites: Vec<Site>,
    count: u32
}

#[get("/search?<query>&<page>")]
async fn search(mut db: Connection<Sites>, query: String, page: u32) -> Json<SearchResults> {
    // Get sites from the database with similar titles
    let sites: Vec<SqliteRow> = sqlx::query("SELECT title, url FROM sitedata WHERE title LIKE ? ESCAPE '\\' LIMIT 30 OFFSET ?")
        .bind(format!("%{}%", query))
        .bind((page - 1) * 30)
        .fetch_all(&mut *db)
        .await
        .unwrap();

    // Get how many sites are found
    let count: u32 = sqlx::query("SELECT COUNT(*) FROM sitedata WHERE title LIKE ? ESCAPE '\\'")
        .bind(format!("%{}%", query))
        .fetch_one(&mut *db)
        .await
        .unwrap()
        .get(0);

    let sites_data: Vec<Site> = sites.iter().map(|site| {
        let title: String = site.try_get::<&str, &str>("title").unwrap_or("Unknown").to_string();
        let url: String = site.try_get::<&str, &str>("url").unwrap_or("Unknown").to_string();
        Site { title, url }
    }).collect();

    Json(SearchResults { sites: sites_data, count })
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
        .attach(AdHoc::on_liftoff("Index web", |rocket| Box::pin(async move {
            if std::env::args().any(|x| x == "--no-index") {
                return;
            }

            let db: &Sites = Sites::fetch(&rocket).unwrap();
            let connection: PoolConnection<Sqlite> = db.acquire().await.unwrap();
            tokio::spawn(async move {
                println!("Liftoff!");
                index_web(connection).await;
            });
        })))
        .launch().await;
}

async fn index_web(mut connection: PoolConnection<Sqlite>) {
    println!("Indexing web...");
    // Look into the database 'sites' and find the staging table.
    // If it's empty, then create it.
    // Turn the PoolConnection<Sqlite> into a SqlitePool

    // First, get the database connection from rocket.
    // Then, make sure the staging table exists.

    sqlx::query("CREATE TABLE IF NOT EXISTS staging (url TEXT PRIMARY KEY);")
        .execute(&mut connection)
        .await
        .unwrap();
    debugMessage!("Staging table created if it didn't exist.");

    // Create the sitedata table if it doesn't exist.
    // This table has a url (text) as the primary key, a title (text), an array of text for keywords, and a rank (integer).
    sqlx::query("CREATE TABLE IF NOT EXISTS sitedata (url TEXT PRIMARY KEY, title TEXT, keywords TEXT, rank INTEGER);")
        .execute(&mut connection)
        .await
        .unwrap();
    debugMessage!("Sitedata table created if it didn't exist.");

    let default_site: String = "https://github.com/".to_string();

    // Check if there are any sites in the sitedata table. If not, this is the first time we've started to index the web.
    // If there are sites, then we can skip adding the default site to the staging table.
    let rows = sqlx::query("SELECT * FROM sitedata;")
        .fetch_all(&mut connection)
        .await
        .unwrap();

    if rows.is_empty() {
        // Add the default site to the staging table.
        sqlx::query("INSERT INTO staging (url) VALUES (?);")
            .bind(default_site.clone())
            .execute(&mut connection)
            .await
            .unwrap();
        debugMessage!("Added {} to the staging table.", default_site);
    }

    // Awesome! We can start indexing the web.
    index_staged_sites(connection).await;
}

async fn index_staged_sites(mut connection: PoolConnection<Sqlite>) {
    // Get the first 100 sites from the staging table and remove them from the staging table.
    // Then, index them.
    let mut rows = sqlx::query("SELECT * FROM staging LIMIT 100;")
        .fetch_all(&mut connection)
        .await
        .unwrap();

    let mut are_staged_sites: bool = true;

    let mut sites_scanned: u64 = 0;

    let client = reqwest::Client::new();

    while are_staged_sites {
        // Remove the sites from the staging table.
        for row in &rows {
            let url: String = row.get("url");
            debugMessage!("{}", url);
            // Make sure we don't move connection so it can be used in the next iteration of the loop.
            sqlx::query("DELETE FROM staging WHERE url = ?;")
                .bind(url.clone())
                .execute(&mut connection)
                .await
                .unwrap();

            debugMessage!("Removed {} from the staging table.", url);

            index_site(url.as_str(), &mut connection, &client).await;

            debugMessage!("Indexed {}.", url);

            sites_scanned += 1;
            debugMessage!("Sites scanned: {}", sites_scanned);
        }

        debugMessage!("Finished indexing set of 100 staged sites.");

        // Check if there are any more sites in the staging table.
        rows = sqlx::query("SELECT * FROM staging LIMIT 100;")
            .fetch_all(&mut connection)
            .await
            .unwrap();
        
        if rows.is_empty() {
            debugMessage!("No more sites in the staging table.");
            are_staged_sites = false;
        }
    }

    // We're done indexing the web!
    println!("Done indexing the web! This probably shouldn't have been called unless this was left running for a rediculous amount of time or something went wrong.");
}

async fn index_site(url: &str, connection: &mut PoolConnection<Sqlite>, client: &Client) {
    // 1. Download the site.
    // Use reqwest to download the site.
    println!("Downloading {}...", url);

    // Make sure that we can get the site, even if it's a redirect.
    let body = client.get(url).send().await;

    debugMessage!("Downloaded {}", url);

    if body.is_err() {
        debugMessage!("Error downloading site: {}. Continuing to next site.", url);
        return;
    }

    let body = body.unwrap();

    if !body.status().is_success() {
        debugMessage!("Error downloading site: {}. Response code: {}. Continuing to next site.", url, body.status().as_str());
        return;
    }
    
    // Make sure the result is a website and not a different file format.
    if !body.headers().get("content-type").unwrap().to_str().unwrap().contains("text/html") {
        debugMessage!("{} is not a website. Continuing to next site.", url);
        return;
    }

    let (title, keywords, rank, links): (String, String, i32, Vec<String>) = {
        let text = body.text().await;

        if text.is_err() {
            debugMessage!("Error getting text from site: {}. Continuing to next site.", url);
            return;
        }

        let text: String = text.unwrap();

        // 2. Parse the site.
        debugMessage!("Parsing site: {}", url);
        let document: Html = Html::parse_document(text.as_str());

        debugMessage!("Parsed site: {}", url);
        
        // 3. Get the title of the site.
        let titles: Vec<ElementRef> = document.select(&Selector::parse("title").unwrap()).collect();

        let title: String = if titles.is_empty() {
            debugMessage!("No title found for site {}.", url);
            "No title".to_string()
        } else {
            debugMessage!("Title found for site {}: {}.", url, titles[0].text().collect::<String>());
            titles[0].text().collect()
        };

        // 4. Get the keywords of the site.
        let keyword_elements: Vec<ElementRef> = document.select(&Selector::parse("meta[name='keywords']").unwrap()).collect();

        let keywords: String = if keyword_elements.is_empty() {
            debugMessage!("No keywords found for site {}.", url);
            "No keywords".to_string()
        } else {
            let keywords: &str = keyword_elements[0].value().attr("content").unwrap_or("");
            debugMessage!("Keywords found for site {}: {}.", url, keywords);
            keywords.to_string()
        };

        // 5. Get the rank of the site.
        let rank = 0;
        // TODO: Rank other sites higher when we find links to them.

        // 7. Add links to the staging table if they aren't already in the staging table or the sitedata table.
        let links: Vec<String> = document.select(&Selector::parse("a").unwrap()).map(|link| {
            let value: &str = link.value().attr("href").unwrap_or("");
            let absolute_url: String = parse_relative_url(url, value);
            absolute_url
        }).collect();
        
        let links: Vec<String> = links.iter().filter(|link| {
            !link.is_empty()
        }).map(|link| {
            link.to_string()
        }).collect();

        (title, keywords, rank, links)
    };

    // 6. Add the site to the sitedata table.
    sqlx::query("INSERT INTO sitedata (url, title, keywords, rank) VALUES (?, ?, ?, ?);")
        .bind(url)
        .bind(title)
        .bind(keywords)
        .bind(rank)
        .execute(&mut *connection)
        .await
        .unwrap();

    debugMessage!("Inserting new links into the staging table.");

    for link in links {
        let rows = sqlx::query("SELECT * FROM staging WHERE url = ?;")
            .bind(link.clone())
            .fetch_all(&mut *connection)
            .await
            .unwrap();
        if rows.is_empty() {
            let rows = sqlx::query("SELECT * FROM sitedata WHERE url = ?;")
                .bind(link.clone())
                .fetch_all(&mut *connection)
                .await
                .unwrap();
            if rows.is_empty() {
                sqlx::query("INSERT INTO staging (url) VALUES (?);")
                    .bind(link)
                    .execute(&mut *connection)
                    .await
                    .unwrap();
            }
        }
    }
}

fn parse_relative_url(base_url: &str, relative_url: &str) -> String {
    let url: String = get_relative_url(base_url, relative_url);

    // Make sure the url is valid.
    let url_regex = regex::Regex::new(r"^(?P<protocol>http|https)://(?P<site>[a-zA-Z0-9\.\-]+)(?P<path>/.*)?$").unwrap();
    let url_match = url_regex.captures(&url);
    if url_match.is_none() {
        return "".to_string();
    }

    url
}

fn get_relative_url(base_url: &str, relative_url: &str) -> String {// TODO: Make this actually test for all cases.
    // Remove all the query parameters of the relative url
    let relative_url = relative_url.split('?').collect::<Vec<&str>>()[0];
    // Remove all the hash parameters of the relative url
    let relative_url = relative_url.split('#').collect::<Vec<&str>>()[0];

    // Check if the relative_url is a full url.
    if relative_url.starts_with("http://") || relative_url.starts_with("https://") {
        return relative_url.to_string()
    }

    // Get the base url without the path.
    let base_url = base_url.split("/").collect::<Vec<&str>>();

    // Check if the relative_url is a relative url.
    if relative_url.starts_with("/") || relative_url.starts_with("./") {
        // Get the relative URL without the . if it has one.
        if relative_url.starts_with("./") {
            return format!("{}//{}/{}", base_url[0], base_url[2], &relative_url[2..])
        } else {
            return format!("{}//{}/{}", base_url[0], base_url[2], &relative_url[1..])
        }
    }
    
    // Check if the start of relative_url is a site without a protocol.
    let url_regex = regex::Regex::new(r"^(?P<site>[a-zA-Z0-9\.\-]+)(?P<path>/.*)?$").unwrap();
    let url_match = url_regex.captures(relative_url);
    if url_match.is_some() {
        let url_match = url_match.unwrap();
        let site = url_match.name("site");
        let path = url_match.name("path");
        if site.is_some() && path.is_some() {
            return format!("http://{}/{}", site.unwrap().as_str(), path.unwrap().as_str())
        } else if site.is_some() {
            return format!("http://{}/", site.unwrap().as_str())
        }
    }

    // Otherwise, it doesn't have a base url.
    return format!("{}//{}/{}", base_url[0], base_url[2], &relative_url)
}