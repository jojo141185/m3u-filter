use crate::m3u_filter_error::info_err;
use crate::m3u_filter_error::{M3uFilterError, M3uFilterErrorKind};
use crate::model::api_proxy::{ProxyType, ProxyUserCredentials};
use crate::model::config::{Config, ConfigTarget, ConfigTargetOptions};
use crate::model::playlist::{M3uPlaylistItem, PlaylistItemType};
use crate::repository::indexed_document::IndexedDocumentIterator;
use crate::repository::m3u_repository::m3u_get_file_paths;
use crate::repository::storage::ensure_target_storage_path;
use crate::utils::file::file_lock_manager::FileReadGuard;

pub const M3U_STREAM_PATH: &str = "m3u-stream";
pub const M3U_RESOURCE_PATH: &str = "resource/m3u";

#[allow(clippy::struct_excessive_bools)]
pub struct M3uPlaylistIterator {
    reader: IndexedDocumentIterator<u32, M3uPlaylistItem>,
    base_url: String,
    username: String,
    password: String,
    target_options: Option<ConfigTargetOptions>,
    mask_redirect_url: bool,
    include_type_in_url: bool,
    started: bool,
    rewrite_resource: bool,
    proxy_type: ProxyType,
    _file_lock: FileReadGuard,
}

impl M3uPlaylistIterator {
    pub fn new(
        cfg: &Config,
        target: &ConfigTarget,
        user: &ProxyUserCredentials,
    ) -> Result<Self, M3uFilterError> {
        let target_path = ensure_target_storage_path(cfg, target.name.as_str())?;
        let (m3u_path, idx_path) = m3u_get_file_paths(&target_path);

        let file_lock = cfg.file_locks.read_lock(&m3u_path);

        let reader =
            IndexedDocumentIterator::<u32, M3uPlaylistItem>::new(&m3u_path, &idx_path)
                .map_err(|err| info_err!(format!("Could not deserialize file {m3u_path:?} - {err}")))?;

        let target_options = target.options.as_ref();
        let include_type_in_url = target_options.is_some_and(|opts| opts.m3u_include_type_in_url);
        let mask_redirect_url = target_options.is_some_and(|opts| opts.m3u_mask_redirect_url);

        let server_info = cfg.get_user_server_info(user);
        Ok(Self {
            reader,
            base_url: server_info.get_base_url(),
            username: user.username.to_string(),
            password: user.password.to_string(),
            target_options: target.options.clone(),
            include_type_in_url,
            mask_redirect_url,
            proxy_type: user.proxy.clone(),
            _file_lock: file_lock, // Save lock inside struct
            started: false,
            rewrite_resource: cfg.is_reverse_proxy_resource_rewrite_enabled(),
        })
    }

    fn get_rewritten_url(&self, m3u_pli: &M3uPlaylistItem, typed: bool, prefix_path: &str) -> String {
        if typed {
            let stream_type = match m3u_pli.item_type {
                PlaylistItemType::Live
                | PlaylistItemType::Catchup
                | PlaylistItemType::LiveUnknown
                | PlaylistItemType::LiveHls => "live",
                PlaylistItemType::Video => "movie",
                PlaylistItemType::Series
                | PlaylistItemType::SeriesInfo => "series",
            };
            format!("{}/{prefix_path}/{stream_type}/{}/{}/{}",
                    &self.base_url,
                    &self.username,
                    &self.password,
                    m3u_pli.virtual_id
            )
        } else {
            format!("{}/{prefix_path}/{}/{}/{}",
                    &self.base_url, &self.username, &self.password, m3u_pli.virtual_id
            )
        }
    }

    fn get_stream_url(&self, m3u_pli: &M3uPlaylistItem, typed: bool) -> String {
        self.get_rewritten_url(m3u_pli, typed, M3U_STREAM_PATH)
    }
    fn get_resource_url(&self, m3u_pli: &M3uPlaylistItem) -> String {
        self.get_rewritten_url(m3u_pli, false, M3U_RESOURCE_PATH)
    }

}

impl Iterator for M3uPlaylistIterator {
    type Item = String;

    fn next(&mut self) -> Option<Self::Item> {
        if !self.started {
            self.started = true;
            return Some("#EXTM3U".to_string());
        }

        // TODO hls and unknown reverse proxy
        self.reader.next().map(|m3u_pli| {
            let rewrite_urls = match m3u_pli.item_type {
                PlaylistItemType::LiveHls => None,
                _ => if match &self.proxy_type {
                    ProxyType::Reverse => true,
                    ProxyType::Redirect => self.mask_redirect_url,
                } {
                    Some((self.get_stream_url(&m3u_pli, self.include_type_in_url), if self.rewrite_resource { Some(self.get_resource_url(&m3u_pli)) } else { None }))
                } else {
                    None
                }
            };
            let target_options = self.target_options.as_ref();
            m3u_pli.to_m3u(target_options, rewrite_urls.as_ref())
        })
    }
}
