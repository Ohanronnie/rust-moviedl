use serde::{Deserialize, Serialize};
use std::fs::File;

#[derive(Serialize, Deserialize, Debug)]
pub struct MetaData {
    pub url: String,
    pub file: String,
    pub size: u64,
    pub chunk_size: u64,
    pub chunks: usize,
    pub progress: Vec<bool>,
}

pub fn load_meta(path: &str) -> Option<MetaData> {
    let meta_path = format!("{}.meta.json", path);
    File::open(&meta_path).ok().and_then(|f| serde_json::from_reader(f).ok())
}

pub fn save_meta(path: &str, meta: &MetaData) -> Result<(), std::io::Error> {
    let meta_path = format!("{}.meta.json", path);
    if let Ok(mut file) = File::create(&meta_path) {
        let _ = serde_json::to_writer_pretty(&mut file, meta);
    } 
   Ok(())
}
