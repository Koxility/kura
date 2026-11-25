//! The JSON API under `/api/v1` and the embedded web app. No logic lives here: handlers
//! check who is asking, call `Core` and turn the answer into JSON.

use std::{convert::Infallible, net::SocketAddr, sync::Arc};

use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{ConnectInfo, DefaultBodyLimit, FromRequestParts, Path, Query, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri,
        header::{
            AUTHORIZATION, CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, COOKIE, ETAG,
            HOST, IF_NONE_MATCH, ORIGIN, REFERRER_POLICY, SET_COOKIE, STRICT_TRANSPORT_SECURITY,
            X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
        },
        request::Parts,
    },
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event as SseEvent, KeepAlive, Sse},
    },
    routing::{delete, get, post, put},
};
use kura_core::{
    Choices, Core, CoreError, Entry, Event, LibraryRoot, REPOSITORY, SESSION_TTL, Selection,
    SyncStatus,
};
use kura_domain::{
    Account, ApiKey, Episode, Genre, Job, Language, Picked, Request, Role, Series, SeriesSummary,
    Settings, SiteId, Theme, TitleRef, User,
};
use rust_embed::RustEmbed;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_stream::{StreamExt, wrappers::BroadcastStream};
use tower_http::{compression::CompressionLayer, set_header::SetResponseHeaderLayer};

use crate::proxy::TrustedProxies;

type App = State<Arc<Core>>;
type ApiResult<T> = Result<T, ApiError>;

/// Answers requests on `listener` until it fails. `proxies` are the reverse proxies whose
/// `X-Forwarded-For` and `X-Forwarded-Proto` are believed.
pub async fn serve(
    listener: tokio::net::TcpListener,
    core: Arc<Core>,
    proxies: TrustedProxies,
) -> std::io::Result<()> {
    // Logins need the address a connection comes from.
    let app = router(core)
        .layer(Extension(Arc::new(proxies)))
        .into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app).await
}

/// Who is asking, as far as the connection and a trusted proxy tell.
type Proxies = Extension<Arc<TrustedProxies>>;

fn router(core: Arc<Core>) -> Router {
    let api = Router::new()
        .route("/status", get(status))
        .route("/setup", post(setup))
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/password-reset", post(request_reset))
        .route("/password-reset/confirm", post(confirm_reset))
        .route("/register/check", post(check_invite))
        .route("/register", post(register))
        .route("/account", get(account))
        .route("/account/password", put(change_password))
        .route("/account/name", put(rename))
        .route("/account/email", put(set_email))
        .route("/account/email/skip", post(skip_email))
        .route("/account/email/confirm", post(confirm_email))
        .route("/account/theme", put(set_theme))
        .route("/account/keys", get(api_keys).post(create_api_key))
        .route("/account/keys/{id}", delete(delete_api_key))
        .route("/theme/songs", get(songs))
        .route("/theme/songs/{id}", get(song))
        .route(
            "/admin/theme/songs",
            // Some room above the limit, so a song just over it gets the clear error.
            post(add_song).layer(DefaultBodyLimit::max(kura_core::MOST_MUSIC_BYTES + 1024)),
        )
        .route("/admin/theme/songs/{id}", delete(remove_song))
        .route("/search", get(search))
        .route("/popular/{site}", get(popular))
        .route("/new/{site}", get(latest))
        .route("/new-episodes/{site}", get(new_episodes))
        .route("/calendar/{site}", get(calendar))
        .route("/genres/{site}", get(genres))
        .route("/genres/{site}/{genre}", get(by_genre))
        .route("/random/{site}", get(random))
        .route("/series/{site}/{slug}", get(series))
        .route("/series/{site}/{slug}/seasons/{season}", get(episodes))
        .route("/series/{site}/{slug}/downloaded", get(downloaded))
        .route("/queue", get(jobs).post(enqueue))
        .route("/queue/clear", post(clear_finished))
        .route("/queue/{id}/cancel", post(cancel))
        .route("/queue/{id}/retry", post(retry))
        .route("/requests", get(requests).post(create_request))
        .route("/requests/clear", post(clear_requests))
        .route("/requests/{id}/cancel", post(cancel_request))
        .route("/requests/{id}/accept", post(accept_request))
        .route("/requests/{id}/reject", post(reject_request))
        .route("/events", get(events))
        .route("/settings", get(settings).put(save_settings))
        .route("/autosync", get(sync_status))
        .route("/autosync/run", post(sync_now))
        .route("/autosync/exclude", post(exclude))
        .route("/autosync/remove", post(remove_tracked))
        .route("/library", get(library_roots))
        .route("/library/{root}", get(browse))
        .route("/admin/ip", get(public_ip))
        .route("/admin/jellyfin/test", post(test_jellyfin))
        .route("/admin/mail", get(mail_settings).put(save_mail))
        .route("/admin/mail/test", post(test_mail))
        .route("/users", get(users).post(create_user))
        .route("/users/{id}", delete(delete_user))
        .route("/users/{id}/email", put(set_user_email))
        .route("/users/{id}/password", put(set_user_password))
        .route("/users/invite", post(invite))
        .route("/invites", get(invites))
        .route("/invites/{id}", delete(revoke_invite))
        .fallback(|| async { ApiError(StatusCode::NOT_FOUND, "no such endpoint".into()) })
        // Answers change with every download; neither the browser nor a proxy may keep one.
        .layer(SetResponseHeaderLayer::if_not_present(
            CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ));
    let header = |name: HeaderName, value: &'static str| {
        SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
    };
    Router::new()
        .nest("/api/v1", api)
        .fallback(asset)
        .with_state(core)
        // Leaves the event stream alone: its default predicate skips `text/event-stream`.
        .layer(CompressionLayer::new())
        // What the browser may do with Kura's pages: run only Kura's own scripts, show covers
        // from the sites, and never show Kura inside another website's frame.
        .layer(header(CONTENT_SECURITY_POLICY, CONTENT_SECURITY))
        .layer(header(X_FRAME_OPTIONS, "DENY"))
        .layer(header(X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(header(REFERRER_POLICY, "no-referrer"))
        .layer(middleware::from_fn(strict_transport))
}

/// Scripts, styles, fonts and requests stay on Kura's own address; covers come from the sites
/// over HTTPS; songs play from Kura. `'unsafe-inline'` for styles only, for the `style`
/// attributes React sets.
const CONTENT_SECURITY: &str = "default-src 'self'; script-src 'self'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' https: data:; media-src 'self' blob:; \
     font-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; \
     form-action 'self'; frame-ancestors 'none'";

/// Over HTTPS (as a trusted proxy reports it), browsers are told to keep to HTTPS for this
/// host. Not over plain HTTP: Kura often runs that way at home, and a browser that once saw
/// the header would refuse the plain address afterwards.
async fn strict_transport(
    Extension(proxies): Proxies,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let https = proxies.client(from.ip(), request.headers()).https;
    let mut response = next.run(request).await;
    if https {
        response.headers_mut().insert(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    response
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
            CoreError::TooManyLogins(_)
            | CoreError::TooManyResets(_)
            | CoreError::TooManyTries(_) => StatusCode::TOO_MANY_REQUESTS,
            CoreError::MailNotSetUp => StatusCode::SERVICE_UNAVAILABLE,
            CoreError::Conflict(_) => StatusCode::CONFLICT,
            CoreError::Mail(_) => StatusCode::BAD_GATEWAY,
            CoreError::Jellyfin(_) => StatusCode::BAD_GATEWAY,
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

/// A logged-in user, by the session cookie of the web app or by an API key another program
/// sends as `Authorization: Bearer kura_…`. Handlers that take this refuse everyone else.
struct Session {
    user: User,
    /// The session token; `None` when an API key was sent.
    token: Option<String>,
}

impl FromRequestParts<Arc<Core>> for Session {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, core: &Arc<Core>) -> ApiResult<Self> {
        let denied = || error(StatusCode::UNAUTHORIZED, "log in first");
        caller(core, &parts.method, &parts.headers)?.ok_or_else(denied)
    }
}

/// Who sends a request: the session cookie wins, then an API key. A cookie that comes with a
/// change sent from another website is refused: the browser adds it on its own, also when a
/// page elsewhere (even on a neighbouring subdomain, which `SameSite` lets through) sends it.
fn caller(core: &Core, method: &Method, headers: &HeaderMap) -> ApiResult<Option<Session>> {
    if let Some(token) = session_token(headers)
        && let Some(user) = core.session(token)?
    {
        if changes(method) && from_elsewhere(headers) {
            return Err(error(
                StatusCode::FORBIDDEN,
                "this request came from another website",
            ));
        }
        return Ok(Some(Session {
            user,
            token: Some(token.to_owned()),
        }));
    }
    let key = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match key {
        Some(key) => {
            let user = core.api_key_user(key.trim())?;
            // A key that was sent and is wrong is an error, not a quiet "logged out".
            let user = user.ok_or_else(|| error(StatusCode::UNAUTHORIZED, "unknown API key"))?;
            Ok(Some(Session { user, token: None }))
        }
        None => Ok(None),
    }
}

fn changes(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Whether the browser says the request was started by a page that is not Kura. Browsers
/// send `Sec-Fetch-Site`, which no page can set; older ones only `Origin`, which is then
/// compared with the address Kura was asked under. Programs that send neither are not
/// browsers and are let through.
fn from_elsewhere(headers: &HeaderMap) -> bool {
    let text = |name| {
        headers
            .get(name)
            .and_then(|value: &HeaderValue| value.to_str().ok())
    };
    if let Some(site) = text(HeaderName::from_static("sec-fetch-site")) {
        return !matches!(site, "same-origin" | "none");
    }
    let Some(origin) = text(ORIGIN) else {
        return false;
    };
    let host = text(HeaderName::from_static("x-forwarded-host")).or(text(HOST));
    let origin = origin.split_once("://").map(|(_, rest)| rest);
    match (origin, host) {
        (Some(origin), Some(host)) => {
            let host = host.split(',').next().unwrap_or_default().trim();
            !origin.eq_ignore_ascii_case(host)
        }
        // `Origin: null` (a sandboxed frame, a file) or no address to compare with.
        _ => true,
    }
}

/// A login in the browser, not an API key: for the password and the keys themselves, so a
/// key that got out cannot make itself new keys or lock out the account's owner.
fn in_browser(session: &Session) -> ApiResult<&str> {
    session.token.as_deref().ok_or_else(|| {
        error(
            StatusCode::FORBIDDEN,
            "log in with your password for this; an API key cannot change passwords, emails, keys, accounts or the server settings",
        )
    })
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

/// A logged-in admin with a password login, not an API key: for what decides who controls
/// Kura (accounts, invites, the mail server, the settings), so a key that got out cannot make
/// itself an admin login or send reset links elsewhere.
struct BrowserAdmin(User);

impl FromRequestParts<Arc<Core>> for BrowserAdmin {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, core: &Arc<Core>) -> ApiResult<Self> {
        let session = Session::from_request_parts(parts, core).await?;
        if session.user.role != Role::Admin {
            return Err(error(StatusCode::FORBIDDEN, "only an admin can do this"));
        }
        in_browser(&session)?;
        Ok(Self(session.user))
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

/// `SameSite=Strict` keeps other websites from acting with the session. `Secure`, which keeps
/// the browser from sending it over plain HTTP, is only set when a trusted proxy says the
/// person came over HTTPS: Kura usually runs on plain HTTP in a home network, where a
/// `Secure` cookie would not be kept at all.
fn session_cookie(token: &str, max_age: i64, https: bool) -> [(axum::http::HeaderName, String); 1] {
    let secure = if https { "; Secure" } else { "" };
    let cookie = format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}"
    );
    [(SET_COOKIE, cookie)]
}

// Accounts

#[derive(Deserialize)]
struct Credentials {
    name: String,
    password: String,
}

async fn status(State(core): App, headers: HeaderMap) -> ApiResult<Json<Value>> {
    let user = caller(&core, &Method::GET, &headers)?.map(|session| session.user);
    // Which build runs, and so which known flaws it has, is for those who use it.
    let known = user.is_some();
    Ok(Json(json!({
        "needs_setup": core.needs_setup()?,
        "user": user,
        "version": known.then_some(env!("CARGO_PKG_VERSION")),
        "commit": known.then_some(env!("KURA_COMMIT")),
        "backdrop": core.settings()?.backdrop,
        "repository": format!("https://github.com/{REPOSITORY}"),
        "latest_version": if known { core.newer_release() } else { None },
        "needs_library": core.settings()?.libraries.is_empty(),
        // Whether the login page offers "Forgot password?".
        "password_reset": core.password_reset_enabled()?,
        "sites": core.enabled_sites()?,
        // What a title's page shows as chosen before anyone picks; library names stay with
        // admins.
        "defaults": match &user {
            Some(user) => {
                let mut defaults = serde_json::to_value(core.defaults()?)
                    .map_err(|err| error(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()))?;
                if user.role != Role::Admin {
                    defaults["libraries"] = json!({});
                }
                defaults
            }
            None => Value::Null,
        },
        // Names only, for the library choice on a title's page; paths stay with admins.
        "libraries": match &user {
            Some(user) if user.role == Role::Admin => {
                let libraries = core.settings()?.libraries.into_iter();
                libraries.map(|library| library.name).collect()
            }
            _ => Vec::new(),
        },
        // Offered by the library setup; the folder the server runs in is a sensible start.
        // Like every path on the server it is for admins, not for whoever asks.
        "suggested_library_path": match &user {
            Some(user) if user.role == Role::Admin => suggested_library_path(),
            _ => None,
        },
    })))
}

fn suggested_library_path() -> Option<String> {
    std::env::var("KURA_LIBRARY_DIR").ok().or_else(|| {
        let here = std::env::current_dir().ok()?;
        Some(here.join("downloads").to_string_lossy().into_owned())
    })
}

async fn setup(
    State(core): App,
    from: ConnectInfo<SocketAddr>,
    proxies: Proxies,
    headers: HeaderMap,
    Json(form): Json<Credentials>,
) -> ApiResult<Response> {
    if core.setup(&form.name, &form.password).await?.is_none() {
        return Err(error(StatusCode::CONFLICT, "Kura is already set up"));
    }
    login(State(core), from, proxies, headers, Json(form)).await
}

async fn login(
    State(core): App,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Extension(proxies): Proxies,
    headers: HeaderMap,
    Json(form): Json<Credentials>,
) -> ApiResult<Response> {
    // Behind a reverse proxy the connection is the proxy's; the brake counts the person.
    let client = proxies.client(from.ip(), &headers);
    let logged_in = core.login(&form.name, &form.password, client.address);
    let Some((user, token)) = logged_in.await? else {
        return Err(error(StatusCode::UNAUTHORIZED, "wrong name or password"));
    };
    let cookie = session_cookie(&token, SESSION_TTL, client.https);
    Ok((cookie, Json(user)).into_response())
}

#[derive(Deserialize)]
struct ResetRequest {
    /// A name or an email address.
    name: String,
}

/// Asks for a reset link. The answer is the same whether or not the account exists, has an
/// email address or just got a link; only a missing mail server and the brake show.
async fn request_reset(
    State(core): App,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Extension(proxies): Proxies,
    headers: HeaderMap,
    Json(form): Json<ResetRequest>,
) -> ApiResult<Response> {
    let client = proxies.client(from.ip(), &headers);
    core.request_password_reset(&form.name, client.address)?;
    let message = "If an account with an email address matches, a link to choose a new password \
                   is on its way. It works for 30 minutes.";
    Ok((StatusCode::ACCEPTED, Json(json!({ "message": message }))).into_response())
}

#[derive(Deserialize)]
struct ResetConfirm {
    token: String,
    password: String,
}

async fn confirm_reset(State(core): App, Json(form): Json<ResetConfirm>) -> ApiResult<StatusCode> {
    if core.reset_password(&form.token, &form.password).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(
            StatusCode::BAD_REQUEST,
            "this link has expired or was already used; ask for a new one",
        ))
    }
}

#[derive(Deserialize)]
struct InviteToken {
    token: String,
}

/// What an invite link stands for, so the page can show the address before the form.
async fn check_invite(
    State(core): App,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Extension(proxies): Proxies,
    headers: HeaderMap,
    Json(form): Json<InviteToken>,
) -> ApiResult<Json<Value>> {
    let client = proxies.client(from.ip(), &headers);
    match core.check_invite(&form.token, client.address)? {
        Some(invite) => Ok(Json(json!({ "email": invite.email, "role": invite.role }))),
        None => Err(invalid_invite()),
    }
}

fn invalid_invite() -> ApiError {
    error(
        StatusCode::BAD_REQUEST,
        "this invite link is invalid, was already used or has expired; ask an admin for a new one",
    )
}

/// Only these fields count: email and role come from the invite, whatever else is sent.
#[derive(Deserialize)]
struct Registration {
    token: String,
    name: String,
    password: String,
}

/// Creates the account an invite stands for and logs it in. The only way to an account without
/// an admin at hand, and only with a link an admin sent.
async fn register(
    State(core): App,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Extension(proxies): Proxies,
    headers: HeaderMap,
    Json(form): Json<Registration>,
) -> ApiResult<Response> {
    let client = proxies.client(from.ip(), &headers);
    let registered = core
        .register(&form.token, &form.name, &form.password, client.address)
        .await?;
    let user = match registered {
        kura_core::Registered::Created(user) => user,
        kura_core::Registered::NameTaken => {
            return Err(error(StatusCode::CONFLICT, "this name is taken"));
        }
        kura_core::Registered::Invalid => return Err(invalid_invite()),
    };
    let logged_in = core
        .login(&user.name, &form.password, client.address)
        .await?;
    let Some((user, token)) = logged_in else {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the new account could not log in",
        ));
    };
    let cookie = session_cookie(&token, SESSION_TTL, client.https);
    Ok((cookie, Json(user)).into_response())
}

#[derive(Deserialize)]
struct NewPassword {
    current: String,
    new: String,
}

/// Every account changes its own password, logged in with the old one; the other sessions
/// and every API key of that account end.
async fn change_password(
    State(core): App,
    session: Session,
    Json(form): Json<NewPassword>,
) -> ApiResult<StatusCode> {
    let keep = in_browser(&session)?;
    let changed = core.change_password(&session.user, keep, &form.current, &form.new);
    if changed.await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(
            StatusCode::FORBIDDEN,
            "the current password is wrong",
        ))
    }
}

async fn logout(
    State(core): App,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Extension(proxies): Proxies,
    headers: HeaderMap,
    session: Session,
) -> ApiResult<Response> {
    if let Some(token) = &session.token {
        core.logout(token)?;
    }
    let https = proxies.client(from.ip(), &headers).https;
    Ok((session_cookie("", 0, https), StatusCode::NO_CONTENT).into_response())
}

async fn account(State(core): App, session: Session) -> ApiResult<Json<Account>> {
    Ok(Json(core.account(&session.user)?))
}

#[derive(Deserialize)]
struct NewName {
    name: String,
}

async fn rename(
    State(core): App,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Extension(proxies): Proxies,
    headers: HeaderMap,
    session: Session,
    Json(form): Json<NewName>,
) -> ApiResult<Json<Account>> {
    let client = proxies.client(from.ip(), &headers);
    if !core.rename(&session.user, &form.name, client.address)? {
        return Err(error(StatusCode::CONFLICT, "this name is taken"));
    }
    Ok(Json(core.account(&session.user)?))
}

#[derive(Deserialize)]
struct NewEmail {
    #[serde(default)]
    email: String,
    /// The current password: the email decides where reset links go, so changing it is as
    /// much as changing the password.
    #[serde(default)]
    password: String,
}

async fn set_email(
    State(core): App,
    session: Session,
    Json(form): Json<NewEmail>,
) -> ApiResult<Json<Account>> {
    let token = in_browser(&session)?;
    // Right after logging in, the password was just typed; that is what the prompt after the
    // first login relies on. Later, it is asked again.
    let proven = if form.password.is_empty() {
        core.recent_login(token)?
    } else {
        core.check_own_password(&session.user, &form.password)
            .await?
    };
    if !proven {
        let message = if form.password.is_empty() {
            "enter your current password to change the email"
        } else {
            "the current password is wrong"
        };
        return Err(error(StatusCode::FORBIDDEN, message));
    }
    // A new address counts once the link mailed to it is opened; the account then shows it
    // as `pending_email`.
    core.change_email(&session.user, &form.email).await?;
    Ok(Json(core.account(&session.user)?))
}

/// Opens the link mailed to a new address; whoever has the link may, also logged out or in
/// another browser.
async fn confirm_email(
    State(core): App,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Extension(proxies): Proxies,
    headers: HeaderMap,
    Json(form): Json<InviteToken>,
) -> ApiResult<StatusCode> {
    let client = proxies.client(from.ip(), &headers);
    if core.confirm_email(&form.token, client.address)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(
            StatusCode::BAD_REQUEST,
            "this link has expired or was already used; enter the address again for a new one",
        ))
    }
}

/// "Not now" to the email prompt after login; it is not shown again.
async fn skip_email(State(core): App, session: Session) -> ApiResult<StatusCode> {
    core.skip_email(&session.user)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_theme(
    State(core): App,
    session: Session,
    Json(theme): Json<Theme>,
) -> ApiResult<Json<Account>> {
    core.set_theme(&session.user, &theme)?;
    Ok(Json(core.account(&session.user)?))
}

async fn api_keys(State(core): App, session: Session) -> ApiResult<Json<Vec<ApiKey>>> {
    Ok(Json(core.api_keys(&session.user)?))
}

/// The key itself is in this answer and nowhere else, ever.
async fn create_api_key(
    State(core): App,
    session: Session,
    Json(form): Json<NewName>,
) -> ApiResult<Json<Value>> {
    in_browser(&session)?;
    let (key, secret) = core.create_api_key(&session.user, &form.name)?;
    Ok(Json(json!({ "key": key, "secret": secret })))
}

async fn delete_api_key(
    State(core): App,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    in_browser(&session)?;
    if core.delete_api_key(&session.user, id)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(StatusCode::NOT_FOUND, "no such key"))
    }
}

async fn songs(State(core): App, _: Session) -> ApiResult<Json<Vec<kura_domain::Song>>> {
    Ok(Json(core.songs()?))
}

/// One song. Kept by the browser and checked with its tag, so a page load does not fetch
/// megabytes again.
async fn song(
    State(core): App,
    _: Session,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let music = core.song(&id)?;
    let music = music.ok_or_else(|| error(StatusCode::NOT_FOUND, "no such song"))?;
    let cache = [
        (CACHE_CONTROL, "private, no-cache".to_owned()),
        (ETAG, music.etag.clone()),
    ];
    let unchanged = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|tag| tag == music.etag);
    if unchanged {
        return Ok((StatusCode::NOT_MODIFIED, cache).into_response());
    }
    Ok((
        cache,
        [(CONTENT_TYPE, "audio/mpeg".to_owned())],
        music.bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct SongName {
    #[serde(default)]
    name: String,
}

/// The MP3 is the body; its name comes in the query, taken from the file name.
async fn add_song(
    State(core): App,
    _: Admin,
    Query(form): Query<SongName>,
    body: Bytes,
) -> ApiResult<Json<kura_domain::Song>> {
    Ok(Json(core.add_song(&form.name, &body)?))
}

async fn remove_song(State(core): App, _: Admin, Path(id): Path<String>) -> ApiResult<StatusCode> {
    if core.remove_song(&id)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(StatusCode::NOT_FOUND, "no such song"))
    }
}

#[derive(Deserialize)]
struct NewUser {
    name: String,
    password: String,
    role: Role,
    /// For password resets; optional.
    #[serde(default)]
    email: String,
}

async fn users(State(core): App, _: Admin) -> ApiResult<Json<Vec<kura_domain::UserEntry>>> {
    Ok(Json(core.user_entries()?))
}

async fn create_user(
    State(core): App,
    _: BrowserAdmin,
    Json(new): Json<NewUser>,
) -> ApiResult<Json<User>> {
    // Checked before the account exists, so a typo or a taken address does not leave a
    // half-made user.
    let email = new.email.trim();
    if !email.is_empty() {
        core.check_email_free(email)?;
    }
    let created = core.create_user(&new.name, &new.password, new.role).await?;
    let taken = || error(StatusCode::CONFLICT, "this name is taken");
    let created = created.ok_or_else(taken)?;
    if !email.is_empty()
        && let Err(err) = core.set_user_email(created.id, email)
    {
        // Someone took the address in the meantime.
        core.delete_user(created.id)?;
        return Err(err.into());
    }
    Ok(Json(created))
}

#[derive(Deserialize)]
struct EmailForm {
    #[serde(default)]
    email: String,
}

/// Logged in with a password, not an API key: the email decides where reset links go.
async fn set_user_email(
    State(core): App,
    _: BrowserAdmin,
    Path(id): Path<i64>,
    Json(form): Json<EmailForm>,
) -> ApiResult<StatusCode> {
    if core.set_user_email(id, &form.email)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(StatusCode::NOT_FOUND, "no such user"))
    }
}

#[derive(Deserialize)]
struct PasswordForm {
    password: String,
}

/// An admin sets someone's password, logged in with their own password, not by API key.
async fn set_user_password(
    State(core): App,
    _: BrowserAdmin,
    Path(id): Path<i64>,
    Json(form): Json<PasswordForm>,
) -> ApiResult<StatusCode> {
    if core.set_user_password(id, &form.password).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(StatusCode::NOT_FOUND, "no such user"))
    }
}

#[derive(Deserialize)]
struct InviteForm {
    email: String,
    #[serde(default = "user_role")]
    role: Role,
}

fn user_role() -> Role {
    Role::User
}

async fn invite(
    State(core): App,
    BrowserAdmin(admin): BrowserAdmin,
    Json(form): Json<InviteForm>,
) -> ApiResult<Json<kura_domain::Invite>> {
    Ok(Json(core.invite(&form.email, form.role, &admin).await?))
}

async fn invites(State(core): App, _: Admin) -> ApiResult<Json<Vec<kura_domain::Invite>>> {
    Ok(Json(core.invites()?))
}

async fn revoke_invite(
    State(core): App,
    _: BrowserAdmin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    if core.revoke_invite(id)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(error(StatusCode::NOT_FOUND, "no such invite"))
    }
}

/// The mail settings as the browser sees them: everything but the password.
fn mail_view(mail: &kura_domain::Mail) -> Value {
    json!({
        "host": mail.host,
        "port": mail.port,
        "encryption": mail.encryption,
        "username": mail.username,
        "password_set": !mail.password.is_empty(),
        "from": mail.from,
        "public_url": mail.public_url,
        "enabled": mail.enabled(),
    })
}

async fn mail_settings(State(core): App, _: Admin) -> ApiResult<Json<Value>> {
    Ok(Json(mail_view(&core.mail()?)))
}

/// An empty `password` keeps the stored one.
async fn save_mail(
    State(core): App,
    _: BrowserAdmin,
    Json(mail): Json<kura_domain::Mail>,
) -> ApiResult<Json<Value>> {
    Ok(Json(mail_view(&core.save_mail(&mail)?)))
}

#[derive(Deserialize)]
struct TestMail {
    to: String,
}

async fn test_mail(
    State(core): App,
    _: BrowserAdmin,
    Json(form): Json<TestMail>,
) -> ApiResult<StatusCode> {
    core.send_test_mail(&form.to).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_user(
    State(core): App,
    _: BrowserAdmin,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
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

async fn new_episodes(
    State(core): App,
    _: Session,
    Path(site): Path<SiteId>,
) -> ApiResult<Json<Vec<kura_domain::NewEpisode>>> {
    Ok(Json(core.new_episodes(site).await?))
}

async fn calendar(
    State(core): App,
    _: Session,
    Path(site): Path<SiteId>,
) -> ApiResult<Json<Vec<kura_domain::CalendarDay>>> {
    Ok(Json(core.calendar(site).await?))
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

/// Title and IMDb ID as the title page shows them, to find the series' folder by name or ID.
#[derive(Deserialize)]
struct SeriesNames {
    title: String,
    imdb: Option<String>,
}

/// `[season, episode]` pairs that are already in a library.
async fn downloaded(
    State(core): App,
    _: Session,
    Path((site, slug)): Path<(SiteId, String)>,
    Query(names): Query<SeriesNames>,
) -> ApiResult<Json<Vec<(u32, u32)>>> {
    // Read the library again: files may have been deleted or added since the last look. Not
    // more often than every half minute, however often it is asked: a scan reads every
    // folder of every library.
    core.forget_scan_older_than(std::time::Duration::from_secs(30));
    let found = core.downloaded_episodes(site, &slug, &names.title, names.imdb.as_deref());
    Ok(Json(found?))
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

/// Everyone sees the queue, but where the files are on the server is for admins.
async fn jobs(State(core): App, session: Session) -> ApiResult<Json<Vec<Job>>> {
    let mut jobs = core.jobs()?;
    if session.user.role != Role::Admin {
        let libraries = core.settings()?.libraries.into_iter();
        let mut folders: Vec<String> = libraries
            .map(|library| library.path.to_string_lossy().into_owned())
            .collect();
        for job in &mut jobs {
            without_paths(job, &mut folders);
        }
    }
    Ok(Json(jobs))
}

/// Takes the folders on the server out of a job. `libraries` are the library paths; the
/// job's own folder joins them, since it may be from a library that is gone by now.
fn without_paths(job: &mut Job, libraries: &mut Vec<String>) {
    let dir = std::mem::take(&mut job.dir);
    if !dir.is_empty() && !libraries.contains(&dir) {
        libraries.push(dir);
    }
    // The longest first, so a library inside another one is replaced whole.
    libraries.sort_by_key(|folder| std::cmp::Reverse(folder.len()));
    job.path = None;
    // What ffmpeg says about a failed download can name the file it wrote to.
    if let Some(error) = &mut job.error {
        for folder in libraries.iter().filter(|folder| !folder.is_empty()) {
            *error = error.replace(folder.as_str(), "<library>");
        }
    }
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

/// A user withdraws one of their own open requests.
async fn cancel_request(
    State(core): App,
    session: Session,
    Path(id): Path<i64>,
) -> ApiResult<StatusCode> {
    core.cancel_request(id, session.user.id)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn clear_requests(State(core): App, _: Admin) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "cleared": core.clear_requests()? })))
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

async fn events(State(core): App, _: Session) -> impl IntoResponse {
    let events = BroadcastStream::new(core.subscribe()).map(|event| {
        // A listener that fell behind lost events; this makes it read the queue again.
        let event = event.unwrap_or(Event::QueueChanged);
        Ok::<_, Infallible>(
            SseEvent::default()
                .json_data(event)
                .unwrap_or_else(|_| SseEvent::default().comment("unreadable event")),
        )
    });
    let stream = Sse::new(events).keep_alive(KeepAlive::default());
    // nginx and similar proxies hold back a response until a buffer fills, which delays
    // events by minutes; this asks them to pass each event on at once.
    let headers = [
        ("x-accel-buffering", "no"),
        ("cache-control", "no-cache, no-transform"),
    ];
    (headers, stream)
}

// Settings

/// The settings as the browser sees them: the Jellyfin API key only as whether one is
/// saved, like the mail password; plus the naming template that applies while they name none.
fn settings_view(settings: &Settings) -> ApiResult<Value> {
    let mut view = serde_json::to_value(settings)
        .map_err(|err| error(StatusCode::INTERNAL_SERVER_ERROR, &err.to_string()))?;
    view["jellyfin"]["api_key"] = json!("");
    view["jellyfin"]["api_key_set"] = json!(!settings.jellyfin.api_key.trim().is_empty());
    view["naming_template_default"] = json!(kura_core::default_template());
    Ok(view)
}

async fn settings(State(core): App, _: Admin) -> ApiResult<Json<Value>> {
    Ok(Json(settings_view(&core.settings()?)?))
}

/// An empty Jellyfin `api_key` keeps the stored one, for the same server.
async fn save_settings(
    State(core): App,
    _: BrowserAdmin,
    Json(settings): Json<Settings>,
) -> ApiResult<Json<Value>> {
    Ok(Json(settings_view(&core.save_settings(&settings)?)?))
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

/// Takes a title off the tracked list until one of its episodes is downloaded again.
async fn remove_tracked(
    State(core): App,
    _: Admin,
    Json(title): Json<TitleRef>,
) -> ApiResult<StatusCode> {
    if title.slug.len() > 200 || title.title.len() > 300 {
        return Err(error(StatusCode::BAD_REQUEST, "the title is too long"));
    }
    core.remove_tracked(title)?;
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

/// Checks the Jellyfin settings sent along, so they can be tried before they are saved.
async fn test_jellyfin(
    State(core): App,
    _: Admin,
    Json(jellyfin): Json<kura_domain::Jellyfin>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({ "server": core.test_jellyfin(&jellyfin).await? }),
    ))
}

// Web app

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct WebApp;

async fn asset(uri: Uri, headers: HeaderMap) -> Response {
    let path = uri.path().trim_start_matches('/');
    // Anything that is not a file is a page of the single-page app.
    let found = WebApp::get(path)
        .map(|file| (path, file))
        .or_else(|| WebApp::get("index.html").map(|file| ("index.html", file)));
    match found {
        Some((path, file)) => {
            let tag = format!("\"{}\"", hex(&file.metadata.sha256_hash()[..16]));
            // Vite names these after their content, so they never change; the page itself
            // is checked every time, so a new version shows at once.
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            let matches = headers
                .get(IF_NONE_MATCH)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.split(',').any(|other| other.trim() == tag));
            let common = [(ETAG, tag), (CACHE_CONTROL, cache.to_owned())];
            if matches {
                return (StatusCode::NOT_MODIFIED, common).into_response();
            }
            let kind = file.metadata.mimetype().to_owned();
            (common, [(CONTENT_TYPE, kind)], file.data).into_response()
        }
        None => {
            let hint =
                "The web app is not built yet. Run `bun run build` in web/, then build Kura again.";
            (StatusCode::NOT_FOUND, hint).into_response()
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
