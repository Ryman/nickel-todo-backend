use nickel::{Response, Responder, MiddlewareResult};
use rustc_serialize::json::{self, Json, ToJson};
use std::collections::BTreeMap;
use postgres;

use SITE_ROOT_URL;

#[derive(RustcDecodable)]
pub struct Todo {
    uid: Option<i32>,
    title: Option<String>,
    order: Option<i32>,
    completed: Option<bool>,
}

impl Todo {
    pub fn uid(&self) -> &Option<i32> {
        &self.uid
    }

    pub fn set_uid(&mut self, uid: i32) {
        self.uid = Some(uid)
    }

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

impl Responder for Todo {
    fn respond<'a>(self, response: Response<'a>) -> MiddlewareResult<'a> {
        response.send(self.to_json())
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
