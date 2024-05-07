use std::fs::File;
use std::io::{Error, ErrorKind, SeekFrom, Seek, Read};
use std::path::Path;
use std::rc::Rc;

use log::error;

use crate::{create_m3u_filter_error_result};
use crate::api::api_utils::get_user_server_info;
use crate::m3u_filter_error::{M3uFilterError, M3uFilterErrorKind};
use crate::model::api_proxy::{ProxyType, ProxyUserCredentials};
use crate::model::config::{Config, ConfigTarget};
use crate::model::playlist::{M3uPlaylistItem, PlaylistGroup, PlaylistItemType};
use crate::repository::repository_utils::{IndexedDocumentReader, IndexedDocumentWriter, IndexRecord};
use crate::utils::file_utils;

macro_rules! cant_write_result {
    ($path:expr, $err:expr) => {
        create_m3u_filter_error_result!(M3uFilterErrorKind::Notify, "failed to write m3u playlist: {} - {}", $path.to_str().unwrap() ,$err)
    }
}

pub(crate) fn get_m3u_file_paths(cfg: &Config, filename: &Option<String>) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    match file_utils::get_file_path(&cfg.working_dir, Some(std::path::PathBuf::from(&filename.as_ref().unwrap()))) {
        Some(m3u_path) => {
            let extension = m3u_path.extension().map(|ext| format!("{}_", ext.to_str().unwrap_or(""))).unwrap_or("".to_owned());
            let index_path = m3u_path.with_extension(format!("{}idx", &extension));
            Some((m3u_path, index_path))
        }
        None => None
    }
}

pub(crate) fn get_m3u_epg_file_path(cfg: &Config, filename: &Option<String>) -> Option<std::path::PathBuf> {
    file_utils::get_file_path(&cfg.working_dir, Some(std::path::PathBuf::from(&filename.as_ref().unwrap())))
        .map(|path| file_utils::add_prefix_to_filename(&path, "epg_", Some("xml")))
}

pub(crate) fn write_m3u_playlist(target: &ConfigTarget, cfg: &Config, new_playlist: &[PlaylistGroup], filename: &Option<String>) -> Result<(), M3uFilterError> {
    if !new_playlist.is_empty() {
        if filename.is_none() {
            return Err(M3uFilterError::new(
                M3uFilterErrorKind::Notify,
                format!("write m3u playlist for target {} failed: No filename set", target.name)));
        }

        if let Some((m3u_path, idx_path)) = get_m3u_file_paths(cfg, filename) {
            match IndexedDocumentWriter::new(m3u_path.clone(), idx_path) {
                Ok(mut writer) => {
                    let m3u_playlist = new_playlist.iter()
                        .flat_map(|pg| &pg.channels)
                        .filter(|&pli| pli.header.borrow().item_type != PlaylistItemType::SeriesInfo)
                        .map(|pli| pli.to_m3u()).collect::<Vec<M3uPlaylistItem>>();
                    let mut stream_id: u32 = 1;
                    for mut m3u in m3u_playlist {
                        m3u.stream_id = Rc::new(stream_id.to_string());
                        if let Err(err) = writer.write_doc(&mut stream_id, &m3u) {
                            return cant_write_result!(&m3u_path, err);
                        }
                    }
                }
                Err(err) => return cant_write_result!(&m3u_path, err)
            }
        }
    }
    Ok(())
}

pub(crate) fn load_rewrite_m3u_playlist(cfg: &Config, target: &ConfigTarget, user: &ProxyUserCredentials) -> Option<String> {
    let filename = target.get_m3u_filename();
    if filename.is_some() {
        if let Some((m3u_path, idx_path)) = get_m3u_file_paths(cfg, &filename) {
            match IndexedDocumentReader::<M3uPlaylistItem>::new(&m3u_path, &idx_path) {
                Ok(mut reader) => {
                    let server_info = get_user_server_info(cfg, user);
                    let url = format!("{}/m3u-stream/{}/{}", server_info.get_base_url(), user.username, user.password);
                    let mut result = vec![];
                    result.push("#EXTM3U".to_string());
                    for m3u_pli in reader.by_ref() {
                        match user.proxy {
                            ProxyType::Reverse => {
                                let stream_id = Rc::clone(&m3u_pli.stream_id);
                                result.push(m3u_pli.to_m3u(target, Some(format!("{}/{}", url, stream_id).as_str())));
                            }
                            ProxyType::Redirect => {
                                result.push(m3u_pli.to_m3u(target, None));
                            }
                        }
                    };
                    if reader.by_ref().has_error() {
                        error!("Could not deserialize item {}", &m3u_path.to_str().unwrap());
                    } else {
                        return Some(result.join("\n"));
                    }
                }
                Err(err) => {
                    error!("Could not deserialize file {} - {}", &m3u_path.to_str().unwrap(), err);
                }
            }
        } else {
            error!("Could not open files for target {}", &target.name);
        }
    } else {
        error!("Target has no filename {}", &target.name);
    }
    None
}

pub(crate) fn get_m3u_item_for_stream_id(stream_id: u32, m3u_path: &Path, idx_path: &Path) -> Result<M3uPlaylistItem, Error> {
    if stream_id < 1 {
        return Err(Error::new(ErrorKind::Other, "id should start with 1"));
    }
    if m3u_path.exists() && idx_path.exists() {
        let offset: u64 = IndexRecord::get_index_offset(stream_id - 1) as u64;
        let mut idx_file = File::open(idx_path)?;
        let mut m3u_file = File::open(m3u_path)?;
        let index_record = IndexRecord::from_file(&mut idx_file, offset)?;
        m3u_file.seek(SeekFrom::Start(index_record.index as u64))?;
        let mut buffer: Vec<u8> = vec![0; index_record.size as usize];
        m3u_file.read_exact(&mut buffer)?;
        if let Ok(m3u_pli) = bincode::deserialize::<M3uPlaylistItem>(&buffer[..]) {
            return Ok(m3u_pli);
        }
    }
    Err(Error::new(ErrorKind::Other, format!("Failed to read m3u for stream-id {}", stream_id)))
}