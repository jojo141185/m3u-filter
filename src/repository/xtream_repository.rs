use std::collections::HashMap;
use std::convert::TryInto;
use std::fs::File;
use std::io::{BufReader, Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use log::error;
use serde_json::{json, Value};

use crate::create_m3u_filter_error_result;
use crate::m3u_filter_error::{M3uFilterError, M3uFilterErrorKind};
use crate::model::config::{Config, ConfigTarget};
use crate::model::playlist::{PlaylistGroup, PlaylistItem, PlaylistItemType, XtreamCluster, XtreamPlaylistItem};
use crate::model::xtream::XtreamMappingOptions;
use crate::repository::indexed_document_reader::{IndexedDocumentReader, read_indexed_item};
use crate::repository::indexed_document_writer::IndexedDocumentWriter;
use crate::repository::index_record::IndexRecord;
use crate::utils::file_utils;
use crate::utils::json_utils::{json_write_documents_to_file, json_iter_array};

pub(crate) static COL_CAT_LIVE: &str = "cat_live";
pub(crate) static COL_CAT_SERIES: &str = "cat_series";
pub(crate) static COL_CAT_VOD: &str = "cat_vod";
pub(crate) static COL_LIVE: &str = "live";
pub(crate) static COL_SERIES: &str = "series";
pub(crate) static COL_VOD: &str = "vod";

macro_rules! cant_write_result {
    ($path:expr, $err:expr) => {
        create_m3u_filter_error_result!(M3uFilterErrorKind::Notify, "failed to write xtream playlist: {} - {}", $path.to_str().unwrap() ,$err)
    }
}

fn get_collection_path(path: &Path, collection: &str) -> PathBuf {
    path.join(format!("{}.json", collection))
}

pub(crate) fn xtream_get_stream_id_cluster_index_file_path(storage_path: &Path) -> PathBuf {
    storage_path.join("stream_id_cluster_index.db")
}

// Maps episode id to provider_episode_id
fn xtream_get_series_episode_mapping_file_path(storage_path: &Path) -> PathBuf {
    storage_path.join("mapping_series_episode_id.db_idx")
}

// maps series_id to the index of series_info index
// direct access is not possible because the series_id is not ascending, it is random
fn xtream_get_series_id_series_info_mapping_file_path(storage_path: &Path) -> PathBuf {
    storage_path.join("mapping_stream_id_series_info.db_idx")
}

fn ensure_xtream_storage_path(cfg: &Config, target_name: &str) -> Result<PathBuf, M3uFilterError> {
    if let Some(path) = xtream_get_storage_path(cfg, target_name) {
        if std::fs::create_dir_all(&path).is_err() {
            let msg = format!("Failed to save xtream data, can't create directory {}", &path.to_str().unwrap());
            return Err(M3uFilterError::new(M3uFilterErrorKind::Notify, msg));
        }
        Ok(path)
    } else {
        let msg = format!("Failed to save xtream data, can't create directory for target {target_name}");
        Err(M3uFilterError::new(M3uFilterErrorKind::Notify, msg))
    }
}

fn xtream_get_info_file_paths(storage_path: &Path, cluster: &XtreamCluster) -> Option<(PathBuf, PathBuf)> {
    if cluster == &XtreamCluster::Series {
        let xtream_path = storage_path.join("series_info.db");
        let extension = xtream_path.extension().map(|ext| format!("{}_", ext.to_str().unwrap_or(""))).unwrap_or("".to_owned());
        let index_path = xtream_path.with_extension(format!("{}idx", &extension));
        return Some((xtream_path, index_path));
    }
    None
}

fn xtream_clear_series_info(storage_path: &Path) {
    if let Some((info_path, idx_path)) = xtream_get_info_file_paths(storage_path, &XtreamCluster::Series) {
        let _ = std::fs::remove_file(info_path);
        let _ = std::fs::remove_file(idx_path);
        let _ = std::fs::remove_file(xtream_get_series_episode_mapping_file_path(storage_path));
        let _ = std::fs::remove_file(xtream_get_series_id_series_info_mapping_file_path(storage_path));
    }
}

fn write_playlist_to_file(storage_path: &Path, stream_id: &mut u32, cluster: &XtreamCluster, playlist: &mut [PlaylistItem]) -> Result<(), M3uFilterError> {
    let (xtream_path, idx_path) = xtream_get_file_paths(storage_path, cluster);
    match IndexedDocumentWriter::new(xtream_path.clone(), idx_path) {
        Ok(mut writer) => {
            for pli in playlist.iter_mut() {
                let mut xtream = pli.to_xtream();
                xtream.stream_id = *stream_id;
                match writer.write_doc(&xtream) {
                    Ok(_) => *stream_id += 1,
                    Err(err) => return cant_write_result!(&xtream_path, err)
                }
            }
            if cluster == &XtreamCluster::Series {
                xtream_clear_series_info(storage_path);
            }
            Ok(())
        }
        Err(err) => cant_write_result!(&xtream_path, err)
    }
}

fn save_stream_id_cluster_mapping(storage_path: &Path, id_data: &mut Vec<(XtreamCluster, u32)>) -> Result<(), Error> {
    let stream_id_path = xtream_get_stream_id_cluster_index_file_path(storage_path);
    id_data.sort_by(|(_, a), (_, b)| b.cmp(a));
    let encoded: Vec<u8> = bincode::serialize(id_data).unwrap();
    std::fs::write(stream_id_path, encoded)
}

fn load_stream_id_cluster_mapping(storage_path: &Path) -> Option<Vec<(XtreamCluster, u32)>> {
    let path = xtream_get_stream_id_cluster_index_file_path(storage_path);
    if path.exists() {
        match std::fs::read(path) {
            Ok(encoded) => {
                let decoded: Vec<(XtreamCluster, u32)> = bincode::deserialize(&encoded[..]).unwrap();
                Some(decoded)
            }
            Err(_) => None,
        }
    } else {
        None
    }
}

fn write_playlists_to_file(storage_path: &Path, collections: Vec<(XtreamCluster, &mut [PlaylistItem])>) -> Result<(), M3uFilterError> {
    let mut id_list: Vec<(XtreamCluster, u32)> = vec![];
    let mut stream_id: u32 = 1;
    for (cluster, playlist) in collections {
        id_list.push((cluster.clone(), stream_id));
        write_playlist_to_file(storage_path, &mut stream_id, &cluster, playlist)?;
    }
    match save_stream_id_cluster_mapping(storage_path, &mut id_list) {
        Ok(_) => Ok(()),
        Err(err) => Err(M3uFilterError::new(M3uFilterErrorKind::Notify, format!("failed to write xtream playlist: {} - {}", storage_path.to_str().unwrap(), err)))
    }
}


fn get_map_item_as_str(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    if let Some(value) = map.get(key) {
        if let Some(result) = value.as_str() {
            return Some(result.to_string());
        }
    }
    None
}

fn load_old_category_ids(path: &Path) -> (u32, HashMap<String, u32>) {
    let mut result: HashMap<String, u32> = HashMap::new();
    let mut max_id: u32 = 0;
    for col_path in [
        get_collection_path(path, COL_CAT_LIVE),
        get_collection_path(path, COL_CAT_VOD),
        get_collection_path(path, COL_CAT_SERIES)] {
        if col_path.exists() {
            if let Ok(file) = File::open(col_path) {
                let reader = BufReader::new(file);
                for entry in json_iter_array::<Value, BufReader<File>>(reader).flatten() {
                    if let Some(item) = entry.as_object() {
                        if let Some(category_id) = get_map_item_as_str(item, "category_id") {
                            if let Some(category_name) = get_map_item_as_str(item, "category_name") {
                                if let Ok(cat_id) = category_id.parse::<u32>() {
                                    result.insert(category_name, cat_id);
                                    max_id = max_id.max(cat_id);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    (max_id, result)
}

pub(crate) fn xtream_get_storage_path(cfg: &Config, target_name: &str) -> Option<PathBuf> {
    file_utils::get_file_path(&cfg.working_dir, Some(std::path::PathBuf::from(target_name.replace(' ', "_"))))
}

pub(crate) fn xtream_get_epg_file_path(path: &Path) -> PathBuf {
    path.join("epg.xml")
}

pub(crate) fn xtream_get_file_paths(storage_path: &Path, cluster: &XtreamCluster) -> (PathBuf, PathBuf) {
    let xtream_path = storage_path.join(format!("{}.db", match cluster {
        XtreamCluster::Live => COL_LIVE,
        XtreamCluster::Video => COL_VOD,
        XtreamCluster::Series => COL_SERIES
    }));
    let extension = xtream_path.extension().map(|ext| format!("{}_", ext.to_str().unwrap_or(""))).unwrap_or("".to_owned());
    let index_path = xtream_path.with_extension(format!("{}idx", &extension));
    (xtream_path, index_path)
}

pub(crate) fn xtream_write_playlist(target: &ConfigTarget, cfg: &Config, playlist: &mut [PlaylistGroup]) -> Result<(), M3uFilterError> {
    match ensure_xtream_storage_path(cfg, target.name.as_str()) {
        Ok(path) => {
            let mut cat_live_col = vec![];
            let mut cat_series_col = vec![];
            let mut cat_vod_col = vec![];
            let mut live_col = vec![];
            let mut series_col = vec![];
            let mut vod_col = vec![];
            let mut errors = Vec::new();

            // preserve category_ids
            let (max_cat_id, existing_cat_ids) = load_old_category_ids(&path);
            let mut cat_id_counter = max_cat_id;
            for plg in playlist.iter_mut() {
                if !&plg.channels.is_empty() {
                    let cat_id = existing_cat_ids.get(plg.title.as_ref()).unwrap_or_else(|| {
                        cat_id_counter += 1;
                        &cat_id_counter
                    });
                    plg.id = *cat_id;

                    match &plg.xtream_cluster {
                        XtreamCluster::Live => &mut cat_live_col,
                        XtreamCluster::Series => &mut cat_series_col,
                        XtreamCluster::Video => &mut cat_vod_col,
                    }.push(json!({
                      "category_id": format!("{}", &cat_id),
                      "category_name": plg.title.clone(),
                      "parent_id": 0
                    }));

                    for pli in plg.channels.drain(..) {
                        let mut header = pli.header.borrow_mut();
                        // we skip resolved series, because this is only necessary when writing m3u files
                        let col = if header.item_type != PlaylistItemType::Series {
                            if header.id.parse::<i32>().is_ok() {
                                header.category_id = *cat_id;
                                Some(match header.xtream_cluster {
                                    XtreamCluster::Live => &mut live_col,
                                    XtreamCluster::Series => &mut series_col,
                                    XtreamCluster::Video => &mut vod_col,
                                })
                            } else {
                                errors.push(format!("Channel does not have an id: {}", pli.header.borrow().title.as_str()));
                                None
                            }
                        } else { None };
                        drop(header);
                        if let Some(pl) = col {
                            pl.push(pli);
                        }
                    }
                }
            }

            for (col_path, data) in [
                (get_collection_path(&path, COL_CAT_LIVE), &cat_live_col),
                (get_collection_path(&path, COL_CAT_VOD), &cat_vod_col),
                (get_collection_path(&path, COL_CAT_SERIES), &cat_series_col)] {
                match json_write_documents_to_file(&col_path, data) {
                    Ok(()) => {}
                    Err(err) => {
                        errors.push(format!("Persisting collection failed: {}: {}", &col_path.to_str().unwrap(), err));
                    }
                }
            }

            match write_playlists_to_file(&path, vec![
                (XtreamCluster::Live, &mut live_col),
                (XtreamCluster::Video, &mut vod_col),
                (XtreamCluster::Series, &mut series_col)]) {
                Ok(()) => {}
                Err(err) => {
                    errors.push(format!("Persisting collection failed:{}", err));
                }
            }

            if !errors.is_empty() {
                return create_m3u_filter_error_result!(M3uFilterErrorKind::Notify, "{}", errors.join("\n"));
            }
        }
        Err(err) => return Err(err)
    }
    Ok(())
}

pub(crate) fn xtream_get_collection_path(cfg: &Config, target_name: &str, collection_name: &str) -> Result<(Option<PathBuf>, Option<String>), Error> {
    if let Some(path) = xtream_get_storage_path(cfg, target_name) {
        let col_path = get_collection_path(&path, collection_name);
        if col_path.exists() {
            return Ok((Some(col_path), None));
        }
    }
    Err(Error::new(ErrorKind::Other, format!("Cant find collection: {}/{}", target_name, collection_name)))
}

pub(crate) fn xtream_get_item_for_stream_id(stream_id: u32, config: &Config, target: &ConfigTarget, xtream_cluster: Option<&XtreamCluster>) -> Result<XtreamPlaylistItem, Error> {
    if let Some(storage_path) = xtream_get_storage_path(config, target.name.as_str()) {
        if let Some(mapping) = load_stream_id_cluster_mapping(&storage_path) {
            if let Some((cluster, cluster_index)) = match xtream_cluster {
                Some(clus) => mapping.iter().find(|(c, _)| c == clus),
                None => mapping.iter().find(|(_, c)| stream_id >= *c),
            } {
                let (xtream_path, idx_path) = xtream_get_file_paths(&storage_path, cluster);
                if stream_id >= *cluster_index {
                    return read_indexed_item::<XtreamPlaylistItem>(&xtream_path, &idx_path, IndexRecord::get_index_offset(stream_id - cluster_index));
                }
            }
        }
    }
    Err(Error::new(ErrorKind::Other, format!("Failed to read xtream item for stream-id {}", stream_id)))
}

pub(crate) fn xtream_load_rewrite_playlist(cluster: &XtreamCluster, config: &Config, target: &ConfigTarget, category_id: u32) -> Result<String, Error> {
    if let Some(storage_path) = xtream_get_storage_path(config, target.name.as_str()) {
        let (xtream_path, idx_path) = xtream_get_file_paths(&storage_path, cluster);
        match IndexedDocumentReader::<XtreamPlaylistItem>::new(&xtream_path, &idx_path) {
            Ok(mut reader) => {
                let options = XtreamMappingOptions::from_target_options(target.options.as_ref());
                let result: Vec<Value> = reader.by_ref().filter(|pli| category_id == 0 || pli.category_id == category_id)
                    .map(|pli| pli.to_doc(&options)).collect();
                if reader.by_ref().has_error() {
                    error!("Could not deserialize item {}", &xtream_path.to_str().unwrap());
                } else {
                    return Ok(serde_json::to_string(&result).unwrap());
                }
            }
            Err(err) => {
                error!("Could not deserialize file {} - {}", &xtream_path.to_str().unwrap(), err);
            }
        }
    }
    Err(Error::new(ErrorKind::Other, format!("Failed to find xtream storage for target {}", &target.name)))
}

fn read_last_id_from_series_info_episode_id_mapping_file(path: &Path) -> Result<u32, Error> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-8))?;
    let mut buffer = [0; 4];
    file.read_exact(&mut buffer)?;
    Ok(u32::from_le_bytes(buffer))
}

pub(crate) fn xtream_get_max_series_info_episode_id(config: &Config, target_name: &str) -> Option<u32> {
    if let Some(storage_path) = xtream_get_storage_path(config, target_name) {
        let mapping_file_path = xtream_get_series_episode_mapping_file_path(&storage_path);
        if mapping_file_path.exists() {
            if let Ok(last_id) = read_last_id_from_series_info_episode_id_mapping_file(&mapping_file_path) {
                return Some(last_id);
            }
        }

        if let Some(storage_path) = xtream_get_storage_path(config, target_name) {
            if let Some(mapping) = load_stream_id_cluster_mapping(&storage_path) {
                return mapping.iter().map(|(_, sid)| sid).max().copied();
            }
        }
    }
    None
}

fn xtream_write_series_info_id_mapping(storage_path: &Path, episode_id_mapping: &[(u32, u32)]) -> Result<(), Error> {
    let file_path = xtream_get_series_episode_mapping_file_path(storage_path);
    if let Ok(mut file) = if file_path.exists() {
        std::fs::OpenOptions::new()
            .append(true) // Open in append mode
            .open(file_path)
    } else {
        File::create(file_path)
    } {
        let mut bytes: Vec<u8> = Vec::new();
        episode_id_mapping.iter().for_each(|(episode_id, provider_id)| {
            bytes.extend_from_slice(&episode_id.to_le_bytes());
            bytes.extend_from_slice(&provider_id.to_le_bytes());
        });
        return file_utils::check_write(file.write_all(&bytes[..]));
    }
    Err(Error::new(ErrorKind::Other, format!("Failed to open series info idmapping file inside {}", storage_path.to_str().unwrap())))
}

pub(crate) fn xtream_write_series_info(config: &Config, target_name: &str,
                                       series_id: u32, episode_id_mapping: &[(u32, u32)],
                                       content: &str) -> Result<(), Error> {
    if let Some(storage_path) = xtream_get_storage_path(config, target_name) {
        if let Some((info_path, idx_path)) = xtream_get_info_file_paths(&storage_path, &XtreamCluster::Series) {
            return match IndexedDocumentWriter::new_append(info_path.clone(), idx_path) {
                Ok(mut writer) => {
                    match writer.write_doc(content) {
                        Ok((_, index_offset)) => {
                            let series_id_index_mapping_path = xtream_get_series_id_series_info_mapping_file_path(&storage_path);
                            IndexRecord::to_file(&series_id_index_mapping_path, series_id, index_offset, true)?;
                        }
                        Err(_) => return Err(Error::new(ErrorKind::Other, format!("failed to write xtream series info for target {}", target_name)))
                    }
                    xtream_write_series_info_id_mapping(&storage_path, episode_id_mapping)
                }
                Err(err) => Err(err)
            };
        }
    }
    Ok(())
}

pub(crate) fn xtream_load_series_info(config: &Config, target_name: &str, series_id: u32) -> Result<String, Error> {
    if let Some(storage_path) = xtream_get_storage_path(config, target_name) {
        let series_id_index_mapping_path = xtream_get_series_id_series_info_mapping_file_path(&storage_path);
        if series_id_index_mapping_path.exists() {
            if let Some(storage_path) = xtream_get_storage_path(config, target_name) {
                if let Some((info_path, idx_path)) = xtream_get_info_file_paths(&storage_path, &XtreamCluster::Series) {
                    if info_path.exists() && idx_path.exists() {
                        let mut file = File::open(series_id_index_mapping_path)?;
                        let mut buffer = [0u8; 8];
                        loop {
                            match file.read_exact(&mut buffer) {
                                Ok(_) => {
                                    let stream_id = u32::from_le_bytes(buffer[..4].try_into().unwrap());
                                    if stream_id == series_id {
                                        let index = u32::from_le_bytes(buffer[4..].try_into().unwrap());
                                        return read_indexed_item::<String>(&info_path, &idx_path, index);
                                    }
                                }
                                Err(e) => return Err(e),
                            }
                        }
                    }
                }
            }
        }
    }
    Err(Error::new(ErrorKind::Other, format!("Failed to read series info for id {} for {}", series_id, target_name)))
}