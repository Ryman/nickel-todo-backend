#![feature(phase, if_let)]
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
use serialize::json::ToJson;
use serialize::json;
use std::collections::TreeMap;

#[deriving(Decodable)]
pub struct Todo {
    uid: Option<i32>,
    title: Option<String>,
    order: Option<i32>,
    completed: Option<bool>,
}

impl Todo {
    pub fn title(&self) -> &str {
        match self.title {
            Some(ref title) => title.as_slice(),
            None => ""
        }
    }

    pub fn order(&self) -> i32 {
        self.order.unwrap_or(0)
    }

    pub fn completed(&self) -> bool {
        self.completed.unwrap_or(false)
    }

    pub fn merge(&mut self, other: Todo) {
        if other.title.is_some() {
            self.title = other.title;
        }
        if other.order.is_some() {
            self.order = other.order
        }
        if other.completed.is_some() {
            self.completed = other.completed
        }
    }
}

// Specify encoding method manually
impl ToJson for Todo {
    fn to_json(&self) -> json::Json {
        let mut d = TreeMap::new();
        // All standard types implement `to_json()`, so use it
        d.insert("title".to_string(), self.title().to_string().to_json());
        d.insert("order".to_string(), self.order().to_json());
        d.insert("completed".to_string(), self.completed().to_json());

        if let Some(uid) = self.uid {
            d.insert("uid".to_string(), uid.to_json());
            // FIXME: use base_url from a config
            d.insert("url".to_string(), format!("http://2588bf84.ngrok.com/todos/{}", uid).to_json());
        }

        json::Object(d)
    }
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

fn item_options_handler(req: &Request, res: &mut Response) {
    res.origin.headers.insert_raw("Access-Control-Allow-Methods".to_string(), b"GET,PATCH,HEAD,DELETE,OPTIONS");
}

fn patch_handler(request: &Request, response: &mut Response) {
    println!("Handling PATCH/POST")
    println!("{}", request.origin.body.as_slice());

    if let Some(mut todo) = find_todo(request, response) {
        let db_conn = request.db_conn();
        let mut diff = request.json_as::<Todo>().unwrap();

        println!("BEFORE: {}", json::encode(&todo.to_json()))
        todo.merge(diff);
        println!("AFTER: {}", json::encode(&todo.to_json()))

        let stmt = db_conn.prepare("UPDATE todos SET title = $1, order_idx = $2, completed = $3 WHERE uid = $4").unwrap();

        let changes = stmt.execute([&todo.title() as &ToSql,
                                   &todo.order() as &ToSql,
                                   &todo.completed() as &ToSql,
                                   &todo.uid.unwrap() as &ToSql]).unwrap();

        if changes == 0 {
            response.origin.status = http::status::NotFound;
        } else if changes > 1 {
            println!("INTERNAL SERVER ERROR")
            response.origin.status = http::status::InternalServerError;
        } else {
            response.send(json::encode(&todo.to_json()))
        }
    }
}

fn find_todo(request: &Request, response: &mut Response) -> Option<Todo> {
    let db_conn = request.db_conn();
    let stmt = db_conn.prepare("SELECT uid, title, order_idx, completed FROM todos WHERE uid = $1").unwrap();
    let uid = from_str::<i32>(request.param("uid").trim());
    let mut iter = stmt.query([&uid as &ToSql]).unwrap();

    match (iter.next(), iter.next()) {
        (Some(select), None) => {
            let todo = Todo {
                uid: select.get(0u),
                title: Some(select.get(1u)),
                order: select.get(2u),
                completed: select.get(3u),
            };
            Some(todo)
        }
        // Just a 404
        (None, None) => {
            response.origin.status = http::status::NotFound;
            None
        }
        // Shouldn't get multiple for a uid
        (Some(_), Some(_)) | (None, Some(_)) => {
            println!("BADBAD: {} gave multiple results", uid)
            response.origin.status = http::status::InternalServerError;
            None
        }
    }
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

    server.utilize(Nickel::json_body_parser());

    server.utilize(Nickel::query_string());

    let mut router = router! {
        get "/todos" => |request, response| {
            let db_conn = request.db_conn();
            let stmt = db_conn.prepare("SELECT uid, title, order_idx, completed FROM todos").unwrap();

            let todos = stmt.query([]).unwrap().map(|select| {
                Todo {
                    uid: select.get(0u),
                    title: select.get(1u),
                    order: select.get(2u),
                    completed: select.get(3u),
                }
            }).collect::<Vec<_>>();

            response.send(json::encode(&todos.to_json()))
        }

        get "/todos/:uid" => |request, response| {
            if let Some(todo) = find_todo(request, response) {
                response.send(json::encode(&todo.to_json()))
            }
        }

        post "/todos" => |request, response| {
            println!("{}", request.origin.body.as_slice());

            let mut todo = request.json_as::<Todo>().unwrap();
            let db_conn = request.db_conn();
            let stmt = db_conn.prepare("INSERT INTO todos (title, order_idx, completed) VALUES ( $1, $2, $3 ) RETURNING uid").unwrap();

            let mut iter = stmt.query([&todo.title() as &ToSql,
                                       &todo.order() as &ToSql,
                                       &todo.completed() as &ToSql]).unwrap();

            match (iter.next(), iter.next()) {
                (Some(select), None) => todo.uid = Some(select.get(0u)),
                // Should have one and only one uid from an insert
                (Some(_), Some(_)) | (None, Some(_)) | (None, None) => {
                    response.origin.status = http::status::InternalServerError;
                    return
                }
            }

            response.send(json::encode(&todo.to_json()))
        }

        delete "/todos" => |request, response| {
            let db_conn = request.db_conn();
            let deletes = db_conn.execute("TRUNCATE todos", []).unwrap();

            println!("DELETED ALL TODOS");
            response.send("{}")
        }

        delete "/todos/:uid" => |request, response| {
            let db_conn = request.db_conn();
            let uid = from_str::<i32>(request.param("uid").trim()).unwrap();
            let deletes = db_conn.execute("DELETE FROM todos * WHERE uid = $1", [&uid as &ToSql]).unwrap();

            if deletes == 0 {
                response.origin.status = http::status::NotFound;
            } else if deletes > 1 {
                response.origin.status = http::status::InternalServerError;
            } else {
                println!("DELETED TODO {}", uid);
                response.send("{}")
            }
        }
    };

    router.add_route(method::Patch, "/todos/:uid", patch_handler);
    router.add_route(method::Post, "/todos/:uid", patch_handler);
    router.add_route(method::Options, "/todos", options_handler);
    router.add_route(method::Options, "/todos/:uid", item_options_handler);

    server.utilize(router);

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
    //db_conn.execute("DROP TABLE IF EXISTS todos;", []).unwrap();
    db_conn.execute("CREATE TABLE IF NOT EXISTS todos (
            uid SERIAL PRIMARY KEY,
            title VARCHAR NOT NULL,
            order_idx INTEGER DEFAULT 0,
            completed BOOL DEFAULT FALSE

    )", []).unwrap();
}
