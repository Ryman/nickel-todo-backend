use std::env;
use nickel::status::StatusCode;
use nickel_postgres::PostgresMiddleware;
use postgres::{Connection, SslMode};
use openssl::ssl::{SslMethod, SslContext};
use r2d2;
use todo::Todo;

//initialise database tables, if has not already been done
pub fn setup() -> PostgresMiddleware {
    let ssl_context = SslContext::new(SslMethod::Tlsv1).unwrap();
    let url = env::var("DATABASE_URL").unwrap();
    let db = PostgresMiddleware::new(&*url,
                                     SslMode::Prefer(Box::new(ssl_context)),
                                     10,
                                     Box::new(r2d2::NopErrorHandler)).unwrap();

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

pub trait DataStore : Sized {
    type Id;
    type Connection;
    type Error;

    fn find_by_id(&Self::Connection, Self::Id) -> Result<Self, Self::Error>;

    fn all(&Self::Connection) -> Result<Vec<Self>, Self::Error>;

    fn save(&mut self, &Self::Connection) -> Result<(), Self::Error>;

    fn delete_by_id(&Self::Connection, Self::Id) -> Result<(), Self::Error>;

    fn delete_all(&Self::Connection) -> Result<(), Self::Error>;
}

impl DataStore for Todo {
    type Id = i32;
    type Connection = Connection;
    type Error = StatusCode;

    fn find_by_id(conn: &Connection, id: i32) -> Result<Todo, StatusCode> {
        let stmt = conn.prepare_cached("SELECT uid, title, order_idx, completed \
                                        FROM todos WHERE uid = $1").unwrap();
        let mut iter = stmt.query(&[&id]).unwrap().into_iter();

        match (iter.next(), iter.next()) {
            (Some(row), None) => Ok(Todo::from(row)),
            // Just a 404
            (None, None) => Err(StatusCode::NotFound),
            // Shouldn't get multiple for a uid
            (Some(_), Some(_)) | (None, Some(_)) => {
                println!("BADBAD: {:?} gave multiple results", id);
                Err(StatusCode::InternalServerError)
            }
        }
    }

    fn all(conn: &Self::Connection) -> Result<Vec<Todo>, StatusCode> {
        let stmt = conn.prepare_cached("SELECT uid, title, order_idx, completed \
                                        FROM todos").unwrap();

        match stmt.query(&[]) {
            Ok(rows) => Ok(rows.into_iter().map(Todo::from).collect()),
            Err(_) => Err(StatusCode::InternalServerError)
        }
    }

    fn save(&mut self, conn: &Self::Connection) -> Result<(), StatusCode> {
        if let Some(uid) = *self.uid() {
            let stmt = conn.prepare_cached("UPDATE todos SET title = $1, \
                                                   order_idx = $2, \
                                                   completed = $3 \
                                            WHERE uid = $4");
            match stmt.and_then(|stmt| stmt.execute(&[&self.title(),
                                                      &self.order(),
                                                      &self.completed(),
                                                      &uid])) {
                Ok(1) => Ok(()),
                Ok(0) => Err(StatusCode::NotFound),
                _ => Err(StatusCode::InternalServerError)
            }
        } else {
            insert_new(self, conn)
        }
    }

    fn delete_by_id(conn: &Self::Connection, id: i32) -> Result<(), StatusCode> {
        let deletes = conn.execute("DELETE FROM todos * WHERE uid = $1", &[&id]);

        match deletes {
            Ok(1) => Ok(()),
            Ok(0) => Err(StatusCode::NotFound),
            _ => Err(StatusCode::InternalServerError)
        }
    }

    fn delete_all(conn: &Self::Connection) -> Result<(), StatusCode> {
        match conn.execute("TRUNCATE todos", &[]) {
            Ok(_) => Ok(()),
            Err(_) => Err(StatusCode::InternalServerError)
        }
    }
}

fn insert_new(todo: &mut Todo, conn: &Connection) -> Result<(), StatusCode> {
    let stmt = conn.prepare_cached("INSERT INTO todos (title, order_idx, completed) \
                                    VALUES ( $1, $2, $3 ) RETURNING uid");

    // Borrowck has complaints matching on stmt.and_then below.
    let stmt = match stmt {
        Ok(stmt) => stmt,
        Err(_) => return Err(StatusCode::InternalServerError)
    };

    match stmt.query(&[&todo.title(),
                       &todo.order(),
                       &todo.completed()]) {
        Ok(rows) => {
            let mut iter = rows.into_iter();

            match (iter.next(), iter.next()) {
                (Some(select), None) => {
                    todo.set_uid(select.get(0));
                    Ok(())
                },
                // Should have one and only one uid from an insert
                _ => Err(StatusCode::InternalServerError)
            }
        }
        Err(_) => Err(StatusCode::InternalServerError)
    }
}
