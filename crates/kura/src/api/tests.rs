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
    assert_eq!(fresh["version"], env!("CARGO_PKG_VERSION"));
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

    let bad = json!({ "email": "not an address" });
    assert_eq!(
        server
            .call(Method::PUT, "/account/email", &sam, bad)
            .await
            .0,
        400
    );
    let email = json!({ "email": "sam@example.org" });
    let (_, account) = server
        .call(Method::PUT, "/account/email", &sam, email)
        .await;
    assert_eq!(account["email"], "sam@example.org");
    let cleared = server.call(Method::PUT, "/account/email", &sam, json!({ "email": "" }));
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
