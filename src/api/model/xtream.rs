use crate::model::api_proxy::{ApiProxyServerInfo, ProxyUserCredentials};
use serde::Serialize;
use chrono::{Duration, Local};

#[derive(Serialize)]
pub struct XtreamUserInfo {
    pub active_cons: String,
    pub allowed_output_formats: Vec<String>,
    //["ts"],
    pub auth: u16,
    // 0 | 1
    pub created_at: i64,
    //1623429679,
    pub exp_date: i64,
    //1628755200,
    pub is_trial: String,
    // 0 | 1
    pub max_connections: String,
    pub message: String,
    pub password: String,
    pub username: String,
    pub status: String, // "Active"
}

#[derive(Serialize)]
pub struct XtreamServerInfo {
    pub url: String,
    pub port: String,
    pub https_port: String,
    pub server_protocol: String, // http, https
    pub rtmp_port: String,
    pub timezone: String,
    pub timestamp_now: i64,
    pub time_now: String, //"2021-06-28 17:07:37"
}

#[derive(Serialize)]
pub struct XtreamAuthorizationResponse {
    pub user_info: XtreamUserInfo,
    pub server_info: XtreamServerInfo,
}

#[derive(Serialize)]
pub struct XtreamServerInfoDto {
    pub url: String,
    pub port: String,
    pub path: Option<String>,
    pub protocol: String, // http, https
    pub timezone: String,
    pub timestamp_now: i64,
    pub time_now: String, //"2021-06-28 17:07:37"
}

impl XtreamAuthorizationResponse {
    pub fn new(server_info: &ApiProxyServerInfo, user: &ProxyUserCredentials) -> Self {
        let now = Local::now();
        Self {
            user_info: XtreamUserInfo {
                active_cons: "0".to_string(),
                allowed_output_formats: Vec::from(["ts".to_string()]),
                auth: 1,
                created_at: (now - Duration::days(365)).timestamp(), // fake
                exp_date: (now + Duration::days(365)).timestamp(), // fake
                is_trial: "0".to_string(),
                max_connections: "1".to_string(),
                message: server_info.message.to_string(),
                password: user.password.to_string(),
                username: user.username.to_string(),
                status: "Active".to_string(),
            },
            server_info: XtreamServerInfo {
                url: server_info.host.clone(),
                port: if server_info.protocol == "http" { server_info.port.clone() } else { String::from("80") },
                https_port: if server_info.protocol == "https" { server_info.port.clone() } else { String::from("443") },
                server_protocol: server_info.protocol.clone(),
                rtmp_port: String::new(),
                timezone: server_info.timezone.to_string(),
                timestamp_now: now.timestamp(),
                time_now: now.format("%Y-%m-%d %H:%M:%S").to_string(),
            },
        }
    }
}
