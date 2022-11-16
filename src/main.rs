use rocket::fs::FileServer;

#[macro_use] extern crate rocket;

#[cfg(test)] mod tests;

// Try visiting:
//   http://127.0.0.1:8000/hello/world
#[get("/")]
fn world() -> &'static str {
    "Hello, world!"
}

// Try visiting:
//   http://127.0.0.1:8000/wave/Rocketeer/100
#[get("/<name>/<age>")]
fn wave(name: &str, age: isize) -> String {
    format!("👋 Hello, {} year old named {}!", age, name)
}

#[launch]
fn rocket() -> _ {
    rocket::build()
        .mount("/", FileServer::from("./public"))
        .mount("/test", routes![wave, world])
}
