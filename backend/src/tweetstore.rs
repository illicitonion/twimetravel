use oauth;
use reqwest;
use serde_json;
use std;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use time;
use url;
use {Context, Interval, IntervalSet, IntervalStore, UniquelyIdentifiedTimeValue};

pub const TWEPOCH_MILLIS: u64 = 1288834974657;

#[derive(Copy, Clone, Debug, Deserialize, Eq, Ord, PartialOrd, PartialEq)]
pub struct SecondsSinceUnixEpoch(pub u64);

impl std::fmt::Display for SecondsSinceUnixEpoch {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Eq, Ord, PartialOrd, PartialEq)]
pub struct Snowflake(pub u64);

impl std::fmt::Display for Snowflake {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<SecondsSinceUnixEpoch> for Snowflake {
    fn from(epoch: SecondsSinceUnixEpoch) -> Snowflake {
        Snowflake((epoch.0 * 1000 - TWEPOCH_MILLIS) << 22)
    }
}

impl From<Snowflake> for SecondsSinceUnixEpoch {
    fn from(epoch: Snowflake) -> SecondsSinceUnixEpoch {
        SecondsSinceUnixEpoch(((epoch.0 >> 22) + TWEPOCH_MILLIS) / 1000)
    }
}

#[derive(Clone, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
pub struct TweetFromTwitter {
    pub id: Snowflake,
}

impl UniquelyIdentifiedTimeValue<Snowflake> for TweetFromTwitter {
    fn time(&self) -> Snowflake {
        self.id
    }
}

#[derive(Clone)]
pub struct TweetStore {
    app_token: oauth::Oauth1Token,
    search_enabled_display_names: HashSet<String>,
    tweets: Arc<RwLock<HashMap<String, Arc<RwLock<IntervalStore<Snowflake, TweetFromTwitter>>>>>>,
}

impl TweetStore {
    pub fn new(
        app_oauth_token: oauth::Oauth1Token,
        search_enabled_display_names: HashSet<String>,
    ) -> TweetStore {
        TweetStore {
            app_token: app_oauth_token,
            search_enabled_display_names: search_enabled_display_names,
            tweets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    // TODO: Accept a list of users
    pub fn tweets(
        &self,
        context: &Context,
        user: &String,
        interval: &Interval<Snowflake>,
    ) -> Vec<TweetFromTwitter> {
        match self.get_known_tweets(user, interval) {
            Ok(tweets) => tweets,
            Err(missing_intervals) => {
                self.fetch_all_tweets(context, user, &missing_intervals)
                    .expect("Fetching tweets");
                self.tweets(context, user, interval)
            }
        }
    }

    fn fetch_all_tweets(
        &self,
        context: &Context,
        user: &String,
        intervals: &IntervalSet<Snowflake>,
    ) -> Result<(), String> {
        for interval in intervals.iter() {
            self.fetch_tweets(context, user, interval)?;
        }
        Ok(())
    }

    fn fetch_tweets(
        &self,
        context: &Context,
        user: &String,
        interval: &Interval<Snowflake>,
    ) -> Result<(), String> {
        let tweets = match self.fetch_usertimeline(context, user, interval)? {
            Some(tweets) => tweets,
            None => {
                if self
                    .search_enabled_display_names
                    .contains(&context.user_screen_name)
                {
                    self.fetch_user_tweets_from_search(context, user, interval)?
                } else {
                    return Err(format!(
                        "No tweets found, but can't guarantee no tweets should have been found"
                    ));
                }
            }
        };

        let interval_store_lock = self.interval_store(user);
        let mut interval_store = interval_store_lock.write().unwrap();
        interval_store.insert(interval, tweets)
    }

    fn fetch_usertimeline(
        &self,
        context: &Context,
        user: &String,
        interval: &Interval<Snowflake>,
    ) -> Result<Option<Vec<TweetFromTwitter>>, String> {
        println!("Fetching from user timeline"); // TODO: Binary log requests and responses.

        let json_string = {
            let client = reqwest::blocking::Client::new();
            let url = "https://api.twitter.com/1.1/statuses/user_timeline.json";
            let params = vec![
                ("screen_name".to_owned(), user.to_owned()),
                ("since_id".to_owned(), format!("{}", interval.0)),
                ("max_id".to_owned(), format!("{}", &interval.1)),
            ];
            let request = client.get(url).query(&params).header(
                reqwest::header::AUTHORIZATION,
                oauth::oauth1_header(
                    "GET",
                    &url::Url::parse(url).expect("Bad twitter URL"),
                    &self.app_token,
                    Some(&context.user_oauth_token),
                    params,
                ),
            );
            let response = request.send().map_err(|err| {
                format!("Error making user timeline request to twitter: {:?}", err)
            })?;
            response
                .text()
                .map_err(|err| format!("Error getting text from user timeline request {:?}", err))?
        };

        println!("DWH: Response: {}", json_string);

        let mut tweets: Vec<TweetFromTwitter> = serde_json::from_str(&json_string)
            .map_err(|err| format!("Error parsing JSON from Twitter: {:?}", err))?;
        tweets.sort();

        if tweets.len() == 0 {
            // It would be great if we had a better heuristic than
            // "no tweets means we hit the 3200 tweet limit".
            return Ok(None);
        }

        Ok(Some(tweets))
    }

    fn fetch_user_tweets_from_search(
        &self,
        context: &Context,
        user: &String,
        interval: &Interval<Snowflake>,
    ) -> Result<Vec<TweetFromTwitter>, String> {
        println!("Fetching from search API"); // TODO: Binary log requests and responses.
        let json_string = {
            let client = reqwest::blocking::Client::new();
            // TODO: Choose which API to use based on interval
            let url = format!(
                "https://api.twitter.com/1.1/tweets/search/{}",
                "30day/dev.json"
            );
            let params: HashMap<&str, String> = vec![
                ("query", format!("from:{}", user)),
                ("fromDate", TweetStore::as_twitter_time(&interval.0.into())),
                ("toDate", TweetStore::as_twitter_time(&interval.1.into())),
            ]
            .into_iter()
            .collect();
            let response = client
                .post(url.as_str())
                .json(&params)
                .header(
                    reqwest::header::AUTHORIZATION,
                    oauth::oauth1_header(
                        "POST",
                        &url::Url::parse(&url).expect("Bad twitter URL"),
                        &self.app_token,
                        Some(&context.user_oauth_token),
                        vec![],
                    ),
                )
                .send()
                .map_err(|err| format!("Error making search request to twitter: {:?}", err))?;
            response
                .text()
                .map_err(|err| format!("Error getting text from search request {:?}", err))?
        };

        let mut tweets: Vec<_> = {
            let response: ResponseFromTwitter = serde_json::from_str(&json_string)
                .map_err(|err| format!("Error parsing JSON from Twitter: {:?}", err))?;
            response.results
        };
        tweets.sort();
        Ok(tweets)
    }

    fn as_twitter_time(s: &SecondsSinceUnixEpoch) -> String {
        let tm = time::strptime(&format!("{}", s), "%s").expect("Parsing tm from snowflake");
        format!(
            "{}",
            tm.strftime("%Y%m%d%H%M")
                .expect("Formatting tm as twitter time")
        )
    }

    fn get_known_tweets(
        &self,
        user: &String,
        interval: &Interval<Snowflake>,
    ) -> Result<Vec<TweetFromTwitter>, IntervalSet<Snowflake>> {
        let interval_store_lock = self.interval_store(user);
        let interval_store = interval_store_lock.read().unwrap();
        match interval_store.get(interval) {
            Some(tweets) => Ok(tweets),
            None => Err(interval_store.missing(interval)),
        }
    }

    fn interval_store(
        &self,
        user: &String,
    ) -> Arc<RwLock<IntervalStore<Snowflake, TweetFromTwitter>>> {
        {
            let user_map = self.tweets.read().unwrap();
            match user_map.get(user) {
                Some(user_bucket) => return user_bucket.clone(),
                None => {}
            }
        }
        {
            let mut user_map = self.tweets.write().unwrap();
            if !user_map.contains_key(user) {
                user_map.insert(user.clone(), Arc::new(RwLock::new(IntervalStore::new())));
            }
            user_map.get(user).unwrap().clone()
        }
    }

    pub fn preload(&self) {
        let mut interval_store = IntervalStore::new();
        interval_store
            .insert(
                &Interval(Snowflake(963140650398646272), Snowflake(963155749893046272)),
                vec![
                    TweetFromTwitter {
                        id: Snowflake(963143061558743040),
                    },
                    TweetFromTwitter {
                        id: Snowflake(963143736631869440),
                    },
                    TweetFromTwitter {
                        id: Snowflake(963144473604534272),
                    },
                    TweetFromTwitter {
                        id: Snowflake(963146750457499648),
                    },
                    TweetFromTwitter {
                        id: Snowflake(963152907255377921),
                    },
                ],
            )
            .expect("Inserting tweets");
        let mut user_map = self.tweets.write().unwrap();
        user_map.insert(
            "harrisimo".to_owned(),
            Arc::new(RwLock::new(interval_store)),
        );
    }
}

#[derive(Deserialize)]
struct ResponseFromTwitter {
    pub results: Vec<TweetFromTwitter>,
}
