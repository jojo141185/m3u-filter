use crate::api::api_utils::{create_fingerprint, try_unwrap_body};
use crate::api::api_utils::{
    force_provider_stream_response, get_stream_alternative_url, is_seek_request,
};
use crate::api::api_utils::{get_headers_from_request, try_option_bad_request, HeaderFilter};
use crate::api::model::UserSession;
use crate::api::model::{create_custom_video_stream_response, CustomVideoStreamType};
use crate::api::model::AppState;
use crate::auth::Fingerprint;
use crate::model::{ConfigTarget, ProxyUserCredentials};
use crate::model::{ConfigInput, InputSource};
use crate::processing::parser::hls::{
    get_hls_session_token_and_url_from_token, rewrite_hls, RewriteHlsProps,
};
use crate::utils::request;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use log::{debug, error};
use serde::Deserialize;
use shared::model::{PlaylistItemType, StreamChannel, TargetType, UserConnectionPermission, XtreamCluster};
use shared::utils::{is_hls_url, replace_url_extension, sanitize_sensitive_info, CUSTOM_VIDEO_PREFIX, HLS_EXT};
use std::sync::Arc;
use crate::repository::m3u_repository::m3u_get_item_for_stream_id;
use crate::repository::xtream_repository;

#[derive(Debug, Deserialize)]
struct HlsApiPathParams {
    username: String,
    password: String,
    input_id: u16,
    stream_id: u32,
    token: String,
}

fn hls_response(hls_content: String) -> impl IntoResponse + Send {
    try_unwrap_body!(axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "application/x-mpegurl")
        .body(hls_content))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::api) async fn handle_hls_stream_request(
    fingerprint: &str,
    addr: &str,
    app_state: &Arc<AppState>,
    user: &ProxyUserCredentials,
    user_session: Option<&UserSession>,
    hls_url: &str,
    virtual_id: u32,
    input: &ConfigInput,
    req_headers: &HeaderMap,
    connection_permission: UserConnectionPermission,
) -> impl IntoResponse + Send {
    let url = replace_url_extension(hls_url, HLS_EXT);
    let server_info = app_state.app_config.get_user_server_info(user);

    let (request_url, session_token) = match user_session {
        Some(session) => {
            match app_state
                .active_provider
                .force_exact_acquire_connection(&session.provider, addr)
                .await
                .get_provider_config()
            {
                Some(provider_cfg) => {
                    let stream_url = get_stream_alternative_url(&url, input, &provider_cfg);
                    (stream_url, Some(session.token.clone()))
                }
                None => (url, None),
            }
        }
        None => {
            match app_state
                .active_provider
                .get_next_provider(&input.name)
                .await
            {
                Some(provider_cfg) => {
                    let stream_url = get_stream_alternative_url(&url, input, &provider_cfg);
                    let user_session_token = create_fingerprint(fingerprint, &user.username, virtual_id);
                    let session_token = app_state.active_users.create_user_session(
                        user,
                        &user_session_token,
                        virtual_id,
                        &provider_cfg.name,
                        &stream_url,
                        addr,
                        connection_permission,
                    ).await;
                    (stream_url, Some(session_token))
                }
                None => (url, None),
            }
        }
    };


    // Don't forward Range on playlist fetch; segments use original headers in provider path
    let filter_header: HeaderFilter = Some(Box::new(|name: &str| !name.eq_ignore_ascii_case("range")));
    let forwarded = get_headers_from_request(req_headers, &filter_header);
    let headers = request::get_request_headers(None, Some(&forwarded));
    let input_source = InputSource::from(input).with_url(request_url);
    match request::download_text_content(
        Arc::clone(&app_state.http_client.load()),
        &input_source,
        Some(&headers),
        None,
    )
        .await
    {
        Ok((content, response_url)) => {
            let rewrite_hls_props = RewriteHlsProps {
                secret: &app_state.app_config.encrypt_secret,
                base_url: &server_info.get_base_url(),
                content: &content,
                hls_url: response_url,
                virtual_id,
                input_id: input.id,
                user_token: session_token.as_deref(),
            };
            let hls_content = rewrite_hls(user, &rewrite_hls_props);
            hls_response(hls_content).into_response()
        }
        Err(err) => {
            error!(
                "Failed to download m3u8 {}",
                sanitize_sensitive_info(err.to_string().as_str())
            );

            let custom_stream_response = app_state.app_config.custom_stream_response.load();
            if custom_stream_response.as_ref().and_then(|c| c.channel_unavailable.as_ref()).is_some() {
                let url = format!(
                    "{}/{CUSTOM_VIDEO_PREFIX}/{}/{}/{}.ts",
                    &server_info.get_base_url(),
                    user.username,
                    user.password,
                    CustomVideoStreamType::ChannelUnavailable);

                let playlist = format!(r"#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:10
#EXT-X-MEDIA-SEQUENCE:0
#EXTINF:10.0,
{url}
#EXT-X-ENDLIST
");
                hls_response(playlist.to_string()).into_response()
            } else {
                axum::http::StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

async fn get_stream_channel(app_state: &Arc<AppState>, target: &Arc<ConfigTarget>, virtual_id: u32) -> Option<StreamChannel> {
    if target.has_output(TargetType::Xtream) {
        if let Ok((pli, _)) = xtream_repository::xtream_get_item_for_stream_id(virtual_id, app_state, target, None).await {
            return Some(pli.to_stream_channel());
        }
    }
     m3u_get_item_for_stream_id(virtual_id, app_state, target).await.ok().map(|pli| pli.to_stream_channel())
}

async fn resolve_stream_channel(
    app_state: &Arc<AppState>,
    target: &Arc<ConfigTarget>,
    virtual_id: u32,
    hls_url: &str,
) -> StreamChannel {
    let mut channel = match get_stream_channel(app_state, target, virtual_id).await {
        Some(channel) => channel,
        None => StreamChannel {
            virtual_id,
            provider_id: 0,
            item_type: PlaylistItemType::LiveHls,
            cluster: XtreamCluster::Live,
            group: "Unknown".to_string(),
            title: "Unknown".to_string(),
            url: hls_url.to_string(),
            shared: false,
        },
    };

    channel.item_type = PlaylistItemType::LiveHls;
    channel
}

#[allow(clippy::too_many_lines)]
async fn hls_api_stream(
    Fingerprint(fingerprint, addr): Fingerprint,
    req_headers: axum::http::HeaderMap,
    axum::extract::Path(params): axum::extract::Path<HlsApiPathParams>,
    axum::extract::State(app_state): axum::extract::State<Arc<AppState>>,
) -> impl axum::response::IntoResponse + Send {
    let (user, target) = try_option_bad_request!(
        app_state
            .app_config
            .get_target_for_user(&params.username, &params.password),
        false,
        format!("Could not find any user for hls stream {}", params.username)
    );
    if user.permission_denied(&app_state) {
        return create_custom_video_stream_response(
            &app_state.app_config,
            CustomVideoStreamType::UserAccountExpired,
        )
            .into_response();
    }

    let target_name = &target.name;
    let virtual_id = params.stream_id;
    let input = try_option_bad_request!(
        app_state.app_config.get_input_by_id(params.input_id),
        true,
        format!(
            "Cant find input for target {target_name}, stream_id {virtual_id}, hls"
        )
    );

    let user_session_token = create_fingerprint(&fingerprint, &user.username, virtual_id);
    let mut user_session = app_state
        .active_users
        .get_and_update_user_session(&user.username, &user_session_token).await;

    if let Some(session) = &mut user_session {
        if session.permission == UserConnectionPermission::Exhausted {
            return create_custom_video_stream_response(
                &app_state.app_config,
                CustomVideoStreamType::UserConnectionsExhausted,
            )
                .into_response();
        }

        if app_state
            .active_provider
            .is_over_limit(&session.provider)
            .await
        {
            return create_custom_video_stream_response(
                &app_state.app_config,
                CustomVideoStreamType::ProviderConnectionsExhausted,
            )
            .into_response();
        }

        let hls_url = match get_hls_session_token_and_url_from_token(
            &app_state.app_config.encrypt_secret,
            &params.token,
        ) {
            Some((Some(session_token), hls_url)) if session.token.eq(&session_token) => hls_url,
            _ => return axum::http::StatusCode::BAD_REQUEST.into_response(),
        };

        session.stream_url.clone_from(&hls_url);
        if session.virtual_id == virtual_id {
            let stream_channel = resolve_stream_channel(&app_state, &target, virtual_id, &hls_url).await;
            if is_seek_request(stream_channel.cluster, &req_headers).await {
                // partial request means we are in reverse proxy mode, seek happened
                return force_provider_stream_response(
                    &addr,
                    &app_state,
                    session,
                    stream_channel,
                    &req_headers,
                    &input,
                    &user,
                )
                .await
                .into_response();
            }
        } else {
            return axum::http::StatusCode::BAD_REQUEST.into_response();
        }

        let connection_permission = user.connection_permission(&app_state).await;
        if connection_permission == UserConnectionPermission::Exhausted {
            return create_custom_video_stream_response(
                &app_state.app_config,
                CustomVideoStreamType::UserConnectionsExhausted,
            )
                .into_response();
        }

        if is_hls_url(&session.stream_url) {
            return handle_hls_stream_request(
                &fingerprint,
                &addr,
                &app_state,
                &user,
                Some(session),
                &session.stream_url,
                virtual_id,
                &input,
                &req_headers,
                connection_permission,
            )
                .await
                .into_response();
        }

        let stream_channel = resolve_stream_channel(&app_state, &target, virtual_id, &hls_url).await;
        force_provider_stream_response(
            &addr,
            &app_state,
            session,
            stream_channel,
            &req_headers,
            &input,
            &user,
        )
            .await
            .into_response()
    } else {
        axum::http::StatusCode::BAD_REQUEST.into_response()
    }
}


pub fn hls_api_register() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route(
        "/hls/{username}/{password}/{input_id}/{stream_id}/{token}",
        axum::routing::get(hls_api_stream),
    )
    //cfg.service(web::resource("/hls/{token}/{stream}").route(web::get().to(xtream_player_api_hls_stream)));
    //cfg.service(web::resource("/play/{token}/{type}").route(web::get().to(xtream_player_api_play_stream)));
}
