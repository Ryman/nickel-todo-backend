#[macro_use] extern crate lazy_static;
#[macro_use] extern crate nickel;
extern crate nickel_postgres;
extern crate rustc_serialize;
extern crate openssl;
extern crate postgres;
extern crate unicase;
extern crate hyper;
extern crate r2d2;

use nickel::{Nickel, Request, HttpRouter, JsonBody};
use nickel::status::StatusCode;

use std::{env, io};
use std::num::ParseIntError;
use hyper::header;
use unicase::UniCase;
use nickel_postgres::PostgresRequestExtensions;
use rustc_serialize::json::{Json, ToJson};

use todo::Todo;
use datastore::DataStore;

mod todo;
mod datastore;

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

fn parse_uid(request: &Request) -> Result<i32, (StatusCode, ParseIntError)> {
    let uid = request.param("uid").unwrap_or("");
    let uid = uid.trim().parse();
    uid.map_err(|e| (StatusCode::BadRequest, e))
}

fn parse_todo(request: &mut Request) -> Result<Todo, (StatusCode, io::Error)> {
    request.json_as().map_err(|e| (StatusCode::BadRequest, e))
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

    server.utilize(datastore::setup());

    server.utilize(router! {
        get "/todos" => |request, response| {
            let todos = try_with!(response, Todo::all(&*request.db_conn()));
            todos.to_json()
        }

        get "/todos/:uid" => |request, response| {
            let uid = try_with!(response, parse_uid(request));

            Todo::find_by_id(&*request.db_conn(), uid)
        }

        post "/todos" => |request, response| {
            let mut todo = try_with!(response, parse_todo(request));
            try_with!(response, todo.save(&*request.db_conn()));
            todo
        }

        delete "/todos" => |request, response| {
            try_with!(response, Todo::delete_all(&*request.db_conn()));

            Json::from_str("{}").unwrap()
        }

        delete "/todos/:uid" => |request, response| {
            let uid = try_with!(response, parse_uid(request));

            try_with!(response, Todo::delete_by_id(&*request.db_conn(), uid));

            Json::from_str("{}").unwrap()
        }

        patch "/todos/:uid" => |request, response| {
            let diff = try_with!(response, parse_todo(request));
            let uid = try_with!(response, parse_uid(request));

            let conn = &*request.db_conn();
            let mut todo = try_with!(response, Todo::find_by_id(conn, uid));
            try_with!(response, todo.merge(diff).save(conn));
            todo
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
