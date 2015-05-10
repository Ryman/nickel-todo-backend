#[macro_use] extern crate lazy_static;
#[macro_use] extern crate nickel;
extern crate nickel_postgres;
extern crate rustc_serialize;
extern crate openssl;
extern crate postgres;
extern crate unicase;
extern crate hyper;
extern crate r2d2;

use nickel::{Nickel, Request, QueryString, HttpRouter, JsonBody};
use nickel::status::StatusCode;

use std::env;
use hyper::header;
use unicase::UniCase;
use nickel_postgres::{PostgresMiddleware, PostgresRequestExtensions};
use postgres::SslMode;
use openssl::ssl::{SslMethod, SslContext};
use rustc_serialize::json::{Json, ToJson};

use todo::Todo;

mod todo;

lazy_static! {
    pub static ref SITE_ROOT_URL: String = {
        let mut root = env::var("SITE_ROOT_URL").unwrap_or_else(|_| "http://0.0.0.0:6767/".to_string());

        if root.is_empty() {
            panic!("Cannot supply an empty `SITE_ROOT_URL`")
        }

        // Ensure slash termination
        if root.as_bytes().last() != Some(&b'/') { root.push('/'); }
        root
    };
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

pub fn main() {
    let mut server = Nickel::new();

    server.utilize(middleware! { |request|
        println!("logging request: {} => {:?}", request.origin.method, request.origin.uri);
    });

    // Enable CORS
    server.utilize(middleware! { |_, mut response|
        response.set(header::AccessControlAllowHeaders(vec![UniCase("content-type".to_string())]));
        response.set(header::AccessControlAllowOrigin::Any);
    });

    server.utilize(db_middleware());

    server.utilize(router! {
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
                Some(todo) => todo,
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
                    todo.set_uid(select.get(0));
                    todo
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

        patch "/todos/:uid" => |request, response| {
            match find_todo(request) {
                None => return response.error(StatusCode::NotFound, ""),
                Some(mut todo) => {
                    let diff = request.json_as::<Todo>().unwrap();
                    todo.merge(diff);

                    let db_conn = request.db_conn();
                    let stmt = db_conn.prepare("UPDATE todos SET title = $1, order_idx = $2, completed = $3 WHERE uid = $4").unwrap();
                    let changes = stmt.execute(&[&todo.title(),
                                                &todo.order(),
                                                &todo.completed(),
                                                &todo.uid().unwrap()]).unwrap();

                    if changes == 0 {
                        return response.error(StatusCode::NotFound, "")
                    } else if changes > 1 {
                        return response.error(StatusCode::InternalServerError, "Too many items patched")
                    } else {
                        todo
                    }
                }
            }
        }

        options "/todos" => |_, mut res| {
            use hyper::method::Method::*;
            res.set(header::AccessControlAllowMethods(vec![Get, Head, Post, Delete, Options, Put]));

            ""
        }

        options "/todos/:uid" => |_, mut res| {
            use hyper::method::Method::*;
            res.set(header::AccessControlAllowMethods(vec![Get, Patch, Head, Delete, Options]));

            ""
        }
    });

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
