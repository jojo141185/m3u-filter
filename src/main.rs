#![allow(clippy::module_name_repetitions)]
extern crate env_logger;
extern crate pest;
#[macro_use]
extern crate pest_derive;
extern crate core;

use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use actix_rt::System;

use clap::Parser;
use env_logger::Builder;
use log::{error, info, LevelFilter};
use crate::auth::password::generate_password;

use crate::model::config::{Config, HealthcheckConfig, ProcessTargets, validate_targets};
use crate::model::healthcheck::Healthcheck;
use crate::processing::playlist_processor;
use crate::utils::{config_reader, file_utils};
mod m3u_filter_error;
mod model;
mod filter;
mod repository;
mod messaging;
mod api;
mod processing;
mod utils;
mod auth;

#[derive(Parser)]
#[command(name = "m3u-filter")]
#[command(author = "euzu <euzu@github.com>")]
#[command(version)]
#[command(about = "Extended M3U playlist filter", long_about = None)]
struct Args {
    /// The config directory
    #[arg(short = 'p', long = "config-path")]
    config_path: Option<String>,

    /// The config file
    #[arg(short = 'c', long = "config")]
    config_file: Option<String>,

    /// The source config file
    #[arg(short = 'i', long = "source")]
    source_file: Option<String>,

    /// The mapping file
    #[arg(short = 'm', long = "mapping")]
    mapping_file: Option<String>,

    /// The target to process
    #[arg(short = 't', long)]
    target: Option<Vec<String>>,

    /// The user file
    #[arg(short = 'a', long = "api-proxy")]
    api_proxy: Option<String>,

    /// Run in server mode
    #[arg(short = 's', long, default_value_t = false, default_missing_value = "true")]
    server: bool,

    /// log level
    #[arg(short = 'l', long = "log-level", default_missing_value = "info")]
    log_level: Option<String>,

    #[arg(short = None, long = "genpwd", default_value_t = false, default_missing_value = "true")]
    genpwd: bool,

    #[arg(short = None, long = "healthcheck", default_value_t = false, default_missing_value = "true")]
    healthcheck: bool,

}

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args = Args::parse();
    let default_log_level = std::env::var("M3U_FILTER_LOG").unwrap_or_else(|_| "info".to_string());
    init_logger(args.log_level.as_ref().unwrap_or(&default_log_level));

    let config_path: String = args.config_path.unwrap_or_else(file_utils::get_default_config_path);
    let config_file: String = args.config_file.unwrap_or_else(|| file_utils::get_default_config_file_path(&config_path));

    if args.healthcheck {
        healthcheck(config_file.as_str());
    }

    let sources_file: String = args.source_file.unwrap_or_else(|| file_utils::get_default_sources_file_path(&config_path));
    let mut cfg = config_reader::read_config(config_path.as_str(), config_file.as_str(), sources_file.as_str()).unwrap_or_else(|err| exit!("{}", err));

    if args.genpwd  {
        match generate_password() {
            Ok(pwd) => println!("{pwd}"),
            Err(err) => error!("{err}")
        }
        return;
    }

    create_directories(&cfg);

    let targets = validate_targets(args.target.as_ref(), &cfg.sources).unwrap_or_else(|err| exit!("{}", err));

    info!("Version: {}", VERSION);
    info!("Current time: {}", chrono::offset::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    info!("Working dir: {:?}", &cfg.working_dir);
    info!("Config dir: {:?}", &cfg.t_config_path);
    info!("Config file: {}", &config_file);
    info!("Source file: {}", &sources_file);

    if let Err(err) = config_reader::read_mappings(args.mapping_file, &mut cfg) {
        exit!("{}", err);
    }

    if args.server {
        config_reader::read_api_proxy_config(args.api_proxy, &mut cfg);
        start_in_server_mode(Arc::new(cfg), Arc::new(targets));
    } else {
        start_in_cli_mode(Arc::new(cfg), Arc::new(targets));
    }
}

fn create_directories(cfg: &Config) {
    // Collect the paths into a vector.
    let paths = [
        Some(cfg.working_dir.clone()),
        cfg.backup_dir.clone(),
        cfg.video.as_ref().and_then(|v| v.download.as_ref()).and_then(|d| d.directory.clone())
    ];

    // Iterate over the paths, filter out `None` values, and process the `Some(path)` values.
    paths.iter()
        .filter_map(|opt| opt.as_ref()) // Get rid of the `Option`
        .for_each(|dir| {
            let path = PathBuf::from(dir);
            if !path.exists() {
                // Create the directory tree if it doesn't exist
                let path_value = path.to_str().unwrap_or("?");
                if let Err(e) = std::fs::create_dir_all(&path) {
                    error!("Failed to create directory {path_value}: {e}");
                } else {
                    info!("Created directory: {path_value}");
                }
            }
        });
}

fn start_in_cli_mode(cfg: Arc<Config>, targets: Arc<ProcessTargets>) {
    System::new().block_on(async { playlist_processor::exec_processing(cfg, targets).await });
}

fn start_in_server_mode(cfg: Arc<Config>, targets: Arc<ProcessTargets>) {
    info!("Server running: http://{}:{}", &cfg.api.host, &cfg.api.port);
    if let Err(err) = api::main_api::start_server(cfg, targets) {
        exit!("Can't start server: {err}");
    };
}

fn get_log_level(log_level: &str) -> LevelFilter {
    match log_level.to_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        // "info" => LevelFilter::Info,
        _ => LevelFilter::Info,
    }
}

fn init_logger(log_level: &str) {
    let mut log_builder = Builder::from_default_env();

    if log_level.contains('=') {
        for pair in log_level.split(',').filter(|s| s.contains('=')) {
            let mut kv_iter = pair.split('=').map(str::trim);
            if let (Some(module), Some(level)) = (kv_iter.next(), kv_iter.next()) {
                log_builder.filter_module(module, get_log_level(level));
            }
        }
    } else {
        // Set the log level based on the parsed value
        log_builder.filter_level(get_log_level(log_level));
    }
    log_builder.filter_module("actix_web::middleware::logger", LevelFilter::Error);
    log_builder.filter_module("reqwest::async_impl::client", LevelFilter::Error);
    log_builder.filter_module("reqwest::connect", LevelFilter::Error);
    log_builder.filter_module("hyper_util::client", LevelFilter::Error);
    log_builder.init();
    info!("Log Level {}", get_log_level(log_level));
}

fn healthcheck(config_file: &str) {
    let path = std::path::PathBuf::from(config_file);
    let file = File::open(path).expect("Failed to open config file");
    let config: HealthcheckConfig = serde_yaml::from_reader(file).expect("Failed to parse config file");

    if let Ok(response) = reqwest::blocking::get(format!("http://localhost:{}/healthcheck", config.api.port)) {
        if let Ok(check) = response.json::<Healthcheck>() {
            if check.status == "ok" {
                std::process::exit(0);
            }
        }
    }

    std::process::exit(1);
}