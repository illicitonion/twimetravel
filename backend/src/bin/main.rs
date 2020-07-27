extern crate env_logger;
extern crate gotham;
#[macro_use]
extern crate gotham_derive;
extern crate hyper;
#[macro_use]
extern crate log;
extern crate mime;
extern crate mime_guess;
extern crate oauthcli;
extern crate reqwest;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
extern crate time;
extern crate toml;
extern crate twimetravel;
extern crate url;
extern crate walkdir;

use gotham::router::builder::{DefineSingleRoute, DrawRoutes};
use gotham::state::FromState;
use hyper::header::AccessControlAllowOrigin;
use mime_guess::from_ext;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use twimetravel::{
    oauth, Context, Interval, SecondsSinceUnixEpoch, TweetStore, UniquelyIdentifiedTimeValue,
};
use walkdir::WalkDir;

fn main() {
    env_logger::init();
    let config: Config = {
        let bytes = read_file("config.toml");
        toml::from_slice(&bytes).expect("Deserializing config")
    };

    let mut static_bytes = HashMap::new();
    for entry in WalkDir::new(&config.static_site_path) {
        let entry = entry.unwrap();
        if entry.file_type().is_dir() {
            continue;
        }
        let path = entry
            .path()
            .strip_prefix(&config.static_site_path)
            .expect("Error stripping prefix");
        if path.components().count() == 0 {
            continue;
        }
        let key = match format!("/{}", path.display()) {
            ref some if some.as_str() == "/index.html" => "/".to_owned(),
            other => other,
        };
        let extension = path
            .extension()
            .and_then(|p| p.to_str())
            .unwrap_or_default();
        println!("{:?}", extension);
        static_bytes.insert(
            key,
            (
                read_file(&entry.path()),
                from_ext(extension).first_or_octet_stream(),
            ),
        );
    }

    let server = Server::new(&config, static_bytes);

    println!("Listening for requests at http://{}", config.listen_address);
    gotham::start(config.listen_address, router(server))
}

fn read_file<P: AsRef<Path>>(path: P) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut file = std::fs::File::open(path).expect("Opening file");
    file.read_to_end(&mut bytes).expect("Reading file");
    bytes
}

struct Server {
    oauth_handler: oauth::OauthHandler,
    tweets: TweetStore,
    static_bytes: HashMap<String, (Vec<u8>, mime::Mime)>,
    domain_name: String,
    cors_origin: String,
    index_url: url::Url,
    oauth_request_url: url::Url,
}

impl<'a> Server {
    pub fn new(config: &Config, static_bytes: HashMap<String, (Vec<u8>, mime::Mime)>) -> Server {
        let app_token = oauth::Oauth1Token {
            oauth_token: config.oauth.app_key.clone(),
            oauth_token_secret: config.oauth.app_secret.clone(),
        };
        let tweets = TweetStore::new(
            app_token.clone(),
            config.search_enabled_display_names.clone(),
        );

        let oauth_handler = oauth::OauthHandler::new(
            url::Url::parse("https://api.twitter.com/oauth/request_token").unwrap(),
            url::Url::parse("https://api.twitter.com/oauth/authenticate").unwrap(),
            url::Url::parse("https://api.twitter.com/1.1/account/verify_credentials.json").unwrap(),
            app_token,
        );

        let domain_name = config.domain_name.clone();
        let cors_origin = format!("https://{}", domain_name);
        let index_url =
            url::Url::parse(&format!("{}/", cors_origin)).expect("Failed to parse index URL");
        let oauth_request_url = url::Url::parse(&format!("https://{}/oauth-request", domain_name))
            .expect("Failed to parse oauth request URL");

        Server {
            oauth_handler,
            tweets,
            static_bytes,
            domain_name,
            cors_origin,
            index_url,
            oauth_request_url,
        }
    }

    pub fn static_page(
        &self,
        state: gotham::state::State,
    ) -> (gotham::state::State, hyper::Response) {
        let res = {
            let path = hyper::Uri::borrow_from(&state).path();
            if path == "/"
                && gotham::middleware::session::SessionData::<Option<oauth::Context>>::borrow_from(
                    &state,
                )
                .is_none()
            {
                let redirect_url = {
                    let uri = hyper::Uri::borrow_from(&state);
                    if uri.is_absolute() {
                        format!("{}", uri)
                    } else {
                        format!("https://{}{}", self.domain_name, uri)
                    }
                };
                let mut dance_url = self.oauth_request_url.clone();
                dance_url
                    .query_pairs_mut()
                    .append_pair("redirect_url", &redirect_url);
                gotham::http::response::create_response(&state, hyper::StatusCode::Found, None)
                    .with_header(hyper::header::Location::new(dance_url.into_string()))
            } else {
                match self.static_bytes.get(path) {
                    Some(&(ref body, ref mime)) => gotham::http::response::create_response(
                        &state,
                        hyper::StatusCode::Ok,
                        Some((body.clone(), mime.clone())),
                    ),
                    None => gotham::http::response::create_response(
                        &state,
                        hyper::StatusCode::NotFound,
                        None,
                    ),
                }
            }
        };
        (state, res)
    }

    pub fn oauth_request(
        &self,
        state: gotham::state::State,
    ) -> (gotham::state::State, hyper::Response) {
        let redirect_url = {
            let query_params: &RedirectUrlQueryParam = RedirectUrlQueryParam::borrow_from(&state);
            let url_result = query_params
                .redirect_url
                .as_ref()
                .map(|s| url::Url::parse(&s));
            match (&query_params.redirect_url, url_result) {
                (_, Some(Ok(url))) => url,
                (_, None) => self.index_url.clone(),
                (&Some(ref redirect_url), Some(Err(err))) => {
                    warn!("Error parsing redirect_url {}: {}", redirect_url, err);
                    self.index_url.clone()
                }
                _ => unreachable!(),
            }
        };
        let response = match self.oauth_handler.dance(redirect_url) {
            Ok(url_to_redirect_to) => {
                gotham::http::response::create_response(&state, hyper::StatusCode::Found, None)
                    .with_header(hyper::header::Location::new(
                        url_to_redirect_to.into_string(),
                    ))
            }
            Err(err) => {
                warn!("Error from oauth dance: {}", err);
                Self::internal_server_error(&state)
            }
        };
        (state, response)
    }

    pub fn oauth_callback(
        &self,
        mut state: gotham::state::State,
    ) -> (gotham::state::State, hyper::Response) {
        let response = {
            let exchange_result = {
                let query_params = OauthCallbackQueryParam::borrow_from(&state);
                self.oauth_handler.exchange(
                    query_params.oauth_token.clone(),
                    query_params.oauth_verifier.clone(),
                )
            };
            match exchange_result {
                Ok((url, context)) => {
                    let response = gotham::http::response::create_response(
                        &state,
                        hyper::StatusCode::Found,
                        None,
                    )
                    .with_header(hyper::header::Location::new(url.into_string()));
                    let session_data: &mut Option<Context> =
                        gotham::middleware::session::SessionData::borrow_mut_from(&mut state);
                    *session_data = Some(context);
                    response
                }
                Err(err) => {
                    warn!("Error in oauth callback: {}", err);
                    Self::internal_server_error(&state)
                }
            }
        };
        (state, response)
    }

    pub fn feed(&self, state: gotham::state::State) -> (gotham::state::State, hyper::Response) {
        let response = {
            let feed_path = FeedPath::borrow_from(&state);
            let maybe_context: &Option<Context> =
                gotham::middleware::session::SessionData::borrow_from(&state);
            let mut response = match maybe_context {
                &Some(ref context) => {
                    let (status_code, contents) = self
                        .feed_impl(feed_path, context)
                        .map(|v| (hyper::StatusCode::Ok, v))
                        .unwrap_or_else(|(status_code, contents)| {
                            (status_code, contents.as_bytes().to_vec())
                        });
                    gotham::http::response::create_response(
                        &state,
                        status_code,
                        Some((contents, mime::APPLICATION_JSON)),
                    )
                }
                &None => {
                    eprintln!("Not authorized");
                    gotham::http::response::create_response(
                        &state,
                        hyper::StatusCode::Unauthorized,
                        Some(("Not authorized".as_bytes().to_vec(), mime::TEXT_PLAIN)),
                    )
                }
            };

            {
                let headers = response.headers_mut();
                headers.set(AccessControlAllowOrigin::Value(self.cors_origin.clone()));
            }
            response
        };

        (state, response)
    }

    fn feed_impl(
        &self,
        feed_path: &FeedPath,
        context: &Context,
    ) -> Result<Vec<u8>, (hyper::StatusCode, String)> {
        let tweets: Vec<_> = self
            .tweets
            .tweets(
                context,
                &feed_path.who,
                &Interval(feed_path.from.into(), feed_path.until.into()),
            )
            .iter()
            .map(|tweet| {
                let seconds_since_unix_epoch: SecondsSinceUnixEpoch = tweet.time().into();
                TweetForJavascript {
                    id: format!("{}", tweet.id),
                    seconds_since_start: seconds_since_unix_epoch.0 - feed_path.from.0,
                }
            })
            .collect();

        let contents = serde_json::to_vec(&tweets).map_err(|err| {
            (
                hyper::StatusCode::InternalServerError,
                format!("Error serializing JSON: {:?}", err),
            )
        })?;
        Ok(contents)
    }

    fn internal_server_error(state: &gotham::state::State) -> hyper::Response {
        gotham::http::response::create_response(
            &state,
            hyper::StatusCode::InternalServerError,
            Some((
                "Internal server error".as_bytes().to_vec(),
                mime::TEXT_PLAIN,
            )),
        )
    }
}

fn router(server: Server) -> gotham::router::Router {
    let server = Arc::new(server);
    let server2 = server.clone();
    let server3 = server.clone();
    let server4 = server.clone();
    let (chain, pipelines) = gotham::pipeline::single::single_pipeline(
        gotham::pipeline::new_pipeline()
            .add(
                gotham::middleware::session::NewSessionMiddleware::default()
                    .with_session_type::<Option<oauth::Context>>(),
            )
            .build(),
    );
    gotham::router::builder::build_router(chain, pipelines, |route| {
        route.get("/healthz").to(healthz);
        for path in server.static_bytes.keys() {
            let server = server.clone();
            route.get(path).to_new_handler(move || {
                let server = server.clone();
                Ok(move |state| server.static_page(state))
            });
        }
        // TODO: Tie these paths statically to Server fields.
        route
            .get("/oauth-request")
            .with_query_string_extractor::<RedirectUrlQueryParam>()
            .to_new_handler(move || {
                let server = server2.clone();
                Ok(move |state| server.oauth_request(state))
            });
        route
            .get("/oauth-callback")
            .with_query_string_extractor::<OauthCallbackQueryParam>()
            .to_new_handler(move || {
                let server = server3.clone();
                Ok(move |state| server.oauth_callback(state))
            });
        route
            .get("/feed/:who/:from/:until")
            .with_path_extractor::<FeedPath>()
            .to_new_handler(move || {
                let server = server4.clone();
                Ok(move |state| server.feed(state))
            });
    })
}

#[derive(Debug, Deserialize, StateData, StaticResponseExtender)]
struct FeedPath {
    who: String,
    from: SecondsSinceUnixEpoch,
    until: SecondsSinceUnixEpoch,
}

#[derive(Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
struct TweetForJavascript {
    id: String,
    seconds_since_start: u64,
}

pub fn healthz(state: gotham::state::State) -> (gotham::state::State, hyper::Response) {
    let res = gotham::http::response::create_response(
        &state,
        hyper::StatusCode::Ok,
        Some(("ok".as_bytes().to_vec(), mime::TEXT_PLAIN)),
    );

    (state, res)
}

#[derive(Deserialize)]
struct Config {
    oauth: OauthConfig,
    listen_address: String,
    domain_name: String,
    static_site_path: String,
    search_enabled_display_names: HashSet<String>,
}

#[derive(Deserialize)]
struct OauthConfig {
    app_key: String,
    app_secret: String,
}

#[derive(Debug, Deserialize, StateData, StaticResponseExtender)]
struct RedirectUrlQueryParam {
    redirect_url: Option<String>,
}

#[derive(Debug, Deserialize, StateData, StaticResponseExtender)]
struct OauthCallbackQueryParam {
    oauth_token: String,
    oauth_verifier: String,
}
