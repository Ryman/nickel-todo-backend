# Todo Backend in Rust

This is a simple implementation of the [Todo-Backend API spec](http://todo-backend.thepete.net/). It persists todos in a Postgres database.

It is running live at [http://nickel-todo-backend.herokuapp.com/todos](http://nickel-todo-backend.herokuapp.com/todos). You can [point a todo-backend client at that live instance](http://www.todobackend.com/client/?https://nickel-todo-backend.herokuapp.com/todos) to play with it. You can also [run the Todo-Backend specs against that live instance](http://www.todobackend.com/specs/index.html?http://nickel-todo-backend.herokuapp.com/todos) to confirm that it complies with the Todo-Backend API spec.

# Local test
```
SITE_ROOT_URL="__BASE_URL__" DATABASE_URL="__DB_INFO__" cargo run --release

// Run tests against localhost:6767
// If running the tests from the website remotely I recommend using `ngrok`
```

## Running on heroku
These are roughly the commands required (not verified recently)
```
git clone http://github.com/Ryman/nickel-todo-backend
cd nickel-todo-backend

// See https://devcenter.heroku.com/articles/creating-apps
heroku create

heroku config:set BUILDPACK_URL="https://github.com/Ryman/heroku-buildpack-rust.git"

heroku config:set DATABASE_URL="__PROBABLY_A_COPY_OF_HEROKU_POSTGRESQL_AQUA_URL__"

heroku config:set SITE_ROOT_URL="http://nickel-todo-backend.herokuapp.com"

// git push heroku etc etc
```

# LICENSE
MIT

# Credit
[emk's heroku-buildpack-rust](github.com/emk/heroku-buildpack-rust.git).

[Sinatra version](https://github.com/moredip/todo-backend-sinatra) for original influence.
