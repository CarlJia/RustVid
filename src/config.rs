use std::{env, path::PathBuf};

use anyhow::Context;

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub max_file_size: u64,
    pub max_concurrent_transcodes: usize,
}

impl Config {
    pub const DEFAULT_MAX_FILE_SIZE: u64 = 2 * 1024 * 1024 * 1024;
    pub const DEFAULT_MAX_CONCURRENT_TRANSCODES: usize = 2;

    pub fn from_env() -> anyhow::Result<Self> {
        let data_dir = env::var("RUSTVID_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data"));
        Self::new(data_dir)
    }

    pub fn new(data_dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&data_dir).context("创建数据目录失败")?;
        Ok(Self {
            data_dir,
            max_file_size: Self::DEFAULT_MAX_FILE_SIZE,
            max_concurrent_transcodes: Self::DEFAULT_MAX_CONCURRENT_TRANSCODES,
        })
    }

    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.data_dir.join("artifacts")
    }

    pub fn work_dir(&self) -> PathBuf {
        self.data_dir.join("work")
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("rustvid.sqlite3")
    }
}
