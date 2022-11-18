use rocket::{tokio};
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
        .attach(AdHoc::on_liftoff("Index web", |rocket| Box::pin(async move {
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
    println!("Staging table created if it didn't exist.");

    // Create the sitedata table if it doesn't exist.
    // This table has a url (text) as the primary key, a title (text), an array of text for keywords, and a rank (integer).
    sqlx::query("CREATE TABLE IF NOT EXISTS sitedata (url TEXT PRIMARY KEY, title TEXT, keywords TEXT, rank INTEGER);")
        .execute(&mut connection)
        .await
        .unwrap();
    println!("Sitedata table created if it didn't exist.");

    // Check if there are any sites in the sitedata table. If not, this is the first time we've started to index the web.
    // If there are sites, then we can skip adding wikipedia to the staging table.
    let rows = sqlx::query("SELECT * FROM sitedata;")
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
}

async fn index_staged_sites(mut connection: PoolConnection<Sqlite>) {
    // Get the first 100 sites from the staging table and remove them from the staging table.
    // Then, index them.
    let mut rows = sqlx::query("SELECT * FROM staging LIMIT 100;")
        .fetch_all(&mut connection)
        .await
        .unwrap();

    let mut are_staged_sites: bool = true;

    while are_staged_sites {
        // Remove the sites from the staging table.
        for row in &rows {
            let url: String = row.get("url");
            println!("{}", url);
            // Make sure we don't move connection so it can be used in the next iteration of the loop.
            sqlx::query("DELETE FROM staging WHERE url = ?;")
                .bind(url.clone())
                .execute(&mut connection)
                .await
                .unwrap();

            println!("Removed {} from the staging table.", url);

            index_site(url.as_str(), &mut connection).await;
        }

        // Check if there are any more sites in the staging table.
        rows = sqlx::query("SELECT * FROM staging LIMIT 100;")
            .fetch_all(&mut connection)
            .await
            .unwrap();
        
        if rows.is_empty() {
            are_staged_sites = false;
        }
    }

    // We're done indexing the web!
    println!("Done indexing the web! This probably shouldn't have been called unless this was left running for a rediculous amount of time or something went wrong.");
}

async fn index_site<'a>(url: &str, mut connection: &mut PoolConnection<Sqlite>) {
    // 1. Download the site.
    // Use reqwest to download the site.
    let client = reqwest::Client::new();
    let body = client.get(url).send().await;

    println!("Downloaded {}", url);

    if body.is_err() {
        println!("Error downloading site: {}. Continuing to next site.", url);
        return;
    }

    let body = body.unwrap();

    // 2. Parse the site.
    println!("Parsing site: {}", url);
    let parsed_site = html_parser::Dom::parse(body.text().await.unwrap().as_str());

    if parsed_site.is_err() {
        println!("Error parsing site: {}. Moving on to next site.", url);
        // TODO: Record this site as having an error so we don't try to index it again.
        return;
    }

    let parsed_site = parsed_site.unwrap();
    
    // 3. Get the title of the site.
    let titles: Vec<html_parser::Element> = find_in_dom(parsed_site.clone(), |node| {
        node.name == "title".to_string()
    });

    let title: String;

    if titles.is_empty() {
        println!("No title found for site: {}. Setting title to the default of 'Site'.", url);
        title = String::from("Site");
    } else {
        title = match titles[0].children[0] {
            html_parser::Node::Text(ref contents) => contents.clone(),
            _ => "".to_string()
        };
    }

    // 4. Get the keywords of the site.
    let keyword_elements: Vec<html_parser::Element> = find_in_dom(parsed_site.clone(), |node| {
        node.name == "meta".to_string() && node.attributes.contains_key("name") && node.attributes["name"] == Some("keywords".to_string())
    });

    let keywords: String;

    if keyword_elements.is_empty() {
        println!("No keywords found for site: {}. Setting keywords to the default of 'None'.", url);
        keywords = String::from("None");
    } else {
        keywords = keyword_elements[0].attributes["content"].clone().unwrap();
    }

    // 5. Get the rank of the site.
    let rank = 0;
    // TODO: Rank other sites higher when we find links to them.

    // 6. Add the site to the sitedata table.
    sqlx::query("INSERT INTO sitedata (url, title, keywords, rank) VALUES (?, ?, ?, ?);")
        .bind(url)
        .bind(title)
        .bind(keywords)
        .bind(rank)
        .execute(&mut *connection)
        .await
        .unwrap();

    // 7. Add links to the staging table if they aren't already in the staging table or the sitedata table.
    let link_elements: Vec<html_parser::Element> = find_in_dom(parsed_site, |node| {
        node.name == "a"
    });
    // Go through the link_elements and turn them into strings of the href attribute.
    let mut links: Vec<String> = Vec::new();
    for link_element in link_elements {
        if link_element.attributes.contains_key("href") {
            let absolute_url: String = parse_relative_url(url, link_element.attributes["href"].clone().unwrap().as_str());
            links.push(absolute_url);
        }
    }
    
    let links: Vec<String> = links.iter().filter(|link| {
        !link.is_empty()
    }).map(|link| {
        link.to_string()
    }).collect();

    for link in links {
        let rows = sqlx::query("SELECT * FROM staging WHERE url = ?;")
            .bind(link.clone())
            .fetch_all(&mut *connection)
            .await
            .unwrap();
        if rows.is_empty() {
            let 
            rows = sqlx::query("SELECT * FROM sitedata WHERE url = ?;")
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

fn find_in_dom<F: Fn(&html_parser::Element) -> bool>(dom: html_parser::Dom, condition: F) -> Vec<html_parser::Element> {
    let mut elements: Vec<html_parser::Element> = Vec::new();
    let mut final_elements: Vec<html_parser::Element> = Vec::new();

    for node in dom.children {
        match node {
            html_parser::Node::Element(ref element) => {
                elements.push(element.clone());
            },
            _ => {}
        }
    }

    let mut elements_have_children: bool = true;
    while elements_have_children {
        elements_have_children = false;
        let mut to_push: Vec<html_parser::Element> = vec![];
        let mut to_remove: Vec<usize> = vec![];
        let mut i: usize = 0;
        for node in &elements {
            if condition(node) {
                final_elements.push(node.clone());
            }
            for child in node.children.clone() {
                match child {
                    html_parser::Node::Element(ref element) => {
                        to_push.push(element.clone());
                        elements_have_children = true;
                    },
                    _ => {}
                }
            }
            to_remove.push(i);
            i += 1;
        }

        for node in to_push {
            elements.push(node);
        }
        let mut j: usize = 0;
        for index in to_remove {
            elements.remove(index - j);
            j += 1;
        }
    }

    final_elements
}

fn parse_relative_url(base_url: &str, relative_url: &str) -> String {
    // TODO: Make this actually test for all cases.

    // Check if the relative_url is a full url.
    if relative_url.starts_with("http://") || relative_url.starts_with("https://") {
        return relative_url.to_string();
    }

    // Get the base url without the path.
    let base_url = base_url.split("/").collect::<Vec<&str>>();

    // Check if the relative_url is a relative url.
    if relative_url.starts_with("/") || relative_url.starts_with("./") {
        // Get the relative URL without the . if it has one.
        if relative_url.starts_with("./") {
            return format!("{}//{}/{}", base_url[0], base_url[2], &relative_url[2..]);
        } else {
            return format!("{}//{}/{}", base_url[0], base_url[2], &relative_url[1..]);
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
            return format!("http://{}/{}", site.unwrap().as_str(), path.unwrap().as_str());
        } else if site.is_some() {
            return format!("http://{}/", site.unwrap().as_str());
        }
    }

    // Otherwise, it doesn't have a base url.
    return format!("{}//{}/{}", base_url[0], base_url[2], &relative_url);
}