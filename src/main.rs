#[macro_use] extern crate lazy_static;
#[macro_use] extern crate nickel;
extern crate nickel_postgres;
extern crate rustc_serialize;
extern crate openssl;
extern crate postgres;
extern crate unicase;
extern crate hyper;
extern crate r2d2;

use nickel::{
    Nickel, Request, Response, QueryString, ResponseFinalizer,
    HttpRouter, JsonBody, MiddlewareResult
};
use nickel::status::StatusCode;

use std::env;
use hyper::method::Method;
use hyper::header;
use unicase::UniCase;
use nickel_postgres::{PostgresMiddleware, PostgresRequestExtensions};
use postgres::SslMode;
use openssl::ssl::{SslMethod, SslContext};
use rustc_serialize::json::{self, Json, ToJson};
use std::collections::BTreeMap;

lazy_static! {
    static ref SITE_ROOT_URL: String = {
        let mut root = env::var("SITE_ROOT_URL").unwrap_or_else(|_| "http://0.0.0.0:6767/".to_string());

        if root.is_empty() {
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
        self.title.as_ref().map_or("", |s| &*s)
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

impl<'a> From<postgres::Row<'a>> for Todo {
    fn from(row: postgres::Row) -> Todo {
        Todo {
            uid: row.get(0),
            title: Some(row.get(1)),
            order: row.get(2),
            completed: row.get(3),
        }
    }
}

// Specify encoding method manually
impl ToJson for Todo {
    fn to_json(&self) -> json::Json {
        let mut d = BTreeMap::new();
        // All standard types implement `to_json()`, so use it
        d.insert("title".to_string(), self.title().to_json());
        d.insert("order".to_string(), self.order().to_json());
        d.insert("completed".to_string(), self.completed().to_json());

        if let Some(uid) = self.uid {
            d.insert("uid".to_string(), uid.to_json());
            d.insert("url".to_string(), format!("{}todos/{}", *SITE_ROOT_URL, uid).to_json());
        }

        Json::Object(d)
    }
}

fn find_todo(request: &Request) -> Option<Todo> {
    let db_conn = request.db_conn();
    let stmt = db_conn.prepare("SELECT uid, title, order_idx, completed FROM todos WHERE uid = $1").unwrap();
    let uid = request.param("uid").trim().parse::<i32>().unwrap();
    let mut iter = stmt.query(&[&uid]).unwrap().into_iter();

    match (iter.next(), iter.next()) {
        (Some(row), None) => Some(Todo::from(row)),
        // Just a 404
        (None, None) => None,
        // Shouldn't get multiple for a uid
        (Some(_), Some(_)) | (None, Some(_)) => {
            println!("BADBAD: {:?} gave multiple results", uid);
            None
        }
    }
}

fn patch_handler<'a>(request: &mut Request, response: Response<'a>) -> MiddlewareResult<'a> {
    match find_todo(request) {
        None => response.error(StatusCode::NotFound, ""),
        Some(mut todo) => {
            let diff = request.json_as::<Todo>().unwrap();
            todo.merge(diff);

            let db_conn = request.db_conn();
            let stmt = db_conn.prepare("UPDATE todos SET title = $1, order_idx = $2, completed = $3 WHERE uid = $4").unwrap();
            let changes = stmt.execute(&[&todo.title(),
                                        &todo.order(),
                                        &todo.completed(),
                                        &todo.uid.unwrap()]).unwrap();

            if changes == 0 {
                response.error(StatusCode::NotFound, "")
            } else if changes > 1 {
                response.error(StatusCode::InternalServerError, "Too many items patched")
            } else {
                todo.to_json().respond(response)
            }
        }
    }
}

pub fn main() {
    let mut server = Nickel::new();

    server.utilize(middleware! { |request|
        println!("logging request: {} => {:?}", request.origin.method, request.origin.uri);
    });

    // Enable CORS
    server.utilize(middleware! { |request, mut response|
        let headers = response.origin.headers_mut();
        headers.set(header::AccessControlAllowHeaders(vec![UniCase("content-type".to_string())]));
        headers.set(header::AccessControlAllowOrigin::Any);
    });

    server.utilize(db_middleware());

    let mut router = router! {
        get "/todos" => |request, response| {
            let db_conn = request.db_conn();
            let stmt = db_conn.prepare("SELECT uid, title, order_idx, completed FROM todos").unwrap();

            stmt.query(&[])
                .unwrap()
                .into_iter()
                .map(Todo::from)
                .collect::<Vec<_>>()
                .to_json()
        }

        get "/todos/:uid" => |request, response| {
            match find_todo(request) {
                Some(todo) => todo.to_json(),
                None => return response.error(StatusCode::NotFound, "{}")
            }
        }

        post "/todos" => |request, response| {
            let mut todo = request.json_as::<Todo>().unwrap();
            let db_conn = request.db_conn();
            let stmt = db_conn.prepare("INSERT INTO todos (title, order_idx, completed) VALUES ( $1, $2, $3 ) RETURNING uid").unwrap();

            let mut iter = stmt.query(&[&todo.title(),
                                        &todo.order(),
                                        &todo.completed()]).unwrap().into_iter();

            match (iter.next(), iter.next()) {
                (Some(select), None) => {
                    todo.uid = Some(select.get(0));
                    todo.to_json()
                },
                // Should have one and only one uid from an insert
                _ => return response.error(StatusCode::InternalServerError, "Inserted row count != 1")
            }
        }

        delete "/todos" => |request, response| {
            request.db_conn()
                   .execute("TRUNCATE todos", &[])
                   .unwrap();
            Json::from_str("{}").unwrap()
        }

        delete "/todos/:uid" => |request, response| {
            let db_conn = request.db_conn();
            let uid = request.param("uid").trim().parse::<i32>().unwrap();
            let deletes = db_conn.execute("DELETE FROM todos * WHERE uid = $1",
                                          &[&uid]).unwrap();

            if deletes == 0 {
                (StatusCode::NotFound, "")
            } else if deletes > 1 {
                (StatusCode::InternalServerError, "More than one deletion?")
            } else {
                println!("DELETED TODO {}", uid);
                (StatusCode::Ok, "{}")
            }
        }
    };

    router.add_route(Method::Patch, "/todos/:uid", patch_handler);
    router.add_route(Method::Post, "/todos/:uid", patch_handler);

    router.add_route(Method::Options, "/todos", middleware! { |req, mut res|
        let headers = res.origin.headers_mut();
        headers.set(header::AccessControlAllowMethods(vec![Method::Get,
                                                           Method::Head,
                                                           Method::Post,
                                                           Method::Delete,
                                                           Method::Options,
                                                           Method::Put]));
        "" // start the request
    });

    router.add_route(Method::Options, "/todos/:uid", middleware! { |req, mut res|
        let headers = res.origin.headers_mut();
        headers.set(header::AccessControlAllowMethods(vec![Method::Get,
                                                           Method::Patch,
                                                           Method::Head,
                                                           Method::Delete,
                                                           Method::Options]));
        "" // start the request
    });

    server.utilize(router);

    // Get port from heroku env
    let port = env::var("PORT").unwrap_or_else(|_| "6767".to_string());
    server.listen(&*format!("0.0.0.0:{}", port));
}

//initialise database tables, if has not already been done
fn db_middleware() -> PostgresMiddleware {
    let ssl_context = SslContext::new(SslMethod::Tlsv1).unwrap();
    let url = env::var("DATABASE_URL").unwrap();
    let db = PostgresMiddleware::new(&*url,
                                     SslMode::Prefer(ssl_context),
                                     5,
                                     Box::new(r2d2::NoopErrorHandler)).unwrap();

    {
        let connection = db.pool.get().unwrap();
        connection.execute("CREATE TABLE IF NOT EXISTS todos (
                                uid SERIAL PRIMARY KEY,
                                title VARCHAR NOT NULL,
                                order_idx INTEGER DEFAULT 0,
                                completed BOOL DEFAULT FALSE)", &[]).unwrap();
    }

    db
}
