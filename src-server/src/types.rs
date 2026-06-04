use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::config::Config;
use crate::worker_pool::WorkerPool;
use crate::metrics::Metrics;
use crate::logger::Logger;

#[derive(Debug, Serialize)]
pub struct PhpRequest {
    pub method: String,
    pub uri: String,
    pub file_path: String,
    pub query_string: String,
    pub query_params: HashMap<String, String>,
    pub post_params: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub content_type: String,
    pub content_length: String,
    pub files: HashMap<String, Vec<FileInfo>>,
}

#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub name: String,
    #[serde(rename = "type")]
    pub content_type: String,
    pub size: usize,
    pub tmp_path: String,
}

#[derive(Debug, Deserialize)]
pub struct PhpResponse {
    pub status: u16,
    pub body: String,
    pub memory: u64,
    pub peak: u64,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: Arc<Mutex<WorkerPool>>,
    pub metrics: Arc<Mutex<Metrics>>,
    pub logger: Arc<Logger>,
}