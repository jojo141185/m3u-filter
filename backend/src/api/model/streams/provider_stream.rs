use crate::api::api_utils::{HeaderFilter};
use crate::api::model::{CustomVideoStream, ThrottledStream};
use crate::model::{AppConfig};
use shared::model::PlaylistItemType;
use log::{trace};
use reqwest::StatusCode;
use axum::response::IntoResponse;
use crate::api::model::stream::ProviderStreamResponse;
use crate::api::model::TransportStreamBuffer;
use crate::api::api_utils::try_unwrap_body;
use std::str::FromStr;
use std::fmt;
use serde::{Serialize, Deserialize, Serializer, Deserializer};

#[derive(Debug, Copy, Clone)]
pub enum CustomVideoStreamType {
    ChannelUnavailable,
    UserConnectionsExhausted,
    ProviderConnectionsExhausted,
    UserAccountExpired
}

impl fmt::Display for CustomVideoStreamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CustomVideoStreamType::ChannelUnavailable => "channel_unavailable",
            CustomVideoStreamType::UserConnectionsExhausted => "user_connections_exhausted",
            CustomVideoStreamType::ProviderConnectionsExhausted => "provider_connections_exhausted",
            CustomVideoStreamType::UserAccountExpired => "user_account_expired",
        };
        write!(f, "{s}")
    }
}

impl FromStr for CustomVideoStreamType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "channel_unavailable" => Ok(Self::ChannelUnavailable),
            "user_connections_exhausted" => Ok(Self::UserConnectionsExhausted),
            "provider_connections_exhausted" => Ok(Self::ProviderConnectionsExhausted),
            "user_account_expired" => Ok(Self::UserAccountExpired),
            _ => Err(format!("Unknown stream type: {s}")),
        }
    }
}

impl Serialize for CustomVideoStreamType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
impl<'de> Deserialize<'de> for CustomVideoStreamType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

fn create_video_stream(video_buffer: Option<&TransportStreamBuffer>, headers: &[(String, String)], log_message: &str) -> ProviderStreamResponse {
    if let Some(video) = video_buffer {
        trace!("{log_message}");
        let mut response_headers: Vec<(String, String)> = headers.iter()
            .filter(|(key, _)| !(key.eq("content-type") || key.eq("content-length") || key.contains("range")))
            .map(|(key, value)| (key.clone(), value.clone())).collect();
        response_headers.push(("content-type".to_string(), "video/mp2t".to_string()));
        (Some(Box::pin(ThrottledStream::new(CustomVideoStream::new(video.clone()), 8000))), Some((response_headers, StatusCode::OK, None)))
    } else {
        (None, None)
    }
}

pub fn create_channel_unavailable_stream(cfg: &AppConfig, headers: &[(String, String)], status: StatusCode) -> ProviderStreamResponse {
    let custom_stream_response = cfg.custom_stream_response.load();
    let video = custom_stream_response.as_ref().and_then(|c| c.channel_unavailable.as_ref());
    create_video_stream(video, headers, &format!("Streaming response channel unavailable for status {status}"))
}

pub fn create_user_connections_exhausted_stream(cfg: &AppConfig, headers: &[(String, String)]) -> ProviderStreamResponse {
    let custom_stream_response = cfg.custom_stream_response.load();
    let video = custom_stream_response.as_ref().and_then(|c| c.user_connections_exhausted.as_ref());
    create_video_stream(video, headers, "Streaming response user connections exhausted")
}

pub fn create_provider_connections_exhausted_stream(cfg: &AppConfig, headers: &[(String, String)]) -> ProviderStreamResponse {
    let custom_stream_response = cfg.custom_stream_response.load();
    let video = custom_stream_response.as_ref().and_then(|c| c.provider_connections_exhausted.as_ref());
    create_video_stream(video, headers, "Streaming response provider connections exhausted")
}

pub fn create_user_account_expired_stream(cfg: &AppConfig, headers: &[(String, String)]) -> ProviderStreamResponse {
    let custom_stream_response = cfg.custom_stream_response.load();
    let video = custom_stream_response.as_ref().and_then(|c| c.user_account_expired.as_ref());
    create_video_stream(video, headers, "Streaming response user account expired")
}

pub fn create_custom_video_stream_response(config: &AppConfig, video_response: CustomVideoStreamType) -> impl axum::response::IntoResponse + Send {
    if let (Some(stream), Some((headers, status_code, _))) = match video_response {
        CustomVideoStreamType::ChannelUnavailable => create_channel_unavailable_stream(config, &[], StatusCode::BAD_REQUEST),
        CustomVideoStreamType::UserConnectionsExhausted => create_user_connections_exhausted_stream(config, &[]),
        CustomVideoStreamType::ProviderConnectionsExhausted => create_provider_connections_exhausted_stream(config, &[]),
        CustomVideoStreamType::UserAccountExpired => create_user_account_expired_stream(config, &[]),
    } {
        let mut builder = axum::response::Response::builder()
            .status(status_code);
        for (key, value) in headers {
            builder = builder.header(key, value);
        }
        return try_unwrap_body!(builder.body(axum::body::Body::from_stream(stream)));
    }
    axum::http::StatusCode::FORBIDDEN.into_response()
}
pub fn get_header_filter_for_item_type(item_type: PlaylistItemType) -> HeaderFilter {
    match item_type {
        PlaylistItemType::Live /*| PlaylistItemType::LiveHls | PlaylistItemType::LiveDash */| PlaylistItemType::LiveUnknown => {
            Some(Box::new(|key| key != "accept-ranges" && key != "range" && key != "content-range"))
        }
        _ => None,
    }
}
