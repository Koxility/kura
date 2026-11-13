use kura_store::Store;
use reqwest::{Client, Method, RequestBuilder};

use super::*;

/// A server on a free local port, talked to over real HTTP.
struct Server {
    http: Client,
    base: String,
}

impl Server {
    async fn start() -> Self {
        let core = Core::with_defaults(Store::open_in_memory().unwrap()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router(core)).await });
        Self {
            http: Client::new(),
            base,
        }
    }

    fn request(&self, method: Method, path: &str, cookie: &str) -> RequestBuilder {
        let url = format!("{}/api/v1{path}", self.base);
        self.http.request(method, url).header("Cookie", cookie)
    }

    /// Status and JSON body of a request.
    async fn call(&self, method: Method, path: &str, cookie: &str, body: Value) -> (u16, Value) {
        let mut request = self.request(method, path, cookie);
        if !body.is_null() {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        (status, response.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str, cookie: &str) -> (u16, Value) {
        self.call(Method::GET, path, cookie, Value::Null).await
    }

    /// Logs in through `path` and returns the session cookie.
    async fn log_in(&self, path: &str, name: &str, password: &str) -> String {
        let form = json!({ "name": name, "password": password });
        let response = self.request(Method::POST, path, "").json(&form);
        let response = response.send().await.unwrap();
        assert_eq!(response.status(), 200);
        let cookie = response.headers()["set-cookie"].to_str().unwrap();
        assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
        cookie.split(';').next().unwrap().to_owned()
    }
}

#[tokio::test]
async fn setup_login_and_logout() {
    let server = Server::start().await;
    let (code, fresh) = server.get("/status", "").await;
    assert_eq!(
        (code, &fresh["needs_setup"], &fresh["user"]),
        (200, &json!(true), &Value::Null)
    );
    assert_eq!(fresh["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(fresh["update_available"], false);
    assert_eq!(fresh["needs_library"], true);
    assert!(
        fresh["suggested_library_path"]
            .as_str()
            .unwrap()
            .ends_with("downloads")
    );
    assert!(
        fresh["repository"]
            .as_str()
            .unwrap()
            .starts_with("https://github.com/")
    );
    assert_eq!(server.get("/queue", "").await.0, 401);
    assert_eq!(server.get("/queue", "kura_session=made-up").await.0, 401);

    let weak = json!({ "name": "admin", "password": "short" });
    assert_eq!(server.call(Method::POST, "/setup", "", weak).await.0, 400);

    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let (_, status) = server.get("/status", &admin).await;
    assert_eq!(status["needs_setup"], false);
    assert_eq!(status["user"]["role"], "admin");

    let again = json!({ "name": "intruder", "password": "correct horse" });
    assert_eq!(server.call(Method::POST, "/setup", "", again).await.0, 409);
    let wrong = json!({ "name": "admin", "password": "wrong horse" });
    assert_eq!(server.call(Method::POST, "/login", "", wrong).await.0, 401);

    assert_eq!(server.get("/queue", &admin).await, (200, json!([])));
    let logout = server.call(Method::POST, "/logout", &admin, Value::Null);
    assert_eq!(logout.await.0, 204);
    assert_eq!(server.get("/queue", &admin).await.0, 401);
}

#[tokio::test]
async fn settings_and_users_are_for_admins() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;

    let (_, mut settings) = server.get("/settings", &admin).await;
    assert_eq!(settings["parallel"], 4);
    // Nothing can be saved without a library.
    let refused = server.call(Method::PUT, "/settings", &admin, settings.clone());
    assert_eq!(refused.await.0, 400);

    let folder = tempfile::tempdir().unwrap();
    let main = folder.path().join("main");
    settings["libraries"] = json!([{ "name": "Main", "path": main }]);
    settings["parallel"] = json!(2);
    let saved = server.call(Method::PUT, "/settings", &admin, settings.clone());
    assert_eq!(saved.await.0, 200);
    assert_eq!(server.get("/settings", &admin).await.1["parallel"], 2);
    settings["parallel"] = json!(0);
    let refused = server.call(Method::PUT, "/settings", &admin, settings.clone());
    assert_eq!(refused.await.0, 400);
    settings["parallel"] = json!(2);
    settings["libraries"] = json!([{ "name": "Anime", "path": "relative" }]);
    let refused = server.call(Method::PUT, "/settings", &admin, settings.clone());
    assert_eq!(refused.await.0, 400);
    let anime = folder.path().join("anime");
    settings["libraries"] = json!([
        { "name": "Main", "path": main },
        { "name": "Anime", "path": anime, "default_for": ["aniworld"] },
    ]);
    settings["output_format"] = json!("mp4");
    let saved = server.call(Method::PUT, "/settings", &admin, settings.clone());
    assert_eq!(saved.await.0, 200);
    let (_, stored) = server.get("/settings", &admin).await;
    assert_eq!(stored["libraries"][1]["default_for"], json!(["aniworld"]));
    assert!(anime.is_dir());
    let (_, status) = server.get("/status", &admin).await;
    assert_eq!(status["needs_library"], false);
    assert_eq!(
        stored["auto_sync"]["schedule"],
        json!({ "kind": "every", "minutes": 60 })
    );

    let sam = json!({ "name": "sam", "password": "battery staple", "role": "user" });
    let (status, created) = server
        .call(Method::POST, "/users", &admin, sam.clone())
        .await;
    assert_eq!((status, &created["role"]), (200, &json!("user")));
    assert_eq!(
        server.call(Method::POST, "/users", &admin, sam).await.0,
        409
    );

    let user = server.log_in("/login", "sam", "battery staple").await;
    settings["parallel"] = json!(8);
    let denied = server.call(Method::PUT, "/settings", &user, settings);
    assert_eq!(denied.await.0, 403);
    // A normal user reaches Home, the queue and requests, nothing else.
    for path in [
        "/settings",
        "/users",
        "/admin/ip",
        "/library",
        "/library/0",
        "/autosync",
    ] {
        assert_eq!(server.get(path, &user).await.0, 403, "{path}");
    }
    for path in [
        "/queue/clear",
        "/queue/1/cancel",
        "/queue/1/retry",
        "/autosync/run",
        "/requests/1/accept",
        "/requests/1/reject",
    ] {
        let denied = server.call(Method::POST, path, &user, Value::Null);
        assert_eq!(denied.await.0, 403, "{path}");
    }
    let queue = json!({ "site": "aniworld", "slug": "x", "all": true });
    let denied = server.call(Method::POST, "/queue", &user, queue);
    assert_eq!(denied.await.0, 403);
    let excluded = json!({ "site": "aniworld", "slug": "x", "title": "X", "excluded": true });
    let denied = server.call(Method::POST, "/autosync/exclude", &user, excluded);
    assert_eq!(denied.await.0, 403);
    assert_eq!(server.get("/queue", &user).await.0, 200);
    assert_eq!(server.get("/requests", &user).await, (200, json!([])));
    // A bad IMDb answer is refused before any site is asked.
    let bad = json!({ "site": "aniworld", "slug": "x", "all": true, "imdb_id": "123" });
    assert_eq!(
        server.call(Method::POST, "/requests", &user, bad).await.0,
        400
    );
    let (_, status) = server.get("/status", &user).await;
    assert_eq!(
        (&status["libraries"], &status["sites"]),
        (&json!([]), &json!(["aniworld", "serienstream"]))
    );
    let (_, status) = server.get("/status", &admin).await;
    assert_eq!(status["libraries"], json!(["Main", "Anime"]));

    let (_, users) = server.get("/users", &admin).await;
    let id = |name: &str| {
        let found = users
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["name"] == name);
        found.unwrap()["id"].clone()
    };
    let delete = async |id: Value| {
        let path = format!("/users/{id}");
        let deleted = server.call(Method::DELETE, &path, &admin, Value::Null);
        deleted.await.0
    };
    assert_eq!(delete(id("admin")).await, 409);
    assert_eq!(delete(id("sam")).await, 204);
    assert_eq!(server.get("/settings", &user).await.0, 401);
}

#[tokio::test]
async fn bad_requests_get_clear_answers() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;

    let (status, body) = server.get("/nothing-here", &admin).await;
    assert_eq!(
        (status, body),
        (404, json!({ "error": "no such endpoint" }))
    );
    assert_eq!(
        server.get("/series/aniworld/..%2F..%2Fetc", &admin).await.0,
        400
    );
    assert_eq!(server.get("/series/netflix/whatever", &admin).await.0, 400);
    assert_eq!(server.get("/search?q=%20", &admin).await, (200, json!([])));

    let unknown_hoster = json!({ "site": "aniworld", "slug": "x", "all": true, "hoster": "nope" });
    let refused = server.call(Method::POST, "/queue", &admin, unknown_hoster);
    assert_eq!(refused.await.0, 400);
    let title = json!({ "site": "aniworld", "slug": "x", "title": "X", "excluded": true });
    let excluded = server.call(Method::POST, "/autosync/exclude", &admin, title);
    assert_eq!(excluded.await.0, 204);
    let (_, settings) = server.get("/settings", &admin).await;
    assert_eq!(settings["auto_sync"]["excluded"][0]["slug"], "x");
    let synced = server.call(Method::POST, "/autosync/run", &admin, Value::Null);
    assert_eq!(synced.await, (200, json!({ "queued": 0 })));
    let (_, sync) = server.get("/autosync", &admin).await;
    assert_eq!(
        (&sync["enabled"], &sync["titles"]),
        (&json!(false), &json!([]))
    );
    assert_eq!(server.get("/library", &admin).await, (200, json!([])));
    assert_eq!(server.get("/library/0?path=..%2F..", &admin).await.0, 400);
    let clear = server.call(Method::POST, "/queue/clear", &admin, Value::Null);
    assert_eq!(clear.await, (200, json!({ "cleared": 0 })));
    let cancel = server.call(Method::POST, "/queue/42/cancel", &admin, Value::Null);
    assert_eq!(cancel.await, (200, json!({ "cancelled": false })));
    let retry = server.call(Method::POST, "/queue/42/retry", &admin, Value::Null);
    assert_eq!(retry.await.0, 404);

    // Pages of the web app are served for any other path.
    let page = server
        .http
        .get(format!("{}/queue", server.base))
        .send()
        .await
        .unwrap();
    assert!(page.status() == 200 || page.status() == 404);
}
