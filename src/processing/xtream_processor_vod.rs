use crate::m3u_filter_error::{M3uFilterError, M3uFilterErrorKind};
use crate::model::config::{Config, ConfigTarget, InputType};
use crate::model::playlist::{FetchedPlaylist, PlaylistItem, PlaylistItemType, XtreamCluster};
use crate::processing::xtream_processor::{create_resolve_info_wal_files, playlist_resolve_download_playlist_item, read_processed_info_ids, should_update_info, write_info_content_to_wal_file};
use crate::repository::xtream_repository::{xtream_update_input_info_file, xtream_update_input_vod_record_from_wal_file, InputVodInfoRecord};
use crate::{create_resolve_options_function_for_xtream_target, handle_error, handle_error_and_return, notify_err};
use crate::utils::json_utils::{get_u32_from_serde_value, get_u64_from_serde_value};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};

const TAG_VOD_INFO_INFO: &str = "info";
const TAG_VOD_INFO_MOVIE_DATA: &str = "movie_data";
const TAG_VOD_INFO_TMDB_ID: &str = "tmdb_id";
const TAG_VOD_INFO_STREAM_ID: &str = "stream_id";
const TAG_VOD_INFO_ADDED: &str = "added";

create_resolve_options_function_for_xtream_target!(video);

async fn read_processed_vod_info_ids(cfg: &Config, errors: &mut Vec<M3uFilterError>, fpl: &FetchedPlaylist<'_>) -> HashMap<u32, u64> {
    read_processed_info_ids(cfg, errors, fpl, PlaylistItemType::Video, |record: &InputVodInfoRecord| record.ts).await
}

fn extract_info_record_from_vod_info(content: &str) -> Option<(u32, InputVodInfoRecord)> {
    let doc = serde_json::from_str::<Map<String, Value>>(content).ok()?;

    let movie_data = doc.get(TAG_VOD_INFO_MOVIE_DATA)?.as_object()?;
    let provider_id = get_u32_from_serde_value(
        movie_data.get(TAG_VOD_INFO_STREAM_ID)?,
    )?;

    let added = movie_data
        .get(TAG_VOD_INFO_ADDED)
        .and_then(get_u64_from_serde_value)
        .unwrap_or(0);

    let tmdb_id = doc.get(TAG_VOD_INFO_INFO)?.as_object()
        .and_then(|info| info.get(TAG_VOD_INFO_TMDB_ID))
        .and_then(get_u32_from_serde_value)
        .unwrap_or(0);

    Some((provider_id, InputVodInfoRecord {
        tmdb_id,
        ts: added,
    }))
}

fn write_vod_info_record_to_wal_file(
    writer: &mut BufWriter<&File>,
    provider_id: u32,
    record: &InputVodInfoRecord,
) -> std::io::Result<()> {
    writer.write_all(&provider_id.to_le_bytes())?;
    writer.write_all(&record.tmdb_id.to_le_bytes())?;
    writer.write_all(&record.ts.to_le_bytes())?;
    Ok(())
}

fn should_update_vod_info(pli: &PlaylistItem, processed_provider_ids: &HashMap<u32, u64>) -> (bool, u32, u64) {
    should_update_info(pli, processed_provider_ids, TAG_VOD_INFO_ADDED)
}

pub async fn playlist_resolve_vod(cfg: &Config, target: &ConfigTarget, errors: &mut Vec<M3uFilterError>, fpl: &FetchedPlaylist<'_>) {
    let (resolve_movies, resolve_delay) = get_resolve_video_options(target, fpl);
    if !resolve_movies { return; }

    // we cant write to the indexed-document directly because of the write lock and time-consuming operation.
    // All readers would be waiting for the lock and the app would be unresponsive.
    // We collect the content into a wal file and write it once we collected everything.
    let Some((wal_content_file, wal_record_file, wal_content_path, wal_record_path)) = create_resolve_info_wal_files(cfg, fpl.input, XtreamCluster::Video)
    else { return; };

    let mut processed_info_ids = read_processed_vod_info_ids(cfg, errors, fpl).await;
    let mut content_writer = BufWriter::new(&wal_content_file);
    let mut record_writer = BufWriter::new(&wal_record_file);
    let mut content_updated = false;

    for pli in fpl.playlistgroups.iter()
        .flat_map(|plg| &plg.channels)
        .filter(|&pli| pli.header.borrow().xtream_cluster == XtreamCluster::Video) {
        let (should_update, _provider_id, _ts) = should_update_vod_info(pli, &processed_info_ids);
        if should_update {
            if let Some(content) = playlist_resolve_download_playlist_item(pli, fpl.input, errors, resolve_delay, XtreamCluster::Video).await {
                if let Some((provider_id, info_record)) = extract_info_record_from_vod_info(&content) {
                    let ts = info_record.ts;
                    handle_error_and_return!(write_info_content_to_wal_file(&mut content_writer, provider_id, &content),
                        |err| errors.push(notify_err!(format!("Failed to resolve vod, could not write to content wal file {err}"))));
                    processed_info_ids.insert(provider_id, ts);
                    handle_error_and_return!(write_vod_info_record_to_wal_file(&mut record_writer, provider_id, &info_record),
                        |err| errors.push(notify_err!(format!("Failed to resolve vod wal, could not write to record wal file {err}"))));
                    content_updated = true;
                }
            }
        }
    }
    if content_updated {
        handle_error!(content_writer.flush(),
            |err| errors.push(notify_err!(format!("Failed to resolve vod, could not write to wal file {err}"))));
        handle_error!(record_writer.flush(),
            |err| errors.push(notify_err!(format!("Failed to resolve vod tmdb, could not write to wal file {err}"))));
        drop(content_writer);
        drop(record_writer);
        drop(wal_content_file);
        drop(wal_record_file);
        handle_error!(xtream_update_input_info_file(cfg, fpl.input, &wal_content_path, XtreamCluster::Video).await,
            |err| errors.push(err));
        handle_error!(xtream_update_input_vod_record_from_wal_file(cfg, fpl.input, &wal_record_path).await,
            |err| errors.push(err));
    }
}