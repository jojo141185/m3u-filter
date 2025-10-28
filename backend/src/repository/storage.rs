use std::path::{Path, PathBuf};
use shared::error::{TuliproxError};
use crate::model::{Config};
use shared::error::{notify_err};
use crate::repository::storage_const;
use crate::utils;

pub(in crate::repository) fn get_target_id_mapping_file(target_path: &Path) -> PathBuf {
    target_path.join(PathBuf::from(storage_const::FILE_ID_MAPPING))
}

pub fn ensure_target_storage_path(cfg: &Config, target_name: &str) -> Result<PathBuf, TuliproxError> {
    if let Some(path) = get_target_storage_path(cfg, target_name) {
        if std::fs::create_dir_all(&path).is_err() {
            let msg = format!("Failed to save target data, can't create directory {}", path.display());
            return Err(notify_err!(msg));
        }
        Ok(path)
    } else {
        let msg = format!("Failed to save target data, can't create directory for target {target_name}");
        Err(notify_err!(msg))
    }
}

pub fn get_target_storage_path(cfg: &Config, target_name: &str) -> Option<PathBuf> {
    utils::get_file_path(&cfg.working_dir, Some(std::path::PathBuf::from(target_name.replace(' ', "_"))))
}

pub fn get_input_storage_path(input_name: &str, working_dir: &str) -> std::io::Result<PathBuf> {
    let name =  format!("input_{input_name}");
    let path = Path::new(working_dir).join(name);
    // Create the directory and return the path or propagate the error
    std::fs::create_dir_all(&path).map(|()| path)
}

pub fn get_geoip_path(working_dir: &str) -> PathBuf {
    Path::new(working_dir).join("geoip.db")
}