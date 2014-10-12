#![feature(phase)]
#![allow(unused_imports)]
extern crate http;
extern crate nickel;
extern crate serialize;
#[phase(plugin)] extern crate nickel_macros;
extern crate postgres;
extern crate nickel_postgres;
extern crate openssl;
extern crate time;

use http::status::NotFound;
use nickel::{
    Nickel, NickelError, ErrorWithStatusCode, Continue, Halt, Request, Response,
    QueryString, JsonBody, StaticFilesHandler, MiddlewareResult, HttpRouter
};
use std::io::net::ip::Ipv4Addr;
use std::os::getenv;
use http::method;
use nickel_postgres::{PostgresMiddleware, PostgresRequestExtensions};
use postgres::pool::PostgresConnectionPool;
use postgres::types::ToSql;
use openssl::ssl;
use time::Timespec;

// #[deriving(Decodable, Encodable)]
// struct Person {
//     firstname: String,
//     lastname:  String,
// }

#[deriving(Decodable,Encodable)]
pub struct Person {
    pub id: i32,
    pub name: String,
    pub created: Timespec
}

#[deriving(Decodable)]
pub struct PersonByPost {
    pub title: String
}

//this is an example middleware function that just logs each request
#[cfg(not(test))]
fn logger(request: &Request, _response: &mut Response) -> MiddlewareResult {
    println!("logging request: {} => {}", request.origin.method, request.origin.request_uri);

    // a request is supposed to return a `bool` to indicate whether additional
    // middleware should continue executing or should be stopped.
    Ok(Continue)
}

//this is how to overwrite the default error handler to handle 404 cases with a custom view
#[cfg(not(test))]
fn custom_404(err: &NickelError, _req: &Request, response: &mut Response) -> MiddlewareResult {
    match err.kind {
        ErrorWithStatusCode(NotFound) => {
            response.content_type("html")
                    .status_code(NotFound)
                    .send("<h1>Call the police!<h1>");
            Ok(Halt)
        },
        _ => Ok(Continue)
    }
}

//this is how to overwrite the default error handler to handle 404 cases with a custom view
#[cfg(not(test))]
fn enable_cors(_req: &Request, response: &mut Response) -> MiddlewareResult {
    response.origin.headers.insert_raw("Access-Control-Allow-Headers".to_string(), b"content-type");
    response.origin.headers.insert_raw("Access-Control-Allow-Origin".to_string(), b"*");
    Ok(Continue)
}

fn options_handler(req: &Request, res: &mut Response) {
    res.origin.headers.insert_raw("Access-Control-Allow-Methods".to_string(), b"GET,HEAD,POST,DELETE,OPTIONS,PUT");
}

#[cfg(not(test))]
fn main() {
    let mut server = Nickel::new();

    server.utilize(logger);

    server.utilize(enable_cors);
    let ssl_context = ssl::SslContext::new(ssl::Tlsv1).unwrap();
    let postgres_url = getenv("DATABASE_URL").unwrap();
    let postgres_middleware: PostgresMiddleware = PostgresMiddleware::new(postgres_url.as_slice(), postgres::PreferSsl(ssl_context), 5);
    initialise_db_tables(postgres_middleware.pool.clone());
    server.utilize(postgres_middleware);

    server.add_route(method::Options, "/todos", options_handler);

    server.utilize(Nickel::json_body_parser());

    server.utilize(Nickel::query_string());

    server.utilize(router! {
        get "/todos" => |request, response| {
            // if env.has_key? "HTTP_ACCESS_CONTROL_REQUEST_HEADERS"

            let db_conn = request.db_conn();
            let stmt = db_conn.prepare("SELECT id, name, created FROM person").unwrap();

            let mut iter = stmt.query([]).unwrap();

            let mut persons: Vec<Person> = Vec::new();
            for select in iter {
                let person = Person {
                    id: select.get(0u),
                    name: select.get(1u),
                    created: select.get(2u)
                };
                persons.push(person);
            }
            let num_persons = persons.len();
            if num_persons == 0 {
                response.origin.status = http::status::Ok;
                response.send("{}");
            }
            else {
                response.origin.status = http::status::Ok;
                response.send("{many}");
            }
        }

        post "/todos" => |request, response| {
            println!("{}", request.origin.body.as_slice());
            // new_todo = json_body
            // stored_todo = @repo.add_todo(new_todo)
            // headers["Location"] = todo_url(stored_todo)
            // status 201
            // # content_type :json
            // todo_repr(stored_todo).to_json
            //response.send(r#"{"title": "a todo"}"#)
            let person: PersonByPost = request.json_as::<PersonByPost>().unwrap();
            let db_conn = request.db_conn();
            let inserts = db_conn.execute("INSERT INTO person (name, created) VALUES ( $1, $2 )",
                                        [&person.title.as_slice() as &ToSql, &time::get_time() as &ToSql]).unwrap();
            if inserts == 0 {
                response.origin.status = http::status::NotFound;
            }
            else if inserts > 1 {
                response.origin.status = http::status::InternalServerError;
            }
            response.send(format!("{} persons were inserted", inserts).as_slice())
        }
    });

    server.handle_error(custom_404);

    println!("Running server!");

    // Get port from heroku env
    let port = getenv("PORT").and_then(|s| from_str::<u16>(s.as_slice().trim())).unwrap_or(6767);
    println!("Binding to port: {}", port)
    server.listen(Ipv4Addr(0, 0, 0, 0), port);
}

//initialise database tables, if has not already been done
fn initialise_db_tables (db_pool_instance: PostgresConnectionPool) {
    let db_conn = db_pool_instance.get_connection();
    db_conn.execute("CREATE TABLE IF NOT EXISTS person (
            id SERIAL PRIMARY KEY,
            name VARCHAR NOT NULL,
            created TIMESTAMP NOT NULL
    )", []).unwrap();
    db_conn.execute("CREATE TABLE IF NOT EXISTS post (
            id SERIAL PRIMARY KEY,
            title VARCHAR NOT NULL,
            text VARCHAR NOT NULL
    )", []).unwrap();
    db_conn.execute("CREATE TABLE IF NOT EXISTS comment (
            id SERIAL PRIMARY KEY,
            text VARCHAR NOT NULL,
            post_id SERIAL REFERENCES post (id)
    )", []).unwrap();
}
