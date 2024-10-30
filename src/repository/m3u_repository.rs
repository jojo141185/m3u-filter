use std::fs::File;
use std::io::{BufWriter, Error, ErrorKind, Write};
use std::path::{Path, PathBuf};

use log::error;

use crate::api::api_utils::get_user_server_info;
use crate::create_m3u_filter_error;
use crate::m3u_filter_error::{M3uFilterError, M3uFilterErrorKind};
use crate::model::api_proxy::{ProxyType, ProxyUserCredentials};
use crate::model::config::{Config, ConfigTarget};
use crate::model::playlist::{M3uPlaylistItem, PlaylistGroup, PlaylistItem, PlaylistItemType};
use crate::repository::indexed_document_reader::IndexedDocumentReader;
use crate::repository::indexed_document_writer::IndexedDocumentWriter;
use crate::repository::storage::ensure_target_storage_path;
use crate::utils::file_utils;

macro_rules! cant_write_result {
    ($path:expr, $err:expr) => {
        create_m3u_filter_error!(M3uFilterErrorKind::Notify, "failed to write m3u playlist: {} - {}", $path.to_str().unwrap() ,$err)
    }
}

fn m3u_get_base_file_path(target_path: &Path) -> PathBuf {
    target_path.join(PathBuf::from("m3u.db"))
}

pub(crate) fn m3u_get_file_paths(target_path: &Path) -> (PathBuf, PathBuf) {
    let m3u_path = m3u_get_base_file_path(target_path);
    let extension = m3u_path.extension().map(|ext| format!("{}_", ext.to_str().unwrap_or("")));
    let index_path = m3u_path.with_extension(format!("{}idx", &extension.unwrap_or_default()));
    (m3u_path, index_path)
}

pub(crate) fn m3u_get_epg_file_path(target_path: &Path) -> PathBuf {
    let path = m3u_get_base_file_path(target_path);
    file_utils::add_prefix_to_filename(&path, "epg_", Some("xml"))
}

fn persist_m3u_playlist_as_text(target: &ConfigTarget, cfg: &Config, m3u_playlist: &Vec<M3uPlaylistItem>) {
    if let Some(filename) = target.get_m3u_filename() {
        if let Some(m3u_filename) = file_utils::get_file_path(&cfg.working_dir, Some(PathBuf::from(filename))) {
            match File::create(&m3u_filename) {
                Ok(file) => {
                    let mut buf_writer = BufWriter::new(file);
                    let _ = buf_writer.write("#EXTM3U\n".as_bytes());
                    for m3u in m3u_playlist {
                        let _ = buf_writer.write(m3u.to_m3u(target, None).as_bytes());
                        let _ = buf_writer.write("\n".as_bytes());
                    }
                }
                Err(_) => {
                    error!("Can't write m3u plain playlist {}", &m3u_filename.to_str().unwrap());
                }
            }
        }
    }
}

pub(crate) fn m3u_write_playlist(target: &ConfigTarget, cfg: &Config, target_path: &Path, new_playlist: &[PlaylistGroup]) -> Result<(), M3uFilterError> {
    if !new_playlist.is_empty() {
        let (m3u_path, idx_path) = m3u_get_file_paths(target_path);
        let m3u_playlist = new_playlist.iter()
            .flat_map(|pg| &pg.channels)
            .filter(|&pli| pli.header.borrow().item_type != PlaylistItemType::SeriesInfo)
            .map(PlaylistItem::to_m3u).collect::<Vec<M3uPlaylistItem>>();

        persist_m3u_playlist_as_text(target, cfg, &m3u_playlist);
        {
            let _file_lock = cfg.file_locks.write_lock(&m3u_path).map_err(|err| M3uFilterError::new(M3uFilterErrorKind::Info, format!("{err}")))?;
            match IndexedDocumentWriter::new(m3u_path.clone(), idx_path) {
                Ok(mut writer) => {
                    for m3u in m3u_playlist {
                        match writer.write_doc(m3u.virtual_id, &m3u) {
                            Ok(()) => {}
                            Err(err) => return Err(cant_write_result!(&m3u_path, err))
                        }
                    }
                    writer.flush().map_err(|err| cant_write_result!(&m3u_path, err))?;
                }
                Err(err) => return Err(cant_write_result!(&m3u_path, err))
            }
        }
    }
    Ok(())
}

pub(crate) fn m3u_load_rewrite_playlist(cfg: &Config, target: &ConfigTarget, user: &ProxyUserCredentials) -> Option<String> {
    match ensure_target_storage_path(cfg, target.name.as_str()) {
        Ok(target_path) => {
            let (m3u_path, _) = m3u_get_file_paths(&target_path);
            {
                let _file_lock = cfg.file_locks.read_lock(&m3u_path).map_err(|err| {
                    error!("Could not lock document {:?}: {}", m3u_path, err);
                    Error::new(ErrorKind::Other, format!("Document Reader error for target {}", &target.name))
                }).ok()?;

                match IndexedDocumentReader::<M3uPlaylistItem>::new(&m3u_path) {
                    Ok(mut reader) => {
                        let server_info = get_user_server_info(cfg, user);
                        let url = format!("{}/m3u-stream/{}/{}", server_info.get_base_url(), user.username, user.password);
                        let mut result = vec![];
                        result.push("#EXTM3U".to_string());
                        for m3u_pli in reader.by_ref() {
                            match user.proxy {
                                ProxyType::Reverse => {
                                    result.push(m3u_pli.to_m3u(target, Some(format!("{url}/{}", m3u_pli.virtual_id).as_str())));
                                }
                                ProxyType::Redirect => {
                                    result.push(m3u_pli.to_m3u(target, None));
                                }
                            }
                        };
                        if reader.by_ref().has_error() {
                            error!("Could not deserialize m3u item {}", &m3u_path.to_str().unwrap());
                        } else {
                            return Some(result.join("\n"));
                        }
                    }
                    Err(err) => {
                        error!("Could not deserialize file {} - {}", &m3u_path.to_str().unwrap(), err);
                    }
                }
            }
        }
        Err(err) => {
            error!("Could not find storage path for target  {} - {}", target.name.as_str(), err);
        }
    }
    None
}

pub(crate) fn m3u_get_item_for_stream_id(cfg: &Config, stream_id: u32, m3u_path: &Path, idx_path: &Path) -> Result<M3uPlaylistItem, Error> {
    if stream_id < 1 {
        return Err(Error::new(ErrorKind::Other, "id should start with 1"));
    }
    {
        let _file_lock = cfg.file_locks.read_lock(m3u_path)?;
        IndexedDocumentReader::<M3uPlaylistItem>::read_indexed_item(m3u_path, idx_path, stream_id)
    }
}