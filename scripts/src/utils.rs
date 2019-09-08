#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use futures::future::{try_join, try_join_all};
use isahc::{prelude::Request, RequestExt, ResponseExt};
use serde::{Deserialize, Serialize};
use serde_json::to_vec;
use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use url::Url;

use crate::consts::VTUBERS;
use crate::types::{
    bilibili::{StatResponse, UpstatResponse},
    youtube::{Channel, ChannelsResponse, Video, VideosResponse},
    Error, Result, Values,
};

pub async fn youtube_videos(id: String) -> Result<Vec<Video>> {
    isahc::get_async(
        Url::parse_with_params(
            "https://www.googleapis.com/youtube/v3/videos",
            &[
                ("part", "id,liveStreamingDetails"),
                ("maxResults", "50"),
                ("key", env!("YOUTUBE_API_KEY0")),
                ("id", &id),
            ],
        )?
        .as_str(),
    )
    .await?
    .json::<VideosResponse>()
    .map(|res| res.items)
    .map_err(Error::Json)
}

pub async fn youtube_videos_snippet(id: String) -> Result<Vec<Video>> {
    isahc::get_async(
        Url::parse_with_params(
            "https://www.googleapis.com/youtube/v3/videos",
            &[
                ("part", "id,liveStreamingDetails,snippet"),
                ("maxResults", "50"),
                ("key", env!("YOUTUBE_API_KEY0")),
                ("id", &id),
            ],
        )?
        .as_str(),
    )
    .await?
    .json::<VideosResponse>()
    .map(|res| res.items)
    .map_err(Error::Json)
}

pub async fn youtube_channels(id: String) -> Result<Vec<Channel>> {
    isahc::get_async(
        Url::parse_with_params(
            "https://www.googleapis.com/youtube/v3/channels",
            &[
                ("part", "statistics"),
                ("key", env!("YOUTUBE_API_KEY1")),
                ("id", &id),
            ],
        )?
        .as_str(),
    )
    .await?
    .json::<ChannelsResponse>()
    .map(|res| res.items)
    .map_err(Error::Json)
}

pub async fn bilibili_stat(id: usize) -> Result<(i32, i32)> {
    let (mut stat, mut upstat) = try_join(
        isahc::get_async(
            Url::parse_with_params(
                "https://api.bilibili.com/x/relation/stat",
                &[("vmid", id.to_string())],
            )?
            .as_str(),
        ),
        isahc::get_async(
            Url::parse_with_params(
                "https://api.bilibili.com/x/space/upstat",
                &[("mid", id.to_string())],
            )?
            .as_str(),
        ),
    )
    .await?;

    Ok((
        stat.json::<StatResponse>()?.data.follower,
        upstat.json::<UpstatResponse>()?.data.archive.view,
    ))
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SignInRequest {
    email: &'static str,
    password: &'static str,
    return_secure_token: bool,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct SignInResponse {
    id_token: String,
    refresh_token: String,
    expires_in: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
struct RefreshResponse {
    id_token: String,
    refresh_token: String,
    expires_in: String,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
struct RefreshRequest<'a> {
    grant_type: &'static str,
    refresh_token: &'a str,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Database {
    id_token: String,
    refresh_token: String,
    expires_at: DateTime<Utc>,
}

impl Default for Database {
    fn default() -> Self {
        Database {
            id_token: String::new(),
            refresh_token: String::new(),
            expires_at: Utc::now(),
        }
    }
}

impl Database {
    pub async fn new() -> Result<Database> {
        if let Ok(json) = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/.token.json")) {
            let mut db = serde_json::from_str::<Database>(&json)?;

            if (db.expires_at - Utc::now()).num_seconds() < 5 {
                db.refresh_token().await?;
            }

            Ok(db)
        } else {
            let mut db = Database::default();

            db.sign_in().await?;

            Ok(db)
        }
    }

    async fn sign_in(&mut self) -> Result<()> {
        let res = Request::post(
            Url::parse_with_params(
                "https://identitytoolkit.googleapis.com/v1/accounts:signInWithPassword",
                &[("key", env!("FIREBASE_WEB_API_KEY"))],
            )?
            .as_str(),
        )
        .header("content-type", "application/json")
        .body(to_vec(&SignInRequest {
            email: env!("FIREBASE_USER_EMAIL"),
            password: env!("FIREBASE_USER_PASSWORD"),
            return_secure_token: true,
        })?)?
        .send_async()
        .await?
        .json::<SignInResponse>()?;

        println!("Signed in successfully.");

        self.expires_at = Utc::now() + Duration::seconds(i64::from_str(&res.expires_in)?);
        self.id_token = res.id_token;
        self.refresh_token = res.refresh_token;

        fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/.token.json"),
            to_vec(&self)?,
        )?;

        Ok(())
    }

    async fn refresh_token(&mut self) -> Result<()> {
        let res = Request::post(
            Url::parse_with_params(
                "https://securetoken.googleapis.com/v1/token",
                &[("key", env!("FIREBASE_WEB_API_KEY"))],
            )?
            .as_str(),
        )
        .header("content-type", "application/json")
        .body(to_vec(&RefreshRequest {
            grant_type: "refresh_token",
            refresh_token: &self.refresh_token,
        })?)?
        .send_async()
        .await?
        .json::<RefreshResponse>()?;

        self.expires_at = Utc::now() + Duration::seconds(i64::from_str(&res.expires_in)?);
        self.id_token = res.id_token;
        self.refresh_token = res.refresh_token;

        println!("Token refreshed successfully.");

        fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/.token.json"),
            to_vec(&self)?,
        )?;

        Ok(())
    }

    pub async fn patch_values(&mut self, values: Values) -> Result<()> {
        let res = Request::patch(
            Url::parse_with_params(
                concat!(
                    "https://",
                    env!("FIREBASE_PROJECT_ID"),
                    ".firebaseio.com/.json"
                ),
                &[("format", "slient"), ("auth", &self.id_token)],
            )?
            .as_str(),
        )
        .body(to_vec(&values)?)?
        .send_async()
        .await?;
        if res.status().is_success() {
            println!("Updated Successfully.")
        } else {
            eprintln!("Failed to update.")
        }
        Ok(())
    }

    pub async fn vtuber_stats_timestamps(&mut self) -> Result<Vec<i64>> {
        let timestamps = isahc::get_async(
            Url::parse_with_params(
                concat!(
                    "https://",
                    env!("FIREBASE_PROJECT_ID"),
                    ".firebaseio.com/vtuberStats/_timestamps.json"
                ),
                &[("auth", &self.id_token)],
            )?
            .as_str(),
        )
        .await?
        .json::<HashMap<i64, bool>>()?;

        let mut timestamps: Vec<_> = timestamps.into_iter().map(|(k, _)| k).collect();

        timestamps.sort();

        Ok(timestamps)
    }

    pub async fn stream_stats<'a>(
        &mut self,
        id: Vec<&'a str>,
    ) -> Result<Vec<(&'a str, usize, usize)>> {
        let vec = try_join_all(id.iter().map(|id| {
            isahc::get_async(
                Url::parse_with_params(
                    &format!(
                        "https://{}.firebaseio.com/streamStats/{}.json",
                        env!("FIREBASE_PROJECT_ID"),
                        id
                    ),
                    &[("auth", &self.id_token)],
                )
                .unwrap()
                .as_str(),
            )
        }))
        .await?;

        let mut results = vec![];

        for (mut res, id) in vec.into_iter().zip(id.into_iter()) {
            if let Ok(map) = res.json::<HashMap<String, usize>>() {
                let array = map.values().copied().collect::<Vec<_>>();

                results.push((
                    id,
                    array.iter().sum::<usize>() / array.len(),
                    *(array.iter().max().unwrap()),
                ));
            }
        }

        Ok(results)
    }

    pub async fn current_streams(&mut self) -> Result<HashMap<String, bool>> {
        isahc::get_async(
            Url::parse_with_params(
                concat!(
                    "https://",
                    env!("FIREBASE_PROJECT_ID"),
                    ".firebaseio.com/streams/_current.json",
                ),
                &[("auth", &self.id_token)],
            )?
            .as_str(),
        )
        .await?
        .json::<HashMap<String, bool>>()
        .map_err(Error::Json)
    }

    pub async fn vtuber_stats_at(&mut self, timestamp: i64) -> Result<Vec<[i32; 4]>> {
        let res = try_join_all(VTUBERS.iter().map(|v| {
            isahc::get_async(
                Url::parse_with_params(
                    &format!(
                        "https://{}.firebaseio.com/vtuberStats/{}/{}.json",
                        env!("FIREBASE_PROJECT_ID"),
                        v.name,
                        timestamp,
                    ),
                    &[("auth", &self.id_token)],
                )
                .unwrap()
                .as_str(),
            )
        }))
        .await?;

        res.into_iter()
            .map(|mut res| res.json::<[i32; 4]>().map_err(Error::Json))
            .collect()
    }
}
