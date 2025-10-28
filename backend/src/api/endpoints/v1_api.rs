use crate::api::api_utils::{json_or_bin_response, try_unwrap_body};
use crate::api::endpoints::download_api;
use crate::api::endpoints::user_api::user_api_register;
use crate::api::endpoints::v1_api_playlist::v1_api_playlist_register;
use crate::api::endpoints::v1_api_user::v1_api_user_register;
use crate::api::model::AppState;
use crate::auth::validator_admin;
use crate::utils::ip_checker::get_ips;
use crate::{VERSION};
use axum::response::IntoResponse;
use shared::model::{InputFetchMethod, IpCheckDto, StatusCheck};
use shared::utils::{concat_path_leading_slash};
use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor};
use std::sync::Arc;
use log::error;
use crate::api::endpoints::extract_accept_header::ExtractAcceptHeader;
use crate::api::endpoints::v1_api_config::v1_api_config_register;
use crate::model::InputSource;
use crate::repository::storage::get_geoip_path;
use crate::utils::GeoIp;
use crate::utils::request::download_text_content;

async fn create_ipinfo_check(app_state: &Arc<AppState>) -> Option<(Option<String>, Option<String>)> {
    let config = app_state.app_config.config.load();
    if let Some(ipcheck) = config.ipcheck.as_ref() {
        if let Ok(check) = get_ips(&app_state.http_client.load(), ipcheck).await {
            return Some(check);
        }
    }
    None
}

pub async fn create_status_check(app_state: &Arc<AppState>) -> StatusCheck {
    let cache = match app_state.cache.load().as_ref().as_ref() {
        None => None,
        Some(lock) => {
            Some(lock.lock().await.get_size_text())
        }
    };
    let (active_users, active_user_connections, active_user_streams) = {
        let active_user = &app_state.active_users;
        (active_user.active_users().await, active_user.active_connections().await, active_user.active_streams().await)
    };

    let active_provider_connections = app_state.active_provider.active_connections().await.map(|c| c.into_iter().collect::<BTreeMap<_, _>>());

    StatusCheck {
        status: "ok".to_string(),
        version: VERSION.to_string(),
        build_time: crate::api::api_utils::get_build_time(),
        server_time: crate::api::api_utils::get_server_time(),
        memory: crate::api::api_utils::get_memory_usage(),
        active_users,
        active_user_connections,
        active_provider_connections,
        active_user_streams,
        cache,
    }
}
async fn status(axum::extract::State(app_state): axum::extract::State<Arc<AppState>>) -> axum::response::Response {
    let status = create_status_check(&app_state).await;
    match serde_json::to_string_pretty(&status) {
        Ok(pretty_json) => try_unwrap_body!(axum::response::Response::builder().status(axum::http::StatusCode::OK)
            .header(axum::http::header::CONTENT_TYPE, mime::APPLICATION_JSON.to_string()).body(pretty_json)),
        Err(_) => axum::Json(status).into_response(),
    }
}

async fn streams(ExtractAcceptHeader(accept): ExtractAcceptHeader,
                 axum::extract::State(app_state): axum::extract::State<Arc<AppState>>) -> axum::response::Response {
    let streams = app_state.active_users.active_streams().await;
    json_or_bin_response(accept.as_ref(), &streams).into_response()
}

async fn geoip_update(axum::extract::State(app_state): axum::extract::State<Arc<AppState>>) -> axum::response::Response {
    let config = app_state.app_config.config.load();
    if let Some(geoip) = config.reverse_proxy.as_ref().and_then(|r| r.geoip.as_ref()) {
        if geoip.enabled {
            let geoip_db_path = &*get_geoip_path(&config.working_dir);
            let _file_lock = app_state.app_config.file_locks.write_lock(geoip_db_path);

            let input_source =  InputSource {
                url: geoip.url.clone(),
                username: None,
                password: None,
                method: InputFetchMethod::GET,
                headers: HashMap::default(),
            };
            return match download_text_content(Arc::clone(&app_state.http_client.load()), &input_source, None, None).await {
                   Ok((content, _)) => {
                       let reader = Cursor::new(content);
                       let mut geoip = GeoIp::new();
                       let result = {
                           match geoip.import_ipv4_from_csv(reader, geoip_db_path) {
                           Ok(size) => {
                               (Some(size), None)
                           }
                           Err(err) => (None, Some(err))
                        }
                       };

                       return match result {
                           (Some(_), None) => {
                               app_state.geoip.store(Some(Arc::new(geoip)));
                               axum::http::StatusCode::OK.into_response()
                           },
                           (None, Some(err)) => {
                               error!("Failed to process geoip db: {err}");
                               axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
                           },
                           _ => {
                               axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response()
                           }
                       }
                   }
                   Err(err) => {
                       error!("Failed to download geoip db: {err}");
                       axum::http::StatusCode::BAD_REQUEST.into_response()
                   }
            }
        }
    }
    axum::http::StatusCode::BAD_REQUEST.into_response()
}


async fn ipinfo(axum::extract::State(app_state): axum::extract::State<Arc<AppState>>) -> axum::response::Response {
    if let Some((ipv4, ipv6)) = create_ipinfo_check(&app_state).await {
        let ipcheck = IpCheckDto {
            ipv4,
            ipv6,
        };
        return match serde_json::to_string(&ipcheck) {
            Ok(json) => try_unwrap_body!(axum::response::Response::builder().status(axum::http::StatusCode::OK)
                .header(axum::http::header::CONTENT_TYPE, mime::APPLICATION_JSON.to_string()).body(json)),
            Err(_) => axum::Json(ipcheck).into_response(),
        };
    }
    axum::http::StatusCode::BAD_REQUEST.into_response()
}

pub fn v1_api_register(web_auth_enabled: bool, app_state: Arc<AppState>, web_ui_path: &str) -> axum::Router<Arc<AppState>> {
    let mut router = axum::Router::new();
    router = router
        .route("/status", axum::routing::get(status))
        .route("/streams", axum::routing::get(streams))
        .route("/geoip/update", axum::routing::get(geoip_update))
        .route("/file/download", axum::routing::post(download_api::queue_download_file))
        .route("/file/download/info", axum::routing::get(download_api::download_file_info))
        .route("/ipinfo", axum::routing::get(ipinfo));
    router = v1_api_config_register(router);
    router = v1_api_user_register(router);
    router = v1_api_playlist_register(router);
    if web_auth_enabled {
        router = router.route_layer(axum::middleware::from_fn_with_state(Arc::clone(&app_state), validator_admin));
    }
    let config = app_state.app_config.config.load();

    let mut base_router = axum::Router::new();
    if config.web_ui.as_ref().is_none_or(|c| c.user_ui_enabled) {
        base_router = base_router.merge(user_api_register(app_state));
    }
    base_router.nest(&concat_path_leading_slash(web_ui_path, "api/v1"), router)
}
