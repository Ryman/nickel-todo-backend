#![allow(unused_imports)]
#[macro_use] extern crate nickel;
extern crate rustc_serialize;

extern crate hyper;
extern crate postgres;
extern crate nickel_postgres;
extern crate openssl;
extern crate time;
#[macro_use] extern crate lazy_static;
extern crate r2d2;

use nickel::status::StatusCode;
use nickel::{
    Nickel, NickelError, Continue, Halt, Request, Response, ResponseFinalizer,
    QueryString, JsonBody, StaticFilesHandler, MiddlewareResult, HttpRouter
};

use std::env;
use hyper::method::Method;
use hyper::header;
use nickel_postgres::{PostgresMiddleware, PostgresRequestExtensions};
use postgres::types::ToSql;
use openssl::ssl;
use time::Timespec;
use rustc_serialize::json::{self, Json, ToJson};
use std::collections::BTreeMap;

lazy_static! {
    static ref SITE_ROOT_URL: String = {
        let mut root = env::var("SITE_ROOT_URL").unwrap_or_else(|_| "http://0.0.0.0:6767/".to_string());

        if root.len() == 0 {
            panic!("Cannot supply an empty `SITE_ROOT_URL`")
        }

        // Ensure slash termination
        if root.as_bytes().last() != Some(&b'/') { root.push('/'); }
        root
    };
}

#[derive(RustcDecodable)]
pub struct Todo {
    uid: Option<i32>,
    title: Option<String>,
    order: Option<i32>,
    completed: Option<bool>,
}

impl Todo {
    pub fn title(&self) -> &str {
        match self.title {
            Some(ref title) => &title[..],
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
        let mut d = BTreeMap::new();
        // All standard types implement `to_json()`, so use it
        d.insert("title".to_string(), self.title().to_string().to_json());
        d.insert("order".to_string(), self.order().to_json());
        d.insert("completed".to_string(), self.completed().to_json());

        if let Some(uid) = self.uid {
            d.insert("uid".to_string(), uid.to_json());
            // FIXME: use base_url from a config
            d.insert("url".to_string(), format!("{}todos/{}", *SITE_ROOT_URL, uid).to_json());
        }

        Json::Object(d)
    }
}

fn find_todo(request: &Request, response: &mut Response) -> Option<Todo> {
    let db_conn = request.db_conn();
    let stmt = db_conn.prepare("SELECT uid, title, order_idx, completed FROM todos WHERE uid = $1").unwrap();
    let uid = request.param("uid").trim().parse::<i32>().unwrap();
    let mut iter = stmt.query(&[&uid as &ToSql]).unwrap().into_iter();

    match (iter.next(), iter.next()) {
        (Some(select), None) => {
            let todo = Todo {
                uid: select.get(0),
                title: Some(select.get(1)),
                order: select.get(2),
                completed: select.get(3),
            };
            Some(todo)
        }
        // Just a 404
        (None, None) => None,
        // Shouldn't get multiple for a uid
        (Some(_), Some(_)) | (None, Some(_)) => {
            println!("BADBAD: {:?} gave multiple results", uid);
            None
        }
    }
}

#[cfg(not(test))]
fn main() {
    let mut server = Nickel::new();

    server.utilize(middleware! { |req, res|
        println!("logging request: {} => {}",
                 request.origin.method,
                 request.origin.request_uri);
    });

    server.utilize(middleware! { |req, res|
        let headers = response.origin.headers_mut();
        headers.set(header::AccessControlAllowHeaders(vec!["content-type"]));
        headers.set(header::AccessControlAllowOrigin::Any);
    });
    let ssl_context = ssl::SslContext::new(ssl::Tlsv1).unwrap();
    let postgres_url = env::var("DATABASE_URL").unwrap();
    let postgres_middleware = PostgresMiddleware::new(postgres_url.as_slice(), postgres::PreferSsl(ssl_context), 5, r2d2::NoopErrorHandler);
    initialise_db(&postgres_middleware);
    server.utilize(postgres_middleware);

    server.utilize(Nickel::json_body_parser());

    server.utilize(Nickel::query_string());

    let mut router = router! {
        get "/todos" => |request, response| {
            let db_conn = request.db_conn();
            let stmt = db_conn.prepare("SELECT uid, title, order_idx, completed FROM todos").unwrap();

            let todos = stmt.query([]).unwrap().map(|select| {
                Todo {
                    uid: select.get(0),
                    title: select.get(1),
                    order: select.get(2),
                    completed: select.get(3),
                }
            }).collect::<Vec<_>>();

            todos.to_json()
        }

        get "/todos/:uid" => |request, response| {
            if let Some(todo) = find_todo(request, response) {
                todo.to_json()
            } else {
                Json::Object("")
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
                (Some(select), None) => {
                    todo.uid = Some(select.get(0u));
                    (StatusCode::Ok, json::encode(&todo.to_json()))
                },
                // Should have one and only one uid from an insert
                (Some(_), Some(_)) | (None, Some(_)) | (None, None) => {
                    (StatusCode::InternalServerError, "More than one update")
                }
            }
        }

        delete "/todos" => |request, response| {
            let db_conn = request.db_conn();
            let deletes = db_conn.execute("TRUNCATE todos", []).unwrap();

            println!("DELETED ALL TODOS");
            Json::Object("{}")
        }

        delete "/todos/:uid" => |request, response| {
            let db_conn = request.db_conn();
            let uid = request.param("uid").trim().parse::<i32>().unwrap();
            let deletes = db_conn.execute("DELETE FROM todos * WHERE uid = $1", [&uid as &ToSql]).unwrap();

            if deletes == 0 {
                (StatusCode::NotFound, "")
            } else if deletes > 1 {
                response.origin.status = StatusCode::InternalServerError;
                (StatusCode::InternalServerError, "More than one deletion?")
            } else {
                println!("DELETED TODO {}", uid);
                (StatusCode::Ok, "{}")
            }
        }
    };

    let patch_handler = middleware! { |request, response|
        println!("Handling PATCH/POST");
        println!("{}", request.origin.body.as_slice());

        if let Some(mut todo) = find_todo(request, response) {
            let db_conn = request.db_conn();
            let diff = request.json_as::<Todo>().unwrap();

            println!("BEFORE: {:?}", json::encode(&todo.to_json()));
            todo.merge(diff);
            println!("AFTER: {:?}", json::encode(&todo.to_json()));

            let stmt = db_conn.prepare("UPDATE todos SET title = $1, order_idx = $2, completed = $3 WHERE uid = $4").unwrap();

            let changes = stmt.execute([&todo.title() as &ToSql,
                                       &todo.order() as &ToSql,
                                       &todo.completed() as &ToSql,
                                       &todo.uid.unwrap() as &ToSql]).unwrap();

            if changes == 0 {
                response.origin.status = StatusCode::NotFound;
            } else if changes > 1 {
                println!("INTERNAL SERVER ERROR");
                response.origin.status = StatusCode::InternalServerError;
            } else {
                return todo.to_json()
            }

            Json::Object("")
        }
    };

    router.add_route(Method::Patch, "/todos/:uid", patch_handler);
    router.add_route(Method::Post, "/todos/:uid", patch_handler);

    router.add_route(Method::Options, "/todos", middleware! { |req, res|
        let headers = response.origin.headers_mut();
        headers.set(header::AccessControlAllowMethods(vec![Method::Get,
                                                           Method::Head,
                                                           Method::Post,
                                                           Method::Delete,
                                                           Method::Options,
                                                           Method::Put]));
        ""
    });

    router.add_route(Method::Options, "/todos/:uid", middleware! { |req, res|
        let headers = response.origin.headers_mut();
        headers.set(header::AccessControlAllowMethods(vec![Method::Get,
                                                           Method::Patch,
                                                           Method::Head,
                                                           Method::Delete,
                                                           Method::Options]));
        ""
    });

    server.utilize(router);

    println!("Running server!");

    // Get port from heroku env
    let port = env::var("PORT").and_then(|s| s.trim().parse::<i32>()).unwrap_or(6767);
    println!("Binding to port: {}", port);
    server.listen(Ipv4Addr(0, 0, 0, 0), port);
}

//initialise database tables, if has not already been done
fn initialise_db(db_middleware: &PostgresMiddleware) {
    let db_conn = db_middleware.pool.get().unwrap();
    //db_conn.execute("DROP TABLE IF EXISTS todos;", []).unwrap();
    db_conn.execute("CREATE TABLE IF NOT EXISTS todos (
            uid SERIAL PRIMARY KEY,
            title VARCHAR NOT NULL,
            order_idx INTEGER DEFAULT 0,
            completed BOOL DEFAULT FALSE

    )", &[]).unwrap();
}
