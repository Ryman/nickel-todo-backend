#[macro_use] extern crate lazy_static;
#[macro_use] extern crate nickel;
extern crate nickel_postgres;
extern crate rustc_serialize;
extern crate openssl;
extern crate postgres;
extern crate unicase;
extern crate hyper;
extern crate r2d2;

use nickel::{Nickel, HttpRouter, JsonBody};
use nickel::status::StatusCode;

use std::env;
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
            match Todo::all(&*request.db_conn()) {
                Ok(todos) => todos.to_json(),
                Err(code) => return response.send(code)
            }
        }

        get "/todos/:uid" => |request, response| {
            let uid = request.param("uid").trim().parse().unwrap();
            match Todo::find_by_id(&*request.db_conn(), uid) {
                Ok(todo) => todo,
                Err(errcode) => return response.error(errcode, "{}")
            }
        }

        post "/todos" => |request, response| {
            let mut todo = match request.json_as::<Todo>() {
                Ok(todo) => todo,
                Err(e) => return response.error(StatusCode::BadRequest, format!("{}", e))
            };

            match todo.save(&*request.db_conn()) {
                Ok(()) => todo,
                Err(errcode) => return response.send(errcode)
            }
        }

        delete "/todos" => |request, response| {
            match Todo::delete_all(&*request.db_conn()) {
                Ok(()) => Json::from_str("{}"),
                Err(errcode) => return response.send(errcode)
            }
        }

        delete "/todos/:uid" => |request, response| {
            let uid = request.param("uid").trim().parse().unwrap();

            return match Todo::delete_by_id(&*request.db_conn(), uid) {
                Ok(()) => response.send(Json::from_str("{}")),
                Err(errcode) => response.send(errcode)
            }
        }

        patch "/todos/:uid" => |request, response| {
            let uid = request.param("uid").trim().parse().unwrap();
            let todo = Todo::find_by_id(&*request.db_conn(), uid); // borrowck

            // `return` is used as these match arms all return `MiddlewareResult`
            // and it won't implement `Responder`, so we short circuit the closure
            return match todo {
                Err(errcode) => response.send(errcode),
                Ok(mut todo) => {
                    match request.json_as::<Todo>() {
                        Ok(diff) => todo.merge(diff),
                        Err(e) => return response.error(StatusCode::BadRequest, format!("{}", e))
                    }

                    match todo.save(&*request.db_conn()) {
                        Ok(()) => response.send(todo),
                        Err(errcode) => response.send(errcode)
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
