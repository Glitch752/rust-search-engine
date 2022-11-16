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

#[get("/")]
fn world() -> &'static str {
    "Hello, world!"
}

#[get("/recommended?<query>")]
fn recommended(query: Option<&str>) -> Json<Vec<Recommendation<String>>> {
    // Return JSON array of recommended search terms for the query
    // For now, just use the query and "bar"
    let results = vec![Recommendation::new(query.unwrap_or("foo")), Recommendation::new("bar")];
    Json(results)
}

// #[get("/<name>/<age>")]
// fn wave(name: &str, age: isize) -> String {
//     format!("👋 Hello, {} year old named {}!", age, name)
// }

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", FileServer::from("./public"))
        .mount("/api", routes![recommended])
}
