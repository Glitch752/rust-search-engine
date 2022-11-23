// TODO:
// Implement meta name="robots"
// Implement sitemap.xml
// Implement link rel="canonical"
// Implement header X-Robots-Tag
// Swap between different hosts so we don't spam a single site repeatedly. Ideally, we would only send a request to each site every 1-2 seconds.

use std::{time::{Duration, SystemTime}, sync::{Arc, Mutex}, collections::HashSet};

use reqwest::header::HeaderValue;
use rocket::{tokio, futures::{stream, StreamExt}};
use rocket::fairing::AdHoc;
use rocket::fs::FileServer;
use rocket::serde::{Serialize, json::Json};

use scraper::{Html, Selector, ElementRef};

use robots_txt::{Robots, matcher::SimpleMatcher};

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
const CONCURRENT_INDEXING_AMOUNT: u16 = 10; // How many network requests to make at once when indexing
const USER_AGENT: &str = "RustSearchEngineCrawler/0.1.1; (+https://github.com/Glitch752/rust-search-engine)"; // User agent to use when crawling

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

#[derive(Serialize)]
struct SiteInformation {
    title: String,
    keywords: Vec<String>,
    rank: u32
}

#[get("/siteInformation?<url>")]
async fn site_information(mut db: Connection<Sites>, url: String) -> Json<SiteInformation> {
    let information: Result<SqliteRow, _> = sqlx::query("SELECT title, keywords, rank FROM sitedata WHERE url = ?")
        .bind(url)
        .fetch_one(&mut *db)
        .await;

    let information = match information {
        Ok(information) => information,
        Err(_) => {
            return Json(SiteInformation {
                title: "".to_string(),
                keywords: vec![],
                rank: 0
            })
        }
    };

    let site_information: SiteInformation = SiteInformation {
        title: information.get::<&str, &str>("title").to_string(),
        keywords: information.get::<&str, &str>("keywords").to_string().split(",").map(|x| x.to_string()).collect(),
        rank: information.get::<u32, &str>("rank")
    };

    Json(site_information)
}

#[get("/search?<query>&<page>")]
async fn search(mut db: Connection<Sites>, query: String, page: u32) -> Json<SearchResults> {
    // Get sites from the database with similar titles
    let sites: Vec<SqliteRow> = sqlx::query("SELECT title, url FROM sitedata WHERE title LIKE ? ESCAPE '\\' ORDER BY rank DESC LIMIT 30 OFFSET ?")
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
        .mount("/api", routes![recommended, search, site_information])
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

enum IndexResult {
    Indexed,
    RetryLater,
    DoNotIndex,
    Unknown
}

impl IndexResult {
    fn is_unknown(&self) -> bool {
        match self {
            IndexResult::Unknown => true,
            _ => false
        }
    }
}

fn index_robots_txt() -> bool {
    return !std::env::args().any(|x| x == "--no-robots-txt");
}

async fn index_web(mut connection: PoolConnection<Sqlite>) {
    println!("Indexing web...");
    // Look into the database 'sites' and find the staging table.
    // If it's empty, then create it.
    // Turn the PoolConnection<Sqlite> into a SqlitePool

    // First, get the database connection from rocket.
    // Then, make sure the staging table exists.

    if std::env::args().any(|x| x == "--purge-staging") {
        println!("Purging staging table...");
        let _ = sqlx::query("DELETE FROM staging").execute(&mut connection).await;
    }

    sqlx::query("CREATE TABLE IF NOT EXISTS staging (url TEXT PRIMARY KEY, temprank INTEGER, domain TEXT);")
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

    // Create the donotindex table if it doesn't exist.
    // This table has a url (text) as the primary key.
    sqlx::query("CREATE TABLE IF NOT EXISTS donotindex (url TEXT PRIMARY KEY);")
        .execute(&mut connection)
        .await
        .unwrap();

    // Create the robotstxt table if it doesn't exist.
    // This table has a url (text) as the primary key, a text field for the content of the robots.txt file, and a timestamp for when it was first added.
    sqlx::query("CREATE TABLE IF NOT EXISTS robotstxt (url TEXT PRIMARY KEY, content TEXT, firstadded INTEGER);")
        .execute(&mut connection)
        .await
        .unwrap();

    let default_site: String;

    if std::env::args().any(|x| x == "--default-site") {
        default_site = std::env::args().nth(std::env::args().position(|x| x == "--default-site").unwrap() + 1).unwrap();
    } else {
        default_site = "https://github.com/".to_string();
    }

    // Check if there are any sites in the sitedata table. If not, this is the first time we've started to index the web.
    // If there are sites, then we can skip adding the default site to the staging table.
    let rows: u32 = sqlx::query("SELECT COUNT(*) FROM staging;")
        .fetch_one(&mut connection)
        .await
        .unwrap()
        .get(0);

    if rows == 0 {
        // Add the default site to the staging table.
        sqlx::query("INSERT INTO staging (url, temprank, domain) VALUES (?, ?, ?);")
            .bind(default_site.clone())
            .bind(0)
            .bind(get_domain(&default_site))
            .execute(&mut connection)
            .await
            .unwrap();
        debugMessage!("Added {} to the staging table.", default_site);
    }

    // Awesome! We can start indexing the web.
    index_staged_sites(connection).await;
}

async fn index_staged_sites(mut connection: PoolConnection<Sqlite>) {
    // Get the first CONCURRENT_INDEXING_AMOUNT of sites from the staging table.
    // Every site should be a different domain (the domain column in the staging table).
    let mut rows: Vec<SqliteRow> = sqlx::query("SELECT * FROM staging WHERE url IN (SELECT MIN(url) FROM staging GROUP BY domain) LIMIT ?;")
        .bind(CONCURRENT_INDEXING_AMOUNT)
        .fetch_all(&mut connection)
        .await
        .unwrap();

    let mut are_staged_sites: bool = true;

    let sites_scanned: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));
    let total_sites_scanned: Arc<Mutex<u64>> = Arc::new(Mutex::new(0));

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(15))
        .user_agent(USER_AGENT)
        .build()
        .unwrap();

    while are_staged_sites {
        // Remove the sites from the staging table.
        let sites: Vec<(String, u32)> = rows.iter().map(|row| {
            (row.try_get::<&str, &str>("url").unwrap_or("Unknown").to_string(),
             row.try_get::<u32, &str>("temprank").unwrap_or(0))
        }).collect();

        let url_sql: String = sites.iter().map(|(url, _)| format!("'{}'", url.replace("'", "''"))).collect::<Vec<String>>().join(", ");

        // TODO: This could potentially create a SQL injection vulnerability with the right URL, but .bind doesn't work with IN.
        //       Possibly because .bind adds quotes around a string and IN takes it as a single string instead of multiple strings?
        let amount: usize = sqlx::query(format!("DELETE FROM staging WHERE url IN ({});", url_sql).as_str())
            .execute(&mut connection)
            .await
            .unwrap()
            .rows_affected() as usize;

        debugMessage!("Removed {} sites from the staging table.", amount);

        // Download the sites concurrently.
        let bodies = stream::iter(sites)
            .map(|(url, rank)| {
                let client = &client;
                async move {
                    let resp = client.get(url.clone()).send().await?;
                    Ok::<(String, reqwest::Response, u32), reqwest::Error>((url, resp, rank))
                }
            })
            .buffer_unordered(CONCURRENT_INDEXING_AMOUNT as usize);

        // Temporarily put connection into a Mutex so we can use it in the async block.
        let new_connection = Arc::new(tokio::sync::Mutex::new(&mut connection));

        bodies
            .for_each(|b: Result<(String, reqwest::Response, u32), reqwest::Error>| async {
                match b {
                    Ok(b) => {
                        let url: String = b.0;
                        let body: reqwest::Response = b.1;
                        let rank: u32 = b.2;

                        // Add 1 to the sites scanned (sites_scanned is an Arc<Mutex<u64>>).
                        *sites_scanned.lock().unwrap() += 1;

                        // Add 1 to the total sites scanned (total_sites_scanned is an Arc<Mutex<u64>>).
                        *total_sites_scanned.lock().unwrap() += 1;
                        
                        debugMessage!("Downloaded {}: {}/{} concurrent requests completed. Total sites scanned: {}", url, *sites_scanned.lock().unwrap(), CONCURRENT_INDEXING_AMOUNT, *total_sites_scanned.lock().unwrap());
                        
                        let mut retry: IndexResult = IndexResult::Unknown;

                        let base_url: String = url.split("/").collect::<Vec<&str>>()[..3].join("/");

                        'robotstxt: { if index_robots_txt() {
                            // Check if the site already has a cached robots.txt file.
                            let robots_txt: Option<String> = sqlx::query("SELECT content FROM robotstxt WHERE url = ?;")
                                .bind(base_url.clone())
                                .fetch_one(&mut new_connection.lock().await as &mut PoolConnection<Sqlite>)
                                .await
                                .ok()
                                .map(|row| row.try_get::<&str, &str>("content").unwrap_or("Unknown").to_string());
    
                            // If the site doesn't have a cached robots.txt file, download it.
                            let robots_txt: String = match robots_txt {
                                Some(robots_txt) => robots_txt,
                                None => {
                                    let robots_txt_url: String = format!("{}/robots.txt", base_url.clone());
    
                                    let resp = client.get(robots_txt_url.clone()).send().await;

                                    let resp = match resp {
                                        Ok(resp) => resp,
                                        Err(_) => {
                                            break 'robotstxt;
                                        }
                                    };
    
                                    let robots_txt = resp.text().await;

                                    let robots_txt = match robots_txt {
                                        Ok(robots_txt) => robots_txt,
                                        Err(err) => {
                                            debugMessage!("Error downloading {}: {}", robots_txt_url, err);
                                            break 'robotstxt;
                                        }
                                    };
    
                                    // Add the robots.txt file to the robotstxt table.
                                    sqlx::query("INSERT INTO robotstxt (url, content, firstadded) VALUES (?, ?, ?);")
                                        .bind(base_url.clone())
                                        .bind(robots_txt.clone())
                                        .bind(SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as u32)
                                        .execute(&mut new_connection.lock().await as &mut PoolConnection<Sqlite>)
                                        .await
                                        .unwrap();
    
                                    debugMessage!("Added {} to the robotstxt table.", base_url.clone());
    
                                    robots_txt
                                }
                            };
    
                            // Check if the site is allowed to be crawled.
                            let robots = Robots::from_str_lossy(&robots_txt);
                            
                            let matcher = SimpleMatcher::new(&robots.choose_section(USER_AGENT).rules);
    
                            if !matcher.check_path(base_url.as_str()) {
                                // The URL is not allowed to be crawled.
                                println!("{} is not allowed to be crawled.", url);
                                retry = IndexResult::DoNotIndex;
                            }
                        } };

                        if retry.is_unknown() {
                            retry = index_site(url.as_str(), rank, body, new_connection.lock().await).await;
                        }

                        match retry {
                            IndexResult::RetryLater => {
                                // Add the site back to the staging table.
                                sqlx::query("INSERT INTO staging (url, temprank, domain) VALUES (?, ?, ?);")
                                    .bind(url.clone())
                                    .bind(rank)
                                    .bind(get_domain(url.as_str()))
                                    .execute(&mut new_connection.lock().await as &mut PoolConnection<Sqlite>)
                                    .await
                                    .unwrap();
                                debugMessage!("Added {} back to the staging table.", url);
                            }
                            IndexResult::DoNotIndex => {
                                // Add the site to the donotindex table.
                                let result = sqlx::query("INSERT INTO donotindex (url) VALUES (?);")
                                    .bind(url.clone())
                                    .execute(&mut new_connection.lock().await as &mut PoolConnection<Sqlite>)
                                    .await;
                                
                                if result.is_ok() {
                                    debugMessage!("Added {} to the donotindex table.", url);
                                } else {
                                    debugMessage!("Error adding {} to the donotindex table: {}", url, result.err().unwrap());
                                }
                            }
                            IndexResult::Indexed => {
                                debugMessage!("Indexed {}.", url);
                            }
                            IndexResult::Unknown => {
                                debugMessage!("Unknown result for {}.", url);
                            }
                        }
                    },
                    Err(ref e) => {
                        debugMessage!("Found an error when indexing site: {}", e);
                    },
                }
            })
            .await;

        *sites_scanned.lock().unwrap() = 0;

        debugMessage!("Finished indexing set of {} staged sites.", CONCURRENT_INDEXING_AMOUNT);

        // Check if there are any more sites in the staging table.
        rows = sqlx::query("SELECT * FROM staging WHERE url IN (SELECT MIN(url) FROM staging GROUP BY domain) LIMIT ?;")
            .bind(CONCURRENT_INDEXING_AMOUNT)
            .fetch_all(&mut connection)
            .await
            .unwrap();
        
        if rows.is_empty() {
            debugMessage!("No more sites in the staging table.");
            are_staged_sites = false;
        }
    }

    // We're done indexing the web!
    println!("Done indexing the web! This probably shouldn't have been called unless this was left running for a ridiculous amount of time or something went wrong.");
}

// The return value of this function determines whether we should retry the request or not.
async fn index_site(url: &str, rank: u32, body: reqwest::Response, mut connection: tokio::sync::MutexGuard<'_, &mut PoolConnection<Sqlite>>) -> IndexResult {
    // 1. Download the site.
    // Use reqwest to download the site.

    if !body.status().is_success() {
        // Check if the status code is 429, and find out how long we should wait before trying again.
        if body.status().as_u16() == 429 {
            let retry_after_header: Option<&HeaderValue> = body.headers().get("Retry-After");
            let retry_after: u64;
            if retry_after_header.is_none() {
                retry_after = 60;
            } else {
                retry_after = retry_after_header.unwrap().to_str().unwrap_or("60").parse().unwrap_or(60);
            }

            debugMessage!("Got a 429 from {}, retrying in {} seconds.", url, retry_after);
            tokio::time::sleep(Duration::from_secs(retry_after)).await;
            return IndexResult::RetryLater;
        }

        debugMessage!("Error downloading site: {}. Response code: {}. Continuing to next site.", url, body.status().as_str());
        return IndexResult::DoNotIndex;
    }
    
    // Make sure the result is a website and not a different file format.
    let content_type: Option<&HeaderValue> = body.headers().get("Content-Type");

    if content_type.is_none() {
        debugMessage!("No content type header for {}. Continuing to next site.", url);
        return IndexResult::DoNotIndex;
    }

    if !content_type.unwrap().to_str().unwrap_or("").contains("text/html") {
        debugMessage!("{} is not a website. Continuing to next site.", url);
        return IndexResult::DoNotIndex;
    }

    let (title, keywords, links): (String, String, Vec<(String, bool)>) = {
        let text = body.text().await;

        if text.is_err() {
            debugMessage!("Error getting text from site: {}. Continuing to next site.", url);
            return IndexResult::DoNotIndex;
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

        // 5. Add links to the staging table if they aren't already in the staging table or the sitedata table.
        let links: Vec<(String, bool)> = document.select(&Selector::parse("a").unwrap()).map(|link| {
            let mut should_impact_ranking: bool = true;
            // Check if the link has rel="nofollow" or rel="external" or rel="ugc" or rel="sponsored". If so,
            // don't impact the ranking of the linked site.
            let rel: Option<&str> = link.value().attr("rel");
            if rel.is_some() {
                let rel: &str = rel.unwrap();
                if rel.contains("nofollow") || rel.contains("ugc") || rel.contains("sponsored") {
                    should_impact_ranking = false;
                }
            }
            let value: &str = link.value().attr("href").unwrap_or("");
            let absolute_url: String = parse_relative_url(url, value);
            (absolute_url, should_impact_ranking)
        }).collect();
        
        let links: Vec<(String, bool)> = links.iter().filter(|link| {
            !link.0.is_empty()
        }).map(|link| {
            (link.0.to_string(), link.1)
        }).collect();

        // Deduplicate the links.
        let links: Vec<(String, bool)> = links.iter().cloned().collect::<HashSet<(String, bool)>>().into_iter().collect();

        (title, keywords, links)
    };

    // 6. Add the site to the sitedata table.
    let result = sqlx::query("INSERT INTO sitedata (url, title, keywords, rank) VALUES (?, ?, ?, ?);")
        .bind(url)
        .bind(title)
        .bind(keywords)
        .bind(rank)
        .execute(&mut connection as &mut PoolConnection<Sqlite>)
        .await;

    if result.is_err() {
        debugMessage!("Error adding {} to sitedata table: {}", url, result.err().unwrap());
        return IndexResult::DoNotIndex;
    }

    debugMessage!("Inserting new links into the staging table.");

    for link in links {
        let rows = sqlx::query("SELECT * FROM donotindex WHERE url = ?;")
            .bind(link.0.as_str())
            .fetch_all(&mut connection as &mut PoolConnection<Sqlite>)
            .await
            .unwrap();
        if rows.is_empty() {
            let rows = sqlx::query("SELECT * FROM staging WHERE url = ?;")
                .bind(link.0.clone())
                .fetch_all(&mut connection as &mut PoolConnection<Sqlite>)
                .await
                .unwrap();
            if rows.is_empty() {
                let rows = sqlx::query("SELECT * FROM sitedata WHERE url = ?;")
                    .bind(link.0.clone())
                    .fetch_all(&mut connection as &mut PoolConnection<Sqlite>)
                    .await
                    .unwrap();
                if rows.is_empty() {
                    sqlx::query("INSERT INTO staging (url, temprank, domain) VALUES (?, ?, ?);")
                        .bind(link.0.clone())
                        .bind(0)
                        .bind(get_domain(&link.0))
                        .execute(&mut connection as &mut PoolConnection<Sqlite>)
                        .await
                        .unwrap();
                } else {
                    if !link.1 {
                        continue;
                    }
                    // Increase the rank of the site by 1.
                    sqlx::query("UPDATE sitedata SET rank = rank + 1 WHERE url = ?;")
                        .bind(link.0)
                        .execute(&mut connection as &mut PoolConnection<Sqlite>)
                        .await
                        .unwrap();
                }
            } else {
                if !link.1 {
                    continue;
                }
                // Increase the temprank of the staged site by 1.
                sqlx::query("UPDATE staging SET temprank = temprank + 1 WHERE url = ?;")
                    .bind(link.0)
                    .execute(&mut connection as &mut PoolConnection<Sqlite>)
                    .await
                    .unwrap();
            }
        }
    }

    return IndexResult::Indexed;
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

// This function takes in a string as a site and gives back the domain name,
// not including the protocol, path, query parameters, or hash parameters.
fn get_domain(path: &str) -> &str {
    let path = path.split('/').collect::<Vec<&str>>();
    if path.len() < 3 {
        return "";
    }
    path[2]
}