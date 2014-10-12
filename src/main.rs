#![feature(phase)]
#![allow(unused_imports)]
extern crate http;
extern crate nickel;
extern crate serialize;
#[phase(plugin)] extern crate nickel_macros;

use http::status::NotFound;
use nickel::{
    Nickel, NickelError, ErrorWithStatusCode, Continue, Halt, Request, Response,
    QueryString, JsonBody, StaticFilesHandler, MiddlewareResult, HttpRouter
};
use std::io::net::ip::Ipv4Addr;
use std::os::getenv;
use http::method;

#[deriving(Decodable, Encodable)]
struct Person {
    firstname: String,
    lastname:  String,
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
fn allow_cors(_req: &Request, response: &mut Response) -> MiddlewareResult {
    response.origin.headers.insert_raw("Access-Control-Allow-Headers".to_string(), b"content-type");
    response.origin.headers.insert_raw("Access-Control-Allow-Origin".to_string(), b"*");
    Ok(Continue)
}

#[cfg(not(test))]
fn main() {
    let mut server = Nickel::new();

    // middleware is optional and can be registered with `utilize`
    server.utilize(logger);

    server.utilize(allow_cors);

    fn options_handler(req: &Request, res: &mut Response) {
        res.origin.headers.insert_raw("Access-Control-Allow-Methods".to_string(), b"GET,HEAD,POST,DELETE,OPTIONS,PUT");
    }

    server.add_route(method::Options, "/todos", options_handler);

    // this will cause json bodies automatically being parsed
    server.utilize(Nickel::json_body_parser());

    // this will cause the query string to be parsed on each request
    server.utilize(Nickel::query_string());

    // go to http://localhost:6767/thoughtram_logo_brain.png to see static file serving in action
    server.utilize(StaticFilesHandler::new("examples/assets/"));

    server.utilize(router! {
        get "/todos" => |request, response| {
            // if env.has_key? "HTTP_ACCESS_CONTROL_REQUEST_HEADERS"
            response.send("{}");
        }

        post "/todos" => |request, response| {
            println!("{}", request.origin.body.as_slice());
            // new_todo = json_body
            // stored_todo = @repo.add_todo(new_todo)
            // headers["Location"] = todo_url(stored_todo)
            // status 201
            // # content_type :json
            // todo_repr(stored_todo).to_json
            response.send(r#"{"title": "a todo"}"#)
        }

        // go to http://localhost:6767/user/4711 to see this route in action
        get "/user/:userid" => |request, response| {
            let text = format!("This is user: {}", request.param("userid"));
            response.send(text.as_slice());
        }

        // go to http://localhost:6767/bar to see this route in action
        get "/bar" => |request, response| {
            response.send("This is the /bar handler");
        }

        // go to http://localhost:6767/some/crazy/route to see this route in action
        get "/some/*/route" => |request, response| {
            response.send("This matches /some/crazy/route but not /some/super/crazy/route");
        }

        // go to http://localhost:6767/some/crazy/route to see this route in action
        get "/a/**/route" => |request, response| {
            response.send("This matches /a/crazy/route and also /a/super/crazy/route");
        }

        // try it with curl
        // curl 'http://localhost:6767/a/post/request' -H 'Content-Type: application/json;charset=UTF-8'  --data-binary $'{ "firstname": "John","lastname": "Connor" }'
        post "/a/post/request" => |request, response| {
            let person = request.json_as::<Person>().unwrap();
            let text = format!("Hello {} {}", person.firstname, person.lastname);
            response.send(text.as_slice());
        }

        // try calling http://localhost:6767/query?foo=bar
        get "/query" => |request, response| {
            let text = format!("Your foo values in the query string are: {}", request.query("foo", "This is only a default value!"));
            response.send(text.as_slice());
        }
    });

    server.handle_error(custom_404);

    println!("Running server!");

    // Get port from heroku env
    let port = getenv("PORT").and_then(|s| from_str::<u16>(s.as_slice().trim())).unwrap_or(6767);
    println!("Binding to port: {}", port)
    server.listen(Ipv4Addr(0, 0, 0, 0), port);
}
