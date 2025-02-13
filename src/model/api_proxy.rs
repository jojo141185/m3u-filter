use std::collections::HashSet;
use std::fmt::Display;
use std::str::FromStr;

use enum_iterator::Sequence;
use log::debug;
use crate::m3u_filter_error::{M3uFilterError, M3uFilterErrorKind, create_m3u_filter_error_result, info_err};
use crate::utils::config_reader;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Sequence, PartialEq, Eq)]
pub enum ProxyType {
    #[serde(rename = "reverse")]
    Reverse,
    #[serde(rename = "redirect")]
    Redirect,
}

impl Default for ProxyType {
    fn default() -> Self {
        Self::Redirect
    }
}

impl ProxyType {
    const REVERSE: &'static str = "reverse";
    const REDIRECT: &'static str = "redirect";
}

impl Display for ProxyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Reverse => Self::REVERSE,
                Self::Redirect => Self::REDIRECT,
            }
        )
    }
}

impl FromStr for ProxyType {
    type Err = M3uFilterError;

    fn from_str(s: &str) -> Result<Self, M3uFilterError> {
        if s.eq("reverse") {
            Ok(Self::Reverse)
        } else if s.eq("redirect") {
            Ok(Self::Redirect)
        } else {
            create_m3u_filter_error_result!(M3uFilterErrorKind::Info, "Unknown ProxyType: {}", s)
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyUserCredentials {
    pub username: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default = "ProxyType::default")]
    pub proxy: ProxyType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub epg_timeshift: Option<String>,
}

impl ProxyUserCredentials {
    pub fn prepare(&mut self, resolve_var: bool) {
        if resolve_var {
            self.username = config_reader::resolve_env_var(&self.username);
            self.password = config_reader::resolve_env_var(&self.password);
            if let Some(tkn) = &self.token {
                self.token = Some(config_reader::resolve_env_var(tkn));
            }
            self.trim();
        }
    }

    pub fn matches_token(&self, token: &str) -> bool {
        if let Some(tkn) = &self.token {
            return tkn.eq(token);
        }
        false
    }

    pub fn matches(&self, username: &str, password: &str) -> bool {
        self.username.eq(username) && self.password.eq(password)
    }

    pub fn trim(&mut self) {
        self.username = self.username.trim().to_string();
        self.password = self.password.trim().to_string();
        match &self.token {
            None => {}
            Some(tkn) => {
                self.token = Some(tkn.trim().to_string());
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TargetUser {
    pub target: String,
    pub credentials: Vec<ProxyUserCredentials>,
}

impl TargetUser {
    pub fn get_target_name(
        &self,
        username: &str,
        password: &str,
    ) -> Option<(&ProxyUserCredentials, &str)> {
        self.credentials
            .iter()
            .find(|c| c.matches(username, password))
            .map(|credentials| (credentials, self.target.as_str()))
    }
    pub fn get_target_name_by_token(&self, token: &str) -> Option<(&ProxyUserCredentials, &str)> {
        self.credentials
            .iter()
            .find(|c| c.matches_token(token))
            .map(|credentials| (credentials, self.target.as_str()))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiProxyServerInfo {
    pub name: String,
    pub protocol: String,
    pub host: String,
    pub port: String,
    pub timezone: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl ApiProxyServerInfo {
    pub fn is_valid(&mut self) -> bool {
        self.protocol = self.protocol.trim().to_string();
        if self.protocol.is_empty() {
            return false;
        }
        self.host = self.host.trim().to_string();
        if self.host.is_empty() {
            return false;
        }
        self.port = self.port.trim().to_string();
        if self.port.is_empty() {
            if self.protocol == "http" {
                self.port = "80".to_string();
            } else {
                self.port = "443".to_string();
            }
        } else if self.port.parse::<u16>().is_err() {
            return false;
        }

        self.timezone = self.timezone.trim().to_string();
        if self.timezone.is_empty() {
            self.timezone = "UTC".to_string();
        }
        if self.message.is_empty() {
            self.message = "Welcome to m3u-filter".to_string();
        }

        true
    }

    pub fn get_base_url(&self) -> String {
        let base_url = if self.port.is_empty() {
            format!("{}://{}", self.protocol, self.host)
        } else {
            format!("{}://{}:{}", self.protocol, self.host, self.port)
        };

        match &self.path {
            None => base_url,
            Some(path) => format!("{base_url}/{}", path.trim_matches('/'))
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiProxyConfig {
    pub server: Vec<ApiProxyServerInfo>,
    pub user: Vec<TargetUser>,
}

impl ApiProxyConfig {
    pub fn prepare(&mut self, resolve_var: bool) -> Result<(), M3uFilterError> {
        let mut usernames = HashSet::new();
        let mut tokens = HashSet::new();
        let mut errors = Vec::new();
        if self.server.is_empty() {
            errors.push("No server info defined".to_string());
        } else {
            let mut name_set = HashSet::new();
            for server in &self.server {
                if server.name.trim().is_empty() {
                    errors.push("Server info name is empty ".to_owned());
                } else if name_set.contains(server.name.as_str()) {
                    errors.push(format!(
                        "Non unique server info name found {}",
                        &server.name
                    ));
                } else {
                    name_set.insert(server.name.clone());
                }
            }
        }
        for target_user in &mut self.user {
            for user in &mut target_user.credentials {
                user.prepare(resolve_var);
                if usernames.contains(&user.username) {
                    errors.push(format!("Non unique username found {}", &user.username));
                } else {
                    usernames.insert(user.username.to_string());
                }
                if let Some(token) = &user.token {
                    if token.is_empty() {
                        user.token = None;
                    } else if tokens.contains(token) {
                        errors.push(format!("Non unique token found {}", &user.username));
                    } else {
                        tokens.insert(token.to_string());
                    }
                }

                if let Some(server_info_name) = &user.server {
                    if !&self
                        .server
                        .iter()
                        .any(|server_info| server_info.name.eq(server_info_name))
                    {
                        errors.push(format!(
                            "No server info with name {} found for user {}",
                            server_info_name, &user.username
                        ));
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(info_err!(errors.join("\n")))
        }
    }

    pub fn get_target_name(
        &self,
        username: &str,
        password: &str,
    ) -> Option<(ProxyUserCredentials, String)> {
        for target_user in &self.user {
            if let Some((credentials, target_name)) =
                target_user.get_target_name(username, password)
            {
                return Some((credentials.clone(), target_name.to_string()));
            };
        }
        debug!("Could not find any target for user {username}");
        None
    }

    pub fn get_target_name_by_token(&self, token: &str) -> Option<(ProxyUserCredentials, String)> {
        for target_user in &self.user {
            if let Some((credentials, target_name)) = target_user.get_target_name_by_token(token) {
                return Some((credentials.clone(), target_name.to_string()));
            };
        }
        None
    }

    pub fn get_user_credentials(&self,username: &str) -> Option<ProxyUserCredentials> {
        let result = self.user.iter()
            .flat_map(|target_user| &target_user.credentials)
            .find(|credential| credential.username == username)
            .cloned();
        if result.is_none() {
            debug!("Could not find any user {username}");
        }
        result
    }
}
