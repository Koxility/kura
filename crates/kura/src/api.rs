//! The JSON API under `/api/v1` and the embedded web app. No logic lives here: handlers
//! check who is asking, call `Core` and turn the answer into JSON.

use std::{convert::Infallible, sync::Arc};

use axum::{
    Json, Router,
    extract::{FromRequestParts, Path, Query, State},
    http::{
        HeaderMap, StatusCode, Uri,
        header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
        request::Parts,
    },
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use kura_core::{
    Choices, Core, CoreError, Entry, Event, LibraryRoot, REPOSITORY, SESSION_TTL, Selection,
    SyncStatus,
};
use kura_domain::{
    Episode, Genre, Job, Language, Picked, Request, Role, Series, SeriesSummary, Settings, SiteId,
    TitleRef, User,
};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};

type App = State<Arc<Core>>;
type ApiResult<T> = Result<T, ApiError>;

pub fn router(core: Arc<Core>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/setup", post(setup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/search", get(search))
        .route("/popular/{site}", get(popular))
        .route("/new/{site}", get(latest))
        .route("/genres/{site}", get(genres))
        .route("/genres/{site}/{genre}", get(by_genre))
        .route("/random/{site}", get(random))
        .route("/series/{site}/{slug}", get(series))
        .route("/series/{site}/{slug}/seasons/{season}", get(episodes))
        .route("/queue", get(jobs).post(enqueue))
        .route("/queue/clear", post(clear_finished))
        .route("/queue/{id}/cancel", post(cancel))
        .route("/queue/{id}/retry", post(retry))
        .route("/requests", get(requests).post(create_request))
        .route("/requests/{id}/accept", post(accept_request))
        .route("/requests/{id}/reject", post(reject_request))
        .route("/events", get(events))
        .route("/settings", get(settings).put(save_settings))
        .route("/autosync", get(sync_status))
        .route("/autosync/run", post(sync_now))
        .route("/autosync/exclude", post(exclude))
        .route("/library", get(library_roots))
        .route("/library/{root}", get(browse))
        .route("/admin/ip", get(public_ip))
        .route("/users", get(users).post(create_user))
        .route("/users/{id}", delete(delete_user))
        .fallback(|| async { ApiError(StatusCode::NOT_FOUND, "no such endpoint".into()) });
    Router::new()
        .nest("/api/v1", api)
        .fallback(asset)
        .with_state(core)
}

// Errors

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<CoreError> for ApiError {
    fn from(err: CoreError) -> Self {
        use kura_core::SiteError;
        let status = match &err {
            CoreError::Invalid(_)
            | CoreError::Folder(..)
            | CoreError::Site(SiteError::BadSlug(_)) => StatusCode::BAD_REQUEST,
            CoreError::UnknownSite | CoreError::SiteDisabled | CoreError::UnknownJob => {
                StatusCode::NOT_FOUND
            }
            // The site did not answer or changed its pages.
            CoreError::Site(_) => StatusCode::BAD_GATEWAY,
            CoreError::Store(_) | CoreError::Http(_) | CoreError::Hashing(_) => {
                tracing::error!("request failed: {err}");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        Self(status, err.to_string())
    }
}

fn error(status: StatusCode, message: &str) -> ApiError {
    ApiError(status, message.to_owned())
}

// Who is asking

const COOKIE_NAME: &str = "kura_session";

/// A logged-in user. Handlers that take this refuse everyone else.
struct Session {
    user: User,
    token: String,
}

impl FromRequestParts<Arc<Core>> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, core: &Arc<Core>) -> ApiResult<Self> {
        let denied = || error(StatusCode::UNAUTHORIZED, "log in first");
        let token = session_token(&parts.headers).ok_or_else(denied)?;
        let user = core.session(token)?.ok_or_else(denied)?;
        Ok(Self {
            user,
            token: token.to_owned(),
        })
    }
}

/// A logged-in admin.
struct Admin(User);

impl FromRequestParts<Arc<Core>> for Admin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, core: &Arc<Core>) -> ApiResult<Self> {
        let user = Session::from_request_parts(parts, core).await?.user;
        match user.role {
            Role::Admin => Ok(Self(user)),
            Role::User => Err(error(StatusCode::FORBIDDEN, "only an admin can do this")),
        }
    }
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|pair| pair.trim().strip_prefix(COOKIE_NAME)?.strip_prefix('='))
}

/// `SameSite=Strict` keeps other websites from acting with the session. `Secure` is left
/// out because Kura usually runs on plain HTTP in a home network.
fn session_cookie(token: &str, max_age: i64) -> [(axum::http::HeaderName, String); 1] {
    let cookie =
        format!("{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}");
    [(SET_COOKIE, cookie)]
}

// Accounts

#[derive(Deserialize)]
struct Credentials {
    name: String,
    password: String,
}

async fn status(State(core): App, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let user = match session_token(&headers) {
        Some(token) => core.session(token)?,
        None => None,
    };
    Ok(Json(json!({
        "needs_setup": core.needs_setup()?,
        "user": user,
        "version": env!("CARGO_PKG_VERSION"),
        "repository": format!("https://github.com/{REPOSITORY}"),
        "update_available": core.update_available(),
        "needs_library": core.settings()?.libraries.is_empty(),
        "sites": core.enabled_sites()?,
        // Names only, for the library choice on a title's page; paths stay with admins.
        "libraries": match &user {
            Some(user) if user.role == Role::Admin => {
                let libraries = core.settings()?.libraries.into_iter();
                libraries.map(|library| library.name).collect()
            }
            _ => Vec::new(),
        },
        // Offered by the library setup; the folder the server runs in is a sensible start.
        "suggested_library_path": std::env::current_dir()
            .map(|dir| dir.join("downloads").to_string_lossy().into_owned())
            .ok(),
    })))
}

async fn setup(State(core): App, Json(form): Json<Credentials>) -> ApiResult<Response> {
    if core.setup(&form.name, &form.password).await?.is_none() {
        return Err(error(StatusCode::CONFLICT, "Kura is already set up"));
    }
    login(State(core), Json(form)).await
}

async fn login(State(core): App, Json(form): Json<Credentials>) -> ApiResult<Response> {
    let Some((user, token)) = core.login(&form.name, &form.password).await? else {
        return Err(error(StatusCode::UNAUTHORIZED, "wrong name or password"));
    };
    Ok((session_cookie(&token, SESSION_TTL), Json(user)).into_response())
}

async fn logout(State(core): App, session: Session) -> ApiResult<Response> {
    core.logout(&session.token)?;
    Ok((session_cookie("", 0), StatusCode::NO_CONTENT).into_response())
}

#[derive(Deserialize)]
struct NewUser {
    name: String,
    password: String,
    role: Role,
}

async fn users(State(core): App, _: Admin) -> ApiResult<Json<Vec<User>>> {
    Ok(Json(core.users()?))
}

async fn create_user(
    State(core): App,
    _: Admin,
    Json(new): Json<NewUser>,
) -> ApiResult<Json<User>> {
    let created = core.create_user(&new.name, &new.password, new.role).await?;
    let taken = || error(StatusCode::CONFLICT, "this name is taken");
    Ok(Json(created.ok_or_else(taken)?))
}

async fn delete_user(State(core): App, _: Admin, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    if core.delete_user(id)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        let message = "no such user, or it is the last admin";
        Err(error(StatusCode::CONFLICT, message))
    }
}

// Browsing

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    /// Leave out to search every site.
    site: Option<SiteId>,
}

async fn search(
    State(core): App,
    _: Session,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<Vec<SeriesSummary>>> {
    Ok(Json(core.search(&query.q, query.site).await?))
}

async fn popular(
    State(core): App,
    _: Session,
    Path(site): Path<SiteId>,
) -> ApiResult<Json<Vec<SeriesSummary>>> {
    Ok(Json(core.popular(site).await?))
}

async fn latest(
    State(core): App,
    _: Session,
    Path(site): Path<SiteId>,
) -> ApiResult<Json<Vec<SeriesSummary>>> {
    Ok(Json(core.latest(site).await?))
}

async fn genres(
    State(core): App,
    _: Session,
    Path(site): Path<SiteId>,
) -> ApiResult<Json<Vec<Genre>>> {
    Ok(Json(core.genres(site).await?))
}

#[derive(Deserialize)]
struct Page {
    #[serde(default = "first_page")]
    page: u32,
}

fn first_page() -> u32 {
    1
}

async fn by_genre(
    State(core): App,
    _: Session,
    Path((site, genre)): Path<(SiteId, String)>,
    Query(Page { page }): Query<Page>,
) -> ApiResult<Json<Vec<SeriesSummary>>> {
    Ok(Json(core.by_genre(site, &genre, page).await?))
}

async fn random(
    State(core): App,
    _: Session,
    Path(site): Path<SiteId>,
) -> ApiResult<Json<SeriesSummary>> {
    Ok(Json(core.random(site).await?))
}

async fn series(
    State(core): App,
    _: Session,
    Path((site, slug)): Path<(SiteId, String)>,
) -> ApiResult<Json<Series>> {
    Ok(Json(core.series(site, &slug).await?))
}

async fn episodes(
    State(core): App,
    _: Session,
    Path((site, slug, season)): Path<(SiteId, String, u32)>,
) -> ApiResult<Json<Vec<Episode>>> {
    Ok(Json(core.episodes(site, &slug, season).await?))
}

// Queue

/// `all` queues the whole series. Otherwise `seasons` lists whole seasons and `episodes`
/// single `[season, episode]` pairs. Language, hoster and library are optional choices.
#[derive(Deserialize)]
struct Enqueue {
    site: SiteId,
    slug: String,
    #[serde(default)]
    all: bool,
    #[serde(default)]
    seasons: Vec<u32>,
    #[serde(default)]
    episodes: Vec<(u32, u32)>,
    language: Option<Language>,
    hoster: Option<String>,
    library: Option<String>,
    /// For the folder name when the site has no IMDb ID: `tt…` or `none`.
    imdb_id: Option<String>,
}

impl Enqueue {
    fn picked(&self) -> Picked {
        Picked {
            all: self.all,
            seasons: self.seasons.clone(),
            episodes: self.episodes.clone(),
        }
    }

    fn choices(&self) -> Choices {
        Choices {
            language: self.language,
            hoster: self.hoster.clone(),
            library: self.library.clone(),
            imdb_id: self.imdb_id.clone(),
        }
    }
}

async fn jobs(State(core): App, _: Session) -> ApiResult<Json<Vec<Job>>> {
    Ok(Json(core.jobs()?))
}

async fn enqueue(
    State(core): App,
    Admin(admin): Admin,
    Json(wanted): Json<Enqueue>,
) -> ApiResult<Json<Value>> {
    let selection = Selection::from_picked(&wanted.picked());
    let choices = wanted.choices();
    let queued = core.enqueue(wanted.site, &wanted.slug, selection, &choices, admin.id);
    Ok(Json(json!({ "queued": queued.await? })))
}

async fn clear_finished(State(core): App, _: Admin) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "cleared": core.clear_finished()? })))
}

async fn cancel(State(core): App, _: Admin, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "cancelled": core.cancel(id)? })))
}

async fn retry(
    State(core): App,
    Admin(admin): Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "queued": core.retry(id, admin.id)? })))
}

// Requests

async fn requests(State(core): App, session: Session) -> ApiResult<Json<Vec<Request>>> {
    let own = (session.user.role != Role::Admin).then_some(session.user.id);
    Ok(Json(core.requests(own)?))
}

/// Same body as queueing; a library choice is ignored, the admin's settings decide.
async fn create_request(
    State(core): App,
    session: Session,
    Json(wanted): Json<Enqueue>,
) -> ApiResult<Json<Value>> {
    let choices = Choices {
        library: None,
        ..wanted.choices()
    };
    let user = session.user.id;
    let picked = wanted.picked();
    let id = core.request(wanted.site, &wanted.slug, &picked, &choices, user);
    Ok(Json(json!({ "id": id.await? })))
}

async fn accept_request(
    State(core): App,
    Admin(admin): Admin,
    Path(id): Path<i64>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({ "queued": core.accept_request(id, admin.id).await? }),
    ))
}

async fn reject_request(State(core): App, _: Admin, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    core.reject_request(id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn events(
    State(core): App,
    _: Session,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let events = BroadcastStream::new(core.subscribe()).map(|event| {
        // A listener that fell behind lost events; this makes it read the queue again.
        let event = event.unwrap_or(Event::QueueChanged);
        Ok(SseEvent::default()
            .json_data(event)
            .unwrap_or_else(|_| SseEvent::default().comment("unreadable event")))
    });
    Sse::new(events).keep_alive(KeepAlive::default())
}

// Settings

async fn settings(State(core): App, _: Admin) -> ApiResult<Json<Settings>> {
    Ok(Json(core.settings()?))
}

async fn save_settings(
    State(core): App,
    _: Admin,
    Json(settings): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    core.save_settings(&settings)?;
    Ok(Json(settings))
}

#[derive(Deserialize)]
struct Exclude {
    #[serde(flatten)]
    title: TitleRef,
    excluded: bool,
}

/// Open to every user: it is a per-title choice made on the title's page.
async fn exclude(State(core): App, _: Admin, Json(form): Json<Exclude>) -> ApiResult<StatusCode> {
    if form.title.slug.len() > 200 || form.title.title.len() > 300 {
        return Err(error(StatusCode::BAD_REQUEST, "the title is too long"));
    }
    core.set_excluded(form.title, form.excluded)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn sync_status(State(core): App, _: Admin) -> ApiResult<Json<SyncStatus>> {
    Ok(Json(core.sync_status()?))
}

/// Checks every tracked title now.
async fn sync_now(State(core): App, _: Admin) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "queued": core.sync_once().await? })))
}

// Library

async fn library_roots(State(core): App, _: Admin) -> ApiResult<Json<Vec<LibraryRoot>>> {
    Ok(Json(core.library_roots()?))
}

#[derive(Deserialize)]
struct Browse {
    /// Folder below the library, e.g. `Series/Some Title`. Empty for the library itself.
    #[serde(default)]
    path: String,
}

async fn browse(
    State(core): App,
    _: Admin,
    Path(root): Path<usize>,
    Query(query): Query<Browse>,
) -> ApiResult<Json<Vec<Entry>>> {
    Ok(Json(core.browse(root, &query.path)?))
}

// Administration

async fn public_ip(State(core): App, _: Admin) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "ip": core.public_ip().await? })))
}

// Web app

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct WebApp;

async fn asset(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Anything that is not a file is a page of the single-page app.
    let found = WebApp::get(path)
        .map(|file| (path, file))
        .or_else(|| WebApp::get("index.html").map(|file| ("index.html", file)));
    match found {
        Some((_, file)) => {
            let kind = file.metadata.mimetype().to_owned();
            ([(CONTENT_TYPE, kind)], file.data).into_response()
        }
        None => {
            let hint =
                "The web app is not built yet. Run `bun run build` in web/, then build Kura again.";
            (StatusCode::NOT_FOUND, hint).into_response()
        }
    }
}

#[cfg(test)]
mod tests;
