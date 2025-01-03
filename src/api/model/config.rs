use serde::{Deserialize, Serialize};

use crate::model::api_proxy::ApiProxyConfig;
use crate::model::config::ProcessingOrder;
use crate::model::config::{ConfigApi, ConfigRename, ConfigSort, ConfigTargetOptions, InputType, MessagingConfig, TargetOutput, VideoConfig};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ServerInputConfig {
    pub id: u16,
    pub input_type: InputType,
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub persist: Option<String>,
    pub name: Option<String>,
    pub enabled: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ServerTargetConfig {
    pub id: u16,
    pub enabled: bool,
    pub name: String,
    pub options: Option<ConfigTargetOptions>,
    pub sort: Option<ConfigSort>,
    pub filter: String,
    #[serde(alias = "type")]
    pub output: Vec<TargetOutput>,
    pub rename: Option<Vec<ConfigRename>>,
    pub mapping: Option<Vec<String>>,
    pub processing_order: ProcessingOrder,
    pub watch: Option<Vec<String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ServerSourceConfig {
    pub inputs: Vec<ServerInputConfig>,
    pub targets: Vec<ServerTargetConfig>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct ServerConfig {
    pub api: ConfigApi,
    pub threads: u8,
    pub working_dir: String,
    pub backup_dir: Option<String>,
    pub schedule: Option<String>,
    pub sources: Vec<ServerSourceConfig>,
    pub messaging: Option<MessagingConfig>,
    pub video: Option<VideoConfig>,
    pub api_proxy: Option<ApiProxyConfig>,
}

