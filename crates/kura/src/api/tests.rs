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
        Self::behind(TrustedProxies::default()).await
    }

    /// A server that believes `X-Forwarded-For` from `proxies`.
    async fn behind(proxies: TrustedProxies) -> Self {
        let core = Core::with_defaults(Store::open_in_memory().unwrap()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(serve(listener, core, proxies));
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
    // Which build runs is for those who are logged in.
    assert_eq!(fresh["version"], Value::Null);
    assert_eq!(fresh["commit"], Value::Null);
    // Everyone, logged in or not, learns how the app looks over an ASCII background.
    assert_eq!(fresh["backdrop"], "glass");
    assert_eq!(fresh["latest_version"], Value::Null);
    assert_eq!(fresh["needs_library"], true);
    // Where the server keeps its files is not for someone who is not logged in.
    assert_eq!(fresh["suggested_library_path"], Value::Null);
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
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["needs_setup"], false);
    assert_eq!(status["user"]["role"], "admin");
    assert!(
        status["suggested_library_path"]
            .as_str()
            .unwrap()
            .ends_with("downloads")
    );

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
    assert_eq!(settings["naming_template"], Value::Null);
    assert_eq!(
        settings["naming_template_default"],
        kura_core::DEFAULT_TEMPLATE
    );
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
    let title = json!({ "site": "aniworld", "slug": "x", "title": "X" });
    let denied = server.call(Method::POST, "/autosync/remove", &user, title);
    assert_eq!(denied.await.0, 403);
    assert_eq!(server.get("/queue", &user).await.0, 200);
    // Every account changes its own password.
    let wrong = json!({ "current": "nope", "new": "another horse" });
    assert_eq!(
        server
            .call(Method::PUT, "/account/password", &user, wrong)
            .await
            .0,
        403
    );
    let new = json!({ "current": "battery staple", "new": "another horse" });
    assert_eq!(
        server
            .call(Method::PUT, "/account/password", &user, new)
            .await
            .0,
        204
    );
    assert_eq!(server.get("/queue", &user).await.0, 200);
    let user = server.log_in("/login", "sam", "another horse").await;
    // Clearing requests is for admins; cancelling someone else's request is refused.
    assert_eq!(
        server
            .call(Method::POST, "/requests/clear", &user, Value::Null)
            .await
            .0,
        403
    );
    let cancel = server.call(Method::POST, "/requests/999/cancel", &user, Value::Null);
    assert_eq!(cancel.await.0, 400);
    let (code, found) = server
        .get("/series/aniworld/x/downloaded?title=X", &user)
        .await;
    assert_eq!((code, found), (200, json!([])));
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
    assert_eq!(status["suggested_library_path"], Value::Null);
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
    let title = json!({ "site": "aniworld", "slug": "y", "title": "Y" });
    let removed = server.call(Method::POST, "/autosync/remove", &admin, title);
    assert_eq!(removed.await.0, 204);
    let (_, settings) = server.get("/settings", &admin).await;
    assert_eq!(settings["auto_sync"]["removed"][0]["slug"], "y");
    assert_eq!(
        settings["auto_sync"]["excluded"].as_array().unwrap().len(),
        1
    );
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

#[tokio::test]
async fn wrong_logins_are_slowed_down() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let wrong = json!({ "name": "admin", "password": "wrong horse" });
    for try_ in 0..5 {
        // Without a trusted proxy, an address in the header is made up and changes nothing.
        let from = format!("203.0.113.{try_}");
        let refused = server.request(Method::POST, "/login", "");
        let refused = refused.header("X-Forwarded-For", from).json(&wrong);
        assert_eq!(refused.send().await.unwrap().status(), 401);
    }
    // From here on the right password has to wait too.
    let right = json!({ "name": "admin", "password": "correct horse" });
    let (code, answer) = server.call(Method::POST, "/login", "", right).await;
    assert_eq!(code, 429);
    assert!(answer["error"].as_str().unwrap().contains("try again in"));
    // A session that exists goes on working.
    assert_eq!(server.get("/queue", &admin).await.0, 200);
}

#[test]
fn users_see_jobs_without_server_paths() {
    let job = |dir: &str, path: Option<&str>, error: Option<&str>| Job {
        id: 1,
        episode: kura_domain::EpisodeId {
            site: SiteId::AniWorld,
            slug: "frieren".into(),
            season: 1,
            number: 1,
        },
        series_title: "Frieren".into(),
        state: kura_domain::JobState::Failed,
        progress: 0.0,
        error: error.map(str::to_owned),
        path: path.map(str::to_owned),
        language: None,
        hoster: None,
        dir: dir.to_owned(),
        hidden: false,
        queued_at: 0,
        imdb_id: None,
        started_at: None,
    };
    let mut libraries = vec!["/media".to_owned(), "/media/anime".to_owned()];

    let file = "/media/anime/Frieren/Season 01/Frieren S01E001.mkv";
    let mut done = job("/media/anime", Some(file), None);
    without_paths(&mut done, &mut libraries);
    assert_eq!((done.dir.as_str(), done.path, done.error), ("", None, None));

    // A library that was removed from the settings since is still a path on the server.
    let error = "voe, german_dub: ffmpeg failed: Error opening output file \
                 /mnt/old/Frieren/Season 01/x.mkv.part; vidmoly: /media/anime/Frieren is full";
    let mut failed = job("/mnt/old", None, Some(error));
    without_paths(&mut failed, &mut libraries);
    assert_eq!(
        failed.error.as_deref(),
        Some(
            "voe, german_dub: ffmpeg failed: Error opening output file \
             <library>/Frieren/Season 01/x.mkv.part; vidmoly: <library>/Frieren is full"
        )
    );
}

/// HAProxy in front of Kura: every connection is the proxy's, the header names the person.
#[tokio::test]
async fn behind_a_trusted_proxy_the_brake_counts_the_person() {
    let server = Server::behind(TrustedProxies::parse("127.0.0.1").unwrap()).await;
    server.log_in("/setup", "admin", "correct horse").await;
    let login = async |from: &str, password: &str, proto: &str| {
        let form = json!({ "name": "admin", "password": password });
        let request = server.request(Method::POST, "/login", "");
        let request = request.header("X-Forwarded-For", from);
        let response = request.header("X-Forwarded-Proto", proto).json(&form);
        response.send().await.unwrap()
    };
    for _ in 0..6 {
        let attacker = login("203.0.113.66", "guess", "https").await;
        assert!([401, 429].contains(&attacker.status().as_u16()));
    }
    assert_eq!(
        login("203.0.113.66", "correct horse", "https")
            .await
            .status(),
        429
    );
    // The admin, somewhere else, is not held up.
    let admin = login("198.51.100.7", "correct horse", "https").await;
    assert_eq!(admin.status(), 200);
    let cookie = admin.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.ends_with("; Secure"), "{cookie}");
    // Over plain HTTP the cookie has to work without `Secure`.
    let plain = login("198.51.100.7", "correct horse", "http").await;
    let cookie = plain.headers()["set-cookie"].to_str().unwrap();
    assert!(!cookie.contains("Secure"), "{cookie}");
}

#[tokio::test]
async fn status_names_the_real_defaults() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let (_, mut settings) = server.get("/settings", &admin).await;
    let folder = tempfile::tempdir().unwrap();
    settings["libraries"] = json!([
        { "name": "Main", "path": folder.path().join("main") },
        { "name": "Anime", "path": folder.path().join("anime"), "default_for": ["aniworld"] },
    ]);
    settings["languages"] = json!(["german_sub", "german_dub"]);
    settings["hosters"] = json!(["made-up", "vidmoly", "voe"]);
    let saved = server.call(Method::PUT, "/settings", &admin, settings);
    assert_eq!(saved.await.0, 200);

    let (_, status) = server.get("/status", &admin).await;
    assert_eq!(
        status["defaults"],
        json!({
            "language": "german_sub",
            // A provider Kura cannot extract from is never the one that is used.
            "hoster": "vidmoly",
            "libraries": { "aniworld": "Anime", "serienstream": "Main" },
        })
    );

    let sam = json!({ "name": "sam", "password": "battery staple", "role": "user" });
    server.call(Method::POST, "/users", &admin, sam).await;
    let user = server.log_in("/login", "sam", "battery staple").await;
    let (_, status) = server.get("/status", &user).await;
    // Library names are for admins.
    assert_eq!(status["defaults"]["libraries"], json!({}));
    assert_eq!(status["defaults"]["language"], "german_sub");
    assert_eq!(server.get("/status", "").await.1["defaults"], Value::Null);
}

#[tokio::test]
async fn answers_are_never_cached_and_events_are_not_buffered() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;

    let queue = server
        .request(Method::GET, "/queue", &admin)
        .send()
        .await
        .unwrap();
    assert_eq!(queue.headers()["cache-control"], "no-store");
    let missing = server
        .request(Method::GET, "/nothing", &admin)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.headers()["cache-control"], "no-store");

    let events = server.request(Method::GET, "/events", &admin);
    let events = events
        .header("Accept-Encoding", "gzip, br")
        .send()
        .await
        .unwrap();
    assert_eq!(events.headers()["content-type"], "text/event-stream");
    assert_eq!(events.headers()["x-accel-buffering"], "no");
    assert_eq!(events.headers()["cache-control"], "no-cache, no-transform");
    // A compressed stream would sit in the compressor until enough piles up.
    assert!(events.headers().get("content-encoding").is_none());
}

#[tokio::test]
async fn the_web_app_is_cached_by_content_and_compressed() {
    let server = Server::start().await;
    let page = server
        .http
        .get(format!("{}/queue", server.base))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    assert_eq!(page.headers()["cache-control"], "no-cache");
    let tag = page.headers()["etag"].to_str().unwrap().to_owned();
    let html = page.text().await.unwrap();

    let again = server.http.get(format!("{}/", server.base));
    let again = again.header("If-None-Match", &tag).send().await.unwrap();
    assert_eq!(again.status(), 304);
    assert_eq!(again.headers()["etag"], tag.as_str());

    // Vite names its files after their content.
    let script = html
        .split('"')
        .find(|part| part.starts_with("/assets/") && part.ends_with(".js"))
        .expect("the page loads a script from /assets/");
    let script = server.http.get(format!("{}{script}", server.base));
    let script = script
        .header("Accept-Encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(script.status(), 200);
    assert_eq!(
        script.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(script.headers()["content-encoding"], "gzip");
}

/// Another program, such as Musubi, uses an API key instead of a login.
#[tokio::test]
async fn api_keys_stand_in_for_a_login() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user" });
    server.call(Method::POST, "/users", &admin, user).await;
    let sam = server.log_in("/login", "sam", "correct horse").await;

    let (code, created) = server
        .call(
            Method::POST,
            "/account/keys",
            &sam,
            json!({ "name": "Musubi" }),
        )
        .await;
    assert_eq!(code, 200);
    let secret = created["secret"].as_str().unwrap().to_owned();
    assert!(secret.starts_with(kura_core::API_KEY_PREFIX));
    let bearer = |path: &str, key: &str| {
        let url = format!("{}/api/v1{path}", server.base);
        server.http.get(url).bearer_auth(key.to_owned()).send()
    };

    let status: Value = bearer("/status", &secret)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["user"]["name"], "sam");
    assert_eq!(bearer("/queue", &secret).await.unwrap().status(), 200);
    // A key is the user's own rights, not more.
    assert_eq!(bearer("/settings", &secret).await.unwrap().status(), 403);
    assert_eq!(bearer("/queue", "kura_wrong").await.unwrap().status(), 401);

    // The list never shows the key itself.
    let (_, keys) = server.get("/account/keys", &sam).await;
    assert_eq!(keys[0]["name"], "Musubi");
    assert!(keys[0].get("secret").is_none());
    assert!(keys[0]["last_used_at"].is_i64());

    let id = created["key"]["id"].as_i64().unwrap();
    let path = format!("/account/keys/{id}");
    // Only its owner deletes it.
    assert_eq!(
        server
            .call(Method::DELETE, &path, &admin, Value::Null)
            .await
            .0,
        404
    );
    assert_eq!(
        server
            .call(Method::DELETE, &path, &sam, Value::Null)
            .await
            .0,
        204
    );
    assert_eq!(bearer("/queue", &secret).await.unwrap().status(), 401);
}

#[tokio::test]
async fn every_user_changes_their_own_account() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user" });
    server.call(Method::POST, "/users", &admin, user).await;
    let sam = server.log_in("/login", "sam", "correct horse").await;

    let (code, account) = server.get("/account", &sam).await;
    assert_eq!(code, 200);
    assert_eq!(account["email"], Value::Null);
    assert_eq!(account["theme"]["music"], true);
    assert_eq!(account["theme"]["ascii"], false);

    let taken = json!({ "name": "Admin" });
    assert_eq!(
        server
            .call(Method::PUT, "/account/name", &sam, taken)
            .await
            .0,
        409
    );
    let renamed = server.call(
        Method::PUT,
        "/account/name",
        &sam,
        json!({ "name": "Samira" }),
    );
    assert_eq!(renamed.await.1["name"], "Samira");
    server.log_in("/login", "samira", "correct horse").await;

    let bad = json!({ "email": "not an address", "password": "correct horse" });
    assert_eq!(
        server
            .call(Method::PUT, "/account/email", &sam, bad)
            .await
            .0,
        400
    );
    // The email decides where reset links go: it needs the current password (a wrong one is
    // refused; right after logging in, none is needed, see `the_email_prompt_is_shown_once`).
    let wrong = json!({ "email": "sam@example.org", "password": "wrong horse" });
    let refused = server.call(Method::PUT, "/account/email", &sam, wrong);
    assert_eq!(refused.await.0, 403);
    let email = json!({ "email": "sam@example.org", "password": "correct horse" });
    let (_, account) = server
        .call(Method::PUT, "/account/email", &sam, email)
        .await;
    assert_eq!(account["email"], "sam@example.org");
    let none = json!({ "email": "", "password": "correct horse" });
    let cleared = server.call(Method::PUT, "/account/email", &sam, none);
    assert_eq!(cleared.await.1["email"], Value::Null);

    let theme = json!({ "music": false, "ascii": true, "ascii_piece": "tokyo-rain" });
    let (_, account) = server
        .call(Method::PUT, "/account/theme", &sam, theme)
        .await;
    assert_eq!(account["theme"]["ascii_piece"], "tokyo-rain");
    let odd = json!({ "music": true, "ascii": true, "ascii_piece": "<script>" });
    assert_eq!(
        server
            .call(Method::PUT, "/account/theme", &sam, odd)
            .await
            .0,
        400
    );

    // Users still see neither the user list nor the settings.
    assert_eq!(server.get("/users", &sam).await.0, 403);
}

#[tokio::test]
async fn admins_manage_the_songs_and_everyone_lists_them() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user" });
    server.call(Method::POST, "/users", &admin, user).await;
    let sam = server.log_in("/login", "sam", "correct horse").await;

    assert_eq!(server.get("/theme/songs", &sam).await, (200, json!([])));
    assert_eq!(server.get("/theme/songs/abc", &sam).await.0, 404);
    let upload = |cookie: &str| {
        server
            .request(Method::POST, "/admin/theme/songs?name=Song.mp3", cookie)
            .body(b"ID3fake".to_vec())
            .send()
    };
    assert_eq!(upload(&sam).await.unwrap().status(), 403);
    // In-memory servers have no data folder to keep songs in.
    assert_eq!(upload(&admin).await.unwrap().status(), 400);
    let remove = server.call(Method::DELETE, "/admin/theme/songs/abc", &sam, Value::Null);
    assert_eq!(remove.await.0, 403);
}

#[tokio::test]
async fn keys_cannot_make_keys_and_end_with_the_password() {
    let server = Server::start().await;
    server.log_in("/setup", "admin", "correct horse").await;
    let admin = server.log_in("/login", "admin", "correct horse").await;
    let other = server.log_in("/login", "admin", "correct horse").await;
    let (_, created) = server
        .call(
            Method::POST,
            "/account/keys",
            &admin,
            json!({ "name": "Musubi" }),
        )
        .await;
    let secret = created["secret"].as_str().unwrap().to_owned();
    let with_key = |method: Method, path: &str, body: Value| {
        let url = format!("{}/api/v1{path}", server.base);
        let request = server.http.request(method, url).bearer_auth(secret.clone());
        async move {
            let request = if body.is_null() {
                request
            } else {
                request.json(&body)
            };
            request.send().await.unwrap().status().as_u16()
        }
    };

    // A key that got out can neither make itself new keys, nor delete keys, nor change the
    // password and lock out the owner.
    let minted = with_key(Method::POST, "/account/keys", json!({ "name": "spare" }));
    assert_eq!(minted.await, 403);
    let id = created["key"]["id"].as_i64().unwrap();
    assert_eq!(
        with_key(Method::DELETE, &format!("/account/keys/{id}"), Value::Null).await,
        403
    );
    let password = json!({ "current": "correct horse", "new": "battery staple" });
    assert_eq!(
        with_key(Method::PUT, "/account/password", password.clone()).await,
        403
    );
    // Everything else the account may do, the key still does.
    assert_eq!(with_key(Method::GET, "/users", Value::Null).await, 200);

    // A new password ends the other sessions and every key.
    let changed = server.call(Method::PUT, "/account/password", &admin, password);
    assert_eq!(changed.await.0, 204);
    assert_eq!(server.get("/account", &admin).await.0, 200);
    assert_eq!(server.get("/account", &other).await.0, 401);
    assert_eq!(with_key(Method::GET, "/account", Value::Null).await, 401);
    assert_eq!(server.get("/account/keys", &admin).await, (200, json!([])));
}

#[tokio::test]
async fn changes_from_other_websites_are_refused() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let host = server.base.trim_start_matches("http://").to_owned();
    let clear = |headers: &[(&str, &str)]| {
        let mut request = server.request(Method::POST, "/queue/clear", &admin);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        async move { request.send().await.unwrap().status().as_u16() }
    };

    // What a page on a neighbouring subdomain could send: `SameSite=Strict` lets the cookie
    // through, so the browser's own headers decide.
    assert_eq!(clear(&[("sec-fetch-site", "same-site")]).await, 403);
    assert_eq!(clear(&[("sec-fetch-site", "cross-site")]).await, 403);
    assert_eq!(clear(&[("origin", "https://evil.example.org")]).await, 403);
    assert_eq!(clear(&[("origin", "null")]).await, 403);
    let song = server.request(Method::POST, "/admin/theme/songs?name=x", &admin);
    let song = song
        .header("sec-fetch-site", "same-site")
        .header("content-type", "text/plain");
    assert_eq!(song.body("ID3").send().await.unwrap().status(), 403);

    // Kura's own pages, older browsers on the same address, and programs.
    assert_eq!(clear(&[("sec-fetch-site", "same-origin")]).await, 200);
    let own = format!("http://{host}");
    assert_eq!(clear(&[("origin", own.as_str())]).await, 200);
    let proxied = [
        ("origin", "https://kura.example.org"),
        ("x-forwarded-host", "kura.example.org"),
    ];
    assert_eq!(clear(&proxied).await, 200);
    assert_eq!(clear(&[]).await, 200);
    // Reading stays open: a page elsewhere cannot read the answer anyway.
    let read = server
        .request(Method::GET, "/queue", &admin)
        .header("sec-fetch-site", "cross-site");
    assert_eq!(read.send().await.unwrap().status(), 200);
}

#[tokio::test]
async fn pages_tell_the_browser_what_is_allowed() {
    let server = Server::start().await;
    for path in ["/", "/api/v1/status"] {
        let response = server
            .http
            .get(format!("{}{path}", server.base))
            .send()
            .await
            .unwrap();
        let headers = response.headers();
        let policy = headers["content-security-policy"].to_str().unwrap();
        assert!(policy.contains("frame-ancestors 'none'") && policy.contains("script-src 'self'"));
        assert_eq!(headers["x-frame-options"], "DENY");
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        // Plain HTTP, as at home: no HSTS, or the browser would refuse it afterwards.
        assert!(headers.get("strict-transport-security").is_none());
    }

    // Behind a trusted proxy that says the person came over HTTPS.
    let proxied = Server::behind(TrustedProxies::parse("127.0.0.1").unwrap()).await;
    let response = proxied.http.get(format!("{}/", proxied.base));
    let response = response
        .header("x-forwarded-proto", "https")
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()["strict-transport-security"],
        "max-age=31536000"
    );
}

/// A mail server that takes every mail without asking for a password, over plain SMTP, and
/// hands each one on as its raw text.
async fn fake_mail_server() -> (u16, tokio::sync::mpsc::UnboundedReceiver<String>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sent, received) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let sent = sent.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut lines = BufReader::new(read).lines();
                write.write_all(b"220 fake ESMTP\r\n").await.unwrap();
                while let Ok(Some(line)) = lines.next_line().await {
                    let command = line.to_ascii_uppercase();
                    let answer: &[u8] = if command.starts_with("EHLO") {
                        b"250-fake\r\n250 8BITMIME\r\n"
                    } else if command.starts_with("DATA") {
                        write.write_all(b"354 go on\r\n").await.unwrap();
                        let mut mail = String::new();
                        while let Ok(Some(line)) = lines.next_line().await {
                            if line == "." {
                                break;
                            }
                            mail.push_str(&line);
                            mail.push('\n');
                        }
                        let _ = sent.send(mail);
                        b"250 queued\r\n"
                    } else if command.starts_with("QUIT") {
                        let _ = write.write_all(b"221 bye\r\n").await;
                        break;
                    } else {
                        b"250 ok\r\n"
                    };
                    write.write_all(answer).await.unwrap();
                }
            });
        }
    });
    (port, received)
}

/// The text of every part of a mail, with quoted-printable and base64 undone.
fn readable(mail: &str) -> String {
    use base64::Engine;
    let mut out = mail.replace("=\n", "").replace("=3D", "=");
    for block in mail.split("\n\n") {
        let joined: String = block.lines().collect();
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(joined.trim()) {
            out.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    out
}

fn reset_token(mail: &str) -> String {
    let text = readable(mail);
    let start = text.find("reset-password#token=").expect("a reset link") + 21;
    text[start..start + 64].to_owned()
}

async fn next_mail(mails: &mut tokio::sync::mpsc::UnboundedReceiver<String>) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(10), mails.recv())
        .await
        .expect("a mail within 10 seconds")
        .unwrap()
}

fn plain_mail_server(port: u16) -> Value {
    json!({
        "host": "127.0.0.1", "port": port, "encryption": "none",
        "from": "Kura <noreply@example.org>", "public_url": "https://kura.example.org/",
    })
}

#[tokio::test]
async fn only_admins_set_the_mail_server_and_its_password_stays_hidden() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user" });
    server.call(Method::POST, "/users", &admin, user).await;
    let sam = server.log_in("/login", "sam", "correct horse").await;

    assert_eq!(server.get("/admin/mail", &sam).await.0, 403);
    let proton = json!({
        "host": "smtp.protonmail.ch", "port": 587, "encryption": "starttls",
        "username": "noreply@example.org", "password": "token-secret",
        "from": "Kura <noreply@example.org>", "public_url": "https://kura.example.org",
    });
    assert_eq!(
        server
            .call(Method::PUT, "/admin/mail", &sam, proton.clone())
            .await
            .0,
        403
    );
    let (code, saved) = server
        .call(Method::PUT, "/admin/mail", &admin, proton.clone())
        .await;
    assert_eq!(code, 200);
    assert_eq!(saved["password_set"], true);
    assert!(saved.get("password").is_none());
    let (_, shown) = server.get("/admin/mail", &admin).await;
    assert!(!shown.to_string().contains("token-secret"));
    assert_eq!(shown["enabled"], true);
    // Neither the settings nor their export carry it.
    let (_, settings) = server.get("/settings", &admin).await;
    assert!(!settings.to_string().contains("token-secret"));

    // An empty password keeps the stored one.
    let mut again = proton.clone();
    again["password"] = json!("");
    let (_, kept) = server.call(Method::PUT, "/admin/mail", &admin, again).await;
    assert_eq!(kept["password_set"], true);

    for (field, value) in [
        (
            "from",
            json!("Kura <noreply@example.org>\r\nBcc: someone@example.org"),
        ),
        ("from", json!("not an address")),
        ("host", json!("https://smtp.example.org")),
        ("public_url", json!("javascript:alert(1)")),
        ("public_url", json!("https://kura.example.org/?next=evil")),
        ("encryption", json!("none")),
        ("port", json!(0)),
    ] {
        let mut bad = proton.clone();
        bad[field] = value.clone();
        let code = server.call(Method::PUT, "/admin/mail", &admin, bad).await.0;
        assert_eq!(code, 400, "{field} = {value}");
    }
    // The status tells everyone whether "Forgot password?" works, nothing more.
    let (_, status) = server.get("/status", "").await;
    assert_eq!(status["password_reset"], true);
    assert!(!status.to_string().contains("protonmail"));
}

#[tokio::test]
async fn a_reset_link_works_once_and_tells_nobody_who_has_an_account() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let ask =
        |name: &str| server.call(Method::POST, "/password-reset", "", json!({ "name": name }));

    // Without a mail server the page says so plainly.
    let (code, answer) = ask("admin").await;
    assert_eq!(code, 503);
    assert!(answer["error"].as_str().unwrap().contains("not set up"));

    let (port, mut mails) = fake_mail_server().await;
    let saved = server.call(Method::PUT, "/admin/mail", &admin, plain_mail_server(port));
    assert_eq!(saved.await.0, 200);
    let sam = json!({ "name": "sam", "password": "correct horse", "role": "user", "email": "sam@example.org" });
    assert_eq!(
        server.call(Method::POST, "/users", &admin, sam).await.0,
        200
    );
    let lee = json!({ "name": "lee", "password": "correct horse", "role": "user" });
    server.call(Method::POST, "/users", &admin, lee).await;
    let session = server.log_in("/login", "sam", "correct horse").await;
    let (_, key) = server
        .call(
            Method::POST,
            "/account/keys",
            &session,
            json!({ "name": "Musubi" }),
        )
        .await;
    let key = key["secret"].as_str().unwrap().to_owned();

    // Same answer for an account with an email, one without, and none at all.
    let answers = [ask("sam").await, ask("lee").await, ask("nobody").await];
    assert!(
        answers.iter().all(|answer| answer == &answers[0]),
        "{answers:?}"
    );
    assert_eq!(answers[0].0, 202);

    let mail = next_mail(&mut mails).await;
    assert!(mail.contains("To: sam <sam@example.org>"), "{mail}");
    assert!(mail.contains("From: Kura <noreply@example.org>"), "{mail}");
    assert!(
        mail.contains("Subject: Kura: reset your password"),
        "{mail}"
    );
    let text = readable(&mail);
    assert!(
        text.contains("https://kura.example.org/reset-password#token="),
        "{text}"
    );
    let token = reset_token(&mail);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    // Only sam has an email: no other mail, and asking again at once sends nothing.
    assert_eq!(ask("SAM@example.org").await.0, 202);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(mails.try_recv().is_err(), "a second mail was sent");

    let confirm = |token: &str, password: &str| {
        let form = json!({ "token": token, "password": password });
        server.call(Method::POST, "/password-reset/confirm", "", form)
    };
    // A password that is too short does not use the link up.
    assert_eq!(confirm(&token, "short").await.0, 400);
    assert_eq!(confirm(&"0".repeat(64), "a new password").await.0, 400);
    assert_eq!(confirm("not hex", "a new password").await.0, 400);
    assert_eq!(confirm(&token, "a new password").await.0, 204);
    // Once only.
    assert_eq!(confirm(&token, "another password").await.0, 400);

    // Every session and key of the account ended; the new password works.
    assert_eq!(server.get("/account", &session).await.0, 401);
    let url = format!("{}/api/v1/account", server.base);
    let by_key = server.http.get(url).bearer_auth(key).send().await.unwrap();
    assert_eq!(by_key.status(), 401);
    server.log_in("/login", "sam", "a new password").await;
}

/// Audit finding: the brake counted per address only, so behind a proxy (or on one network)
/// five requests for any name held up resets and invite links for everyone. It now counts per
/// address and what is asked for.
#[tokio::test]
async fn reset_requests_are_braked_per_address_and_name() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let (port, _mails) = fake_mail_server().await;
    server
        .call(Method::PUT, "/admin/mail", &admin, plain_mail_server(port))
        .await;
    let server = &server;
    let ask = |name: &str| {
        let form = json!({ "name": name });
        async move {
            server
                .call(Method::POST, "/password-reset", "", form)
                .await
                .0
        }
    };
    let mut codes = Vec::new();
    for _ in 0..8 {
        codes.push(ask("someone").await);
    }
    assert_eq!(&codes[..5], &[202; 5]);
    assert_eq!(codes[7], 429, "{codes:?}");
    // However the name is written, it is the same name.
    assert_eq!(ask(" SOMEONE ").await, 429);
    // Someone else, from the same address, is not held up, and neither are invite links.
    assert_eq!(ask("sam").await, 202);
    let check = json!({ "token": "a".repeat(64) });
    assert_eq!(
        server
            .call(Method::POST, "/register/check", "", check)
            .await
            .0,
        400
    );
}

#[tokio::test]
async fn admins_manage_emails_and_send_a_test_mail() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user" });
    let (_, sam) = server.call(Method::POST, "/users", &admin, user).await;
    let id = sam["id"].as_i64().unwrap();
    let sam_session = server.log_in("/login", "sam", "correct horse").await;
    let path = format!("/users/{id}/email");

    let email = json!({ "email": "sam@example.org" });
    assert_eq!(
        server
            .call(Method::PUT, &path, &sam_session, email.clone())
            .await
            .0,
        403
    );
    assert_eq!(server.call(Method::PUT, &path, &admin, email).await.0, 204);
    let bad = json!({ "email": "sam@example.org\r\nBcc: x@example.org" });
    assert_eq!(server.call(Method::PUT, &path, &admin, bad).await.0, 400);
    assert_eq!(
        server
            .call(
                Method::PUT,
                "/users/999/email",
                &admin,
                json!({ "email": "" })
            )
            .await
            .0,
        404
    );
    let (_, users) = server.get("/users", &admin).await;
    assert_eq!(users[1]["email"], "sam@example.org");
    let bad_user =
        json!({ "name": "kim", "password": "correct horse", "role": "user", "email": "nope" });
    assert_eq!(
        server
            .call(Method::POST, "/users", &admin, bad_user)
            .await
            .0,
        400
    );
    assert_eq!(
        server
            .get("/users", &admin)
            .await
            .1
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let to = json!({ "to": "admin@example.org" });
    assert_eq!(
        server
            .call(Method::POST, "/admin/mail/test", &admin, to.clone())
            .await
            .0,
        503
    );
    let (port, mut mails) = fake_mail_server().await;
    server
        .call(Method::PUT, "/admin/mail", &admin, plain_mail_server(port))
        .await;
    assert_eq!(
        server
            .call(Method::POST, "/admin/mail/test", &sam_session, to.clone())
            .await
            .0,
        403
    );
    assert_eq!(
        server
            .call(Method::POST, "/admin/mail/test", &admin, to)
            .await
            .0,
        204
    );
    let mail = next_mail(&mut mails).await;
    assert!(mail.contains("Subject: Kura: test mail"), "{mail}");

    // A mail server that is not there: a clear error, not a hang.
    let gone = json!({ "host": "127.0.0.1", "port": 9, "encryption": "none",
        "from": "noreply@example.org", "public_url": "http://kura.local" });
    server.call(Method::PUT, "/admin/mail", &admin, gone).await;
    let (code, answer) = server
        .call(
            Method::POST,
            "/admin/mail/test",
            &admin,
            json!({ "to": "admin@example.org" }),
        )
        .await;
    assert_eq!(code, 502);
    assert!(
        answer["error"]
            .as_str()
            .unwrap()
            .starts_with("the mail server"),
        "{answer}"
    );
}

/// Audit finding: a key that got out set its account's email and took the account over with
/// a reset link. Neither a key nor a session without the password can move the email now.
#[tokio::test]
async fn a_key_cannot_redirect_reset_links() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let (port, _mails) = fake_mail_server().await;
    server
        .call(Method::PUT, "/admin/mail", &admin, plain_mail_server(port))
        .await;
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user" });
    let (_, sam) = server.call(Method::POST, "/users", &admin, user).await;
    let id = sam["id"].as_i64().unwrap();
    let mut keys = Vec::new();
    for (who, password) in [("sam", "correct horse"), ("admin", "correct horse")] {
        let session = server.log_in("/login", who, password).await;
        let (_, key) = server
            .call(
                Method::POST,
                "/account/keys",
                &session,
                json!({ "name": "Musubi" }),
            )
            .await;
        keys.push(key["secret"].as_str().unwrap().to_owned());
    }
    let with_key = |key: &str, path: &str, body: Value| {
        let url = format!("{}/api/v1{path}", server.base);
        let request = server.http.put(url).bearer_auth(key.to_owned()).json(&body);
        async move { request.send().await.unwrap().status().as_u16() }
    };
    let attacker = json!({ "email": "attacker@example.net", "password": "correct horse" });
    assert_eq!(
        with_key(&keys[0], "/account/email", attacker.clone()).await,
        403
    );
    // An admin's key cannot redirect someone else's links either.
    assert_eq!(
        with_key(&keys[1], &format!("/users/{id}/email"), attacker).await,
        403
    );
    let (_, users) = server.get("/users", &admin).await;
    assert_eq!(users[1]["email"], Value::Null);
}

/// Audit finding: the stored mail password went along to whatever server was entered next.
#[tokio::test]
async fn the_mail_password_stays_with_its_server() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let proton = json!({
        "host": "smtp.protonmail.ch", "port": 587, "encryption": "starttls",
        "username": "noreply@example.org", "password": "token-secret",
        "from": "Kura <noreply@example.org>", "public_url": "https://kura.example.org",
    });
    server
        .call(Method::PUT, "/admin/mail", &admin, proton.clone())
        .await;
    let mut same = proton.clone();
    same["password"] = json!("");
    let (_, kept) = server
        .call(Method::PUT, "/admin/mail", &admin, same.clone())
        .await;
    assert_eq!(kept["password_set"], true);
    for (field, value) in [
        ("host", json!("smtp.attacker.example")),
        ("port", json!(2587)),
        ("encryption", json!("tls")),
        ("username", json!("other@example.org")),
    ] {
        server
            .call(Method::PUT, "/admin/mail", &admin, proton.clone())
            .await;
        let mut moved = same.clone();
        moved[field] = value;
        let (_, saved) = server.call(Method::PUT, "/admin/mail", &admin, moved).await;
        assert_eq!(saved["password_set"], false, "{field}");
    }
}

#[tokio::test]
async fn the_email_prompt_is_shown_once() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    // The first admin is asked too.
    assert_eq!(server.get("/account", &admin).await.1["ask_email"], true);
    for name in ["sam", "kim"] {
        let user = json!({ "name": name, "password": "correct horse", "role": "user" });
        server.call(Method::POST, "/users", &admin, user).await;
    }
    let with_email = json!({ "name": "lee", "password": "correct horse", "role": "user", "email": "lee@example.org" });
    server
        .call(Method::POST, "/users", &admin, with_email)
        .await;

    // "Not now": gone, also after logging in again elsewhere.
    let sam = server.log_in("/login", "sam", "correct horse").await;
    assert_eq!(server.get("/account", &sam).await.1["ask_email"], true);
    let skipped = server.call(Method::POST, "/account/email/skip", &sam, Value::Null);
    assert_eq!(skipped.await.0, 204);
    let elsewhere = server.log_in("/login", "sam", "correct horse").await;
    assert_eq!(
        server.get("/account", &elsewhere).await.1["ask_email"],
        false
    );

    // Right after logging in, the email is set without typing the password again.
    let kim = server.log_in("/login", "kim", "correct horse").await;
    let email = json!({ "email": "kim@example.org" });
    let (code, account) = server
        .call(Method::PUT, "/account/email", &kim, email)
        .await;
    assert_eq!(code, 200);
    assert_eq!(
        (account["email"].as_str(), &account["ask_email"]),
        (Some("kim@example.org"), &json!(false))
    );
    // An API key cannot, even of a fresh account.
    let (_, key) = server
        .call(Method::POST, "/account/keys", &kim, json!({ "name": "k" }))
        .await;
    let url = format!("{}/api/v1/account/email", server.base);
    let by_key = server
        .http
        .put(url)
        .bearer_auth(key["secret"].as_str().unwrap())
        .json(&json!({ "email": "x@example.org" }));
    assert_eq!(by_key.send().await.unwrap().status(), 403);

    // An account with an email is never asked.
    let lee = server.log_in("/login", "lee", "correct horse").await;
    assert_eq!(server.get("/account", &lee).await.1["ask_email"], false);
}

fn invite_token(mail: &str) -> String {
    let text = readable(mail);
    let start = text.find("register#token=").expect("an invite link") + 15;
    text[start..start + 64].to_owned()
}

#[tokio::test]
async fn admins_set_a_password_for_someone_else() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user" });
    let (_, sam) = server.call(Method::POST, "/users", &admin, user).await;
    let id = sam["id"].as_i64().unwrap();
    let sam_session = server.log_in("/login", "sam", "correct horse").await;
    let path = format!("/users/{id}/password");
    let new = json!({ "password": "battery staple" });

    assert_eq!(
        server
            .call(Method::PUT, &path, &sam_session, new.clone())
            .await
            .0,
        403
    );
    let (_, key) = server
        .call(
            Method::POST,
            "/account/keys",
            &admin,
            json!({ "name": "k" }),
        )
        .await;
    let url = format!("{}/api/v1{path}", server.base);
    let by_key = server
        .http
        .put(url)
        .bearer_auth(key["secret"].as_str().unwrap())
        .json(&new);
    assert_eq!(by_key.send().await.unwrap().status(), 403);
    let short = json!({ "password": "short" });
    assert_eq!(server.call(Method::PUT, &path, &admin, short).await.0, 400);
    assert_eq!(
        server
            .call(Method::PUT, "/users/999/password", &admin, new.clone())
            .await
            .0,
        404
    );

    assert_eq!(server.call(Method::PUT, &path, &admin, new).await.0, 204);
    assert_eq!(server.get("/account", &sam_session).await.0, 401);
    server.log_in("/login", "sam", "battery staple").await;
    // The admin's own session goes on.
    assert_eq!(server.get("/account", &admin).await.0, 200);
}

#[tokio::test]
async fn an_invite_creates_one_account_with_the_invited_email_and_role() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let to = json!({ "email": "kim@example.org", "role": "user" });

    // Without a mail server, inviting says so.
    assert_eq!(
        server
            .call(Method::POST, "/users/invite", &admin, to.clone())
            .await
            .0,
        503
    );
    let (port, mut mails) = fake_mail_server().await;
    server
        .call(Method::PUT, "/admin/mail", &admin, plain_mail_server(port))
        .await;

    // Only admins invite.
    let user = json!({ "name": "sam", "password": "correct horse", "role": "user", "email": "sam@example.org" });
    server.call(Method::POST, "/users", &admin, user).await;
    let sam = server.log_in("/login", "sam", "correct horse").await;
    assert_eq!(
        server
            .call(Method::POST, "/users/invite", &sam, to.clone())
            .await
            .0,
        403
    );
    assert_eq!(server.get("/invites", &sam).await.0, 403);
    // An address an account has already.
    let taken = json!({ "email": "SAM@example.org" });
    assert_eq!(
        server
            .call(Method::POST, "/users/invite", &admin, taken)
            .await
            .0,
        409
    );

    let (code, invite) = server.call(Method::POST, "/users/invite", &admin, to).await;
    assert_eq!(code, 200, "{invite}");
    let mail = next_mail(&mut mails).await;
    assert!(mail.contains("To: kim@example.org"), "{mail}");
    assert!(mail.contains("Subject: Kura: you are invited"), "{mail}");
    let token = invite_token(&mail);
    let (_, listed) = server.get("/invites", &admin).await;
    assert_eq!(listed[0]["email"], "kim@example.org");

    let check = |token: &str| {
        server.call(
            Method::POST,
            "/register/check",
            "",
            json!({ "token": token }),
        )
    };
    let (code, shown) = check(&token).await;
    assert_eq!((code, &shown["email"]), (200, &json!("kim@example.org")));

    // A taken name keeps the invite; a role or email in the body is ignored.
    let register = |name: &str| {
        let body = json!({ "token": token, "name": name, "password": "correct horse",
            "role": "admin", "email": "other@example.org" });
        server.call(Method::POST, "/register", "", body)
    };
    assert_eq!(register("Sam").await.0, 409);
    let response = server
        .request(Method::POST, "/register", "")
        .json(
            &json!({ "token": token, "name": "kim", "password": "correct horse", "role": "admin" }),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let (_, account) = server.get("/account", &cookie).await;
    assert_eq!(account["role"], "user");
    assert_eq!(account["email"], "kim@example.org");
    assert_eq!(account["ask_email"], false);

    // Once only.
    assert_eq!(register("kim2").await.0, 400);
    assert_eq!(check(&token).await.0, 400);
    assert_eq!(server.get("/invites", &admin).await.1, json!([]));
}

#[tokio::test]
async fn nobody_signs_up_without_an_admin() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let users = || async {
        server
            .get("/users", &admin)
            .await
            .1
            .as_array()
            .unwrap()
            .len()
    };

    // Setup is closed once an account exists.
    let again = json!({ "name": "intruder", "password": "correct horse" });
    assert_eq!(server.call(Method::POST, "/setup", "", again).await.0, 409);
    // No token, a made-up one, garbage: nothing is created.
    for body in [
        json!({ "name": "intruder", "password": "correct horse" }),
        json!({ "token": "", "name": "intruder", "password": "correct horse" }),
        json!({ "token": "0".repeat(64), "name": "intruder", "password": "correct horse" }),
        json!({ "token": "' OR 1=1 --", "name": "intruder", "password": "correct horse" }),
    ] {
        let code = server
            .call(Method::POST, "/register", "", body.clone())
            .await
            .0;
        assert!(code == 400 || code == 422, "{body}: {code}");
    }
    // Creating users directly is for admins.
    let direct = json!({ "name": "intruder", "password": "correct horse", "role": "admin" });
    assert_eq!(
        server
            .call(Method::POST, "/users", "", direct.clone())
            .await
            .0,
        401
    );
    assert_eq!(users().await, 1);

    // A revoked invite is dead.
    let (port, mut mails) = fake_mail_server().await;
    server
        .call(Method::PUT, "/admin/mail", &admin, plain_mail_server(port))
        .await;
    let (_, invite) = server
        .call(
            Method::POST,
            "/users/invite",
            &admin,
            json!({ "email": "lee@example.org" }),
        )
        .await;
    let token = invite_token(&next_mail(&mut mails).await);
    let path = format!("/invites/{}", invite["id"]);
    assert_eq!(
        server
            .call(Method::DELETE, &path, &admin, Value::Null)
            .await
            .0,
        204
    );
    let body = json!({ "token": token, "name": "lee", "password": "correct horse" });
    assert_eq!(
        server.call(Method::POST, "/register", "", body).await.0,
        400
    );
    assert_eq!(users().await, 1);
}

/// Audit finding: an admin's API key could create an admin with a password, log in as it and
/// lock the owner out, or send reset links to another address. Everything that decides who
/// controls Kura now needs a login with a password.
#[tokio::test]
async fn an_admin_key_cannot_take_over_kura() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let (port, _mails) = fake_mail_server().await;
    server
        .call(Method::PUT, "/admin/mail", &admin, plain_mail_server(port))
        .await;
    let (_, created) = server
        .call(
            Method::POST,
            "/account/keys",
            &admin,
            json!({ "name": "Musubi" }),
        )
        .await;
    let secret = created["secret"].as_str().unwrap().to_owned();
    let with_key = |method: Method, path: &str, body: Value| {
        let url = format!("{}/api/v1{path}", server.base);
        let request = server.http.request(method, url).bearer_auth(secret.clone());
        async move {
            let request = if body.is_null() {
                request
            } else {
                request.json(&body)
            };
            request.send().await.unwrap().status().as_u16()
        }
    };

    let backdoor = json!({ "name": "backdoor", "password": "backdoor pw 1", "role": "admin" });
    assert_eq!(with_key(Method::POST, "/users", backdoor).await, 403);
    assert_eq!(with_key(Method::DELETE, "/users/1", Value::Null).await, 403);
    let mut elsewhere = plain_mail_server(port);
    elsewhere["public_url"] = json!("https://evil.example");
    assert_eq!(with_key(Method::PUT, "/admin/mail", elsewhere).await, 403);
    let to = json!({ "to": "admin@example.org" });
    assert_eq!(with_key(Method::POST, "/admin/mail/test", to).await, 403);
    let invite = json!({ "email": "attacker@example.net", "role": "admin" });
    assert_eq!(with_key(Method::POST, "/users/invite", invite).await, 403);
    assert_eq!(
        with_key(Method::DELETE, "/invites/1", Value::Null).await,
        403
    );
    let (_, settings) = server.get("/settings", &admin).await;
    assert_eq!(with_key(Method::PUT, "/settings", settings).await, 403);

    // Nothing changed, and the key still does what a program needs.
    let (_, users) = server.get("/users", &admin).await;
    assert_eq!(users.as_array().unwrap().len(), 1);
    let (_, mail) = server.get("/admin/mail", &admin).await;
    assert_ne!(mail["public_url"], "https://evil.example");
    assert_eq!(with_key(Method::GET, "/queue", Value::Null).await, 200);
    assert_eq!(with_key(Method::GET, "/users", Value::Null).await, 200);
}

/// A Jellyfin that names itself only to requests that carry `key`.
async fn fake_jellyfin(key: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = vec![0u8; 4096];
            let read = stream.read(&mut request).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]).into_owned();
            let answer = if request.contains(key) {
                let body = r#"{"ServerName":"den","Version":"10.10"}"#;
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_owned()
            };
            let _ = stream.write_all(answer.as_bytes()).await;
        }
    });
    url
}

/// Audit finding: the settings showed the Jellyfin API key to every admin and every admin's
/// API key. Like the mail password, it is now only saved, never shown, and kept only for the
/// server it was entered for.
#[tokio::test]
async fn the_jellyfin_key_is_kept_but_never_shown() {
    let server = Server::start().await;
    let admin = server.log_in("/setup", "admin", "correct horse").await;
    let jellyfin = fake_jellyfin("JELLY-KEY-1").await;
    let folder = tempfile::tempdir().unwrap();
    let (_, mut settings) = server.get("/settings", &admin).await;
    settings["libraries"] = json!([{ "name": "Main", "path": folder.path().join("main") }]);
    settings["jellyfin"] = json!({ "url": jellyfin, "api_key": "JELLY-KEY-1" });
    let (code, saved) = server
        .call(Method::PUT, "/settings", &admin, settings.clone())
        .await;
    assert_eq!(code, 200);
    assert!(!saved.to_string().contains("JELLY-KEY-1"), "{saved}");
    let (_, shown) = server.get("/settings", &admin).await;
    assert!(!shown.to_string().contains("JELLY-KEY-1"), "{shown}");
    assert_eq!(shown["jellyfin"]["api_key"], "");
    assert_eq!(shown["jellyfin"]["api_key_set"], true);

    // Saving what the browser got back, with the empty key, keeps the key; testing uses it.
    let (code, _) = server
        .call(Method::PUT, "/settings", &admin, shown.clone())
        .await;
    assert_eq!(code, 200);
    let test = json!({ "url": format!("{jellyfin}/"), "api_key": "" });
    let (code, answer) = server
        .call(Method::POST, "/admin/jellyfin/test", &admin, test)
        .await;
    assert_eq!((code, &answer["server"]), (200, &json!("den 10.10")));

    // Another address does not get the stored key, neither to test nor to keep.
    let elsewhere = fake_jellyfin("JELLY-KEY-1").await;
    let test = json!({ "url": elsewhere, "api_key": "" });
    let (code, _) = server
        .call(Method::POST, "/admin/jellyfin/test", &admin, test)
        .await;
    assert_eq!(code, 400);
    let mut moved = shown.clone();
    moved["jellyfin"]["url"] = json!(elsewhere);
    let (code, _) = server.call(Method::PUT, "/settings", &admin, moved).await;
    assert_eq!(code, 400);
}
