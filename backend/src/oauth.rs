use oauthcli;
use reqwest;
use serde_urlencoded;
use url;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct OauthHandler {
    // token -> (secret, redirect_url)
    // TODO: Expire these after some time
    // TODO: Keep this in the session store
    tokens_awaiting_callbacks: Arc<Mutex<HashMap<String, (String, url::Url)>>>,

    request_token_url: url::Url,
    authentication_url: url::Url,

    app_token: Oauth1Token,
}

impl OauthHandler {
    pub fn new(
        request_token_url: url::Url,
        authentication_url: url::Url,
        app_token: Oauth1Token,
    ) -> OauthHandler {
        let tokens_awaiting_callbacks = Arc::new(Mutex::new(HashMap::new()));
        OauthHandler {
            tokens_awaiting_callbacks,
            request_token_url,
            authentication_url,
            app_token,
        }
    }

    pub fn dance(&self, redirect_url: url::Url) -> Result<url::Url, String> {
        let client = reqwest::Client::new();
        let mut response = client
            .get(self.request_token_url.as_str())
            .header(oauth1_header(
                "GET",
                &self.request_token_url,
                &self.app_token,
                None,
                vec![],
            ))
            .send()
            .map_err(|err| format!("Error requesting token: {:?}", err))?;
        let response_text = response.text().map_err(|err| {
            format!(
                "Error getting text from /oauth/request_token request {:?}",
                err
            )
        })?;
        let v: Oauth1Token = serde_urlencoded::from_str(&response_text).map_err(|err| {
            format!(
                "Error deserializing dance respose ({}): {:?}",
                response_text, err
            )
        })?;

        let mut url = self.authentication_url.clone();
        url.query_pairs_mut()
            .append_pair("oauth_token", &v.oauth_token);

        {
            let mut tokens_awaiting_callbacks = self.tokens_awaiting_callbacks.lock().unwrap();
            tokens_awaiting_callbacks.insert(v.oauth_token, (v.oauth_token_secret, redirect_url));
        }

        Ok(url)
    }

    pub fn exchange(
        &self,
        oauth_token: String,
        oauth_verifier: String,
    ) -> Result<(url::Url, Oauth1Token), String> {
        let client = reqwest::Client::new();
        let url =
            url::Url::parse("https://api.twitter.com/oauth/access_token").expect("Bad twitter URL");
        let params = vec![("oauth_verifier".to_owned(), oauth_verifier)];
        let mut request = client.post(url.clone());
        request.form(&params);
        let oauth_token_secret = {
            let tokens_awaiting_callbacks = self.tokens_awaiting_callbacks.lock().unwrap();
            tokens_awaiting_callbacks
                .get(&oauth_token)
                .expect("TODO")
                .0
                .clone()
        };
        // TODO: Avoid these clones, should just be references everywhere
        request.header(oauth1_header(
            "POST",
            &url,
            &self.app_token,
            Some(&Oauth1Token {
                oauth_token: oauth_token.clone(),
                oauth_token_secret: oauth_token_secret,
            }),
            params,
        ));
        let mut response = request
            .send()
            .map_err(|err| format!("Error making user timeline request to twitter: {:?}", err))?;

        let redirect_url = {
            let mut tokens_awaiting_callbacks = self.tokens_awaiting_callbacks.lock().unwrap();
            tokens_awaiting_callbacks
                .remove(&oauth_token)
                .expect("TODO")
                .1
        };

        let response_text = response
            .text()
            .map_err(|err| format!("Error getting text from user timeline request {:?}", err))?;
        let v: Oauth1Token = serde_urlencoded::from_str(&response_text).map_err(|err| {
            format!(
                "Error deserializing dance respose ({}): {:?}",
                response_text, err
            )
        })?;
        println!("DWH: Got dance response: {:?}", v);

        Ok((redirect_url, v))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Oauth1Token {
    pub oauth_token: String,
    pub oauth_token_secret: String,
}

pub fn oauth1_header(
    method: &str,
    url: &url::Url,
    app_token: &Oauth1Token,
    user_token: Option<&Oauth1Token>,
    params: Vec<(String, String)>,
) -> reqwest::header::Authorization<String> {
    let mut builder = oauthcli::OAuthAuthorizationHeaderBuilder::new(
        method,
        url,
        app_token.oauth_token.as_str(),
        app_token.oauth_token_secret.as_str(),
        oauthcli::SignatureMethod::HmacSha1,
    );
    match user_token {
        Some(token) => {
            builder.token(
                token.oauth_token.as_str(),
                token.oauth_token_secret.as_str(),
            );
        }
        None => {}
    };
    reqwest::header::Authorization(
        builder
            .request_parameters(params.into_iter())
            .finish_for_twitter()
            .to_string(),
    )
}
