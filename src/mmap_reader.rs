use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;
use anyhow::{Result, Context};
use memmap2::{Mmap, MmapOptions};

use crate::disk_reader::{DiskImageReader, DiskInfo, ImageFormat, SECTOR_SIZE};

pub struct MmapDiskReader {
    file: File,
    mmap: Option<Arc<Mmap>>,
    info: DiskInfo,
    current_offset: u64,
}

impl MmapDiskReader {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("无法打开文件: {}", path.display()))?;

        let metadata = file.metadata()?;
        let total_size = metadata.len();

        let info = DiskInfo {
            format: ImageFormat::RawDd,
            total_size,
            total_sectors: total_size / SECTOR_SIZE,
            sector_size: SECTOR_SIZE,
            path: path.to_path_buf(),
        };

        let mmap = if total_size > 0 {
            let m = unsafe { Mmap::map(&file) }
                .with_context(|| "无法创建内存映射".to_string())?;
            Some(Arc::new(m))
        } else {
            None
        };

        Ok(MmapDiskReader {
            file,
            mmap,
            info,
            current_offset: 0,
        })
    }

    pub fn get_mmap(&self) -> Option<&[u8]> {
        self.mmap.as_deref().map(|m| &m[..])
    }

    pub fn as_slice(&self) -> Option<&[u8]> {
        self.mmap.as_deref().map(|m| &m[..])
    }
}

impl DiskImageReader for MmapDiskReader {
    fn read_sectors(&mut self, start_sector: u64, count: u64, buffer: &mut [u8]) -> Result<usize> {
        let offset = start_sector * SECTOR_SIZE;
        let size = count * SECTOR_SIZE;
        self.read_offset(offset, size, buffer)
    }

    fn read_offset(&mut self, offset: u64, size: u64, buffer: &mut [u8]) -> Result<usize> {
        let total_size = self.info.total_size;
        if offset >= total_size {
            return Ok(0);
        }

        let available = total_size.saturating_sub(offset);
        let to_read = size.min(available).min(buffer.len() as u64) as usize;

        if to_read == 0 {
            return Ok(0);
        }

        if let Some(slice) = self.mmap.as_deref() {
            let start = offset as usize;
            let end = start + to_read;
            buffer[..to_read].copy_from_slice(&slice[start..end]);
            self.current_offset = offset + to_read as u64;
            Ok(to_read)
        } else {
            self.file.seek(SeekFrom::Start(offset))?;
            let bytes_read = self.file.read(&mut buffer[..to_read])?;
            self.current_offset = offset + bytes_read as u64;
            Ok(bytes_read)
        }
    }

    fn get_disk_info(&self) -> &DiskInfo {
        &self.info
    }

    fn box_clone(&self) -> Box<dyn DiskImageReader> {
        Box::new(MmapDiskReader {
            file: self.file.try_clone().expect("无法克隆文件句柄"),
            mmap: self.mmap.clone(),
            info: self.info.clone(),
            current_offset: 0,
        })
    }
}

impl Clone for MmapDiskReader {
    fn clone(&self) -> Self {
        MmapDiskReader {
            file: self.file.try_clone().expect("无法克隆文件句柄"),
            mmap: self.mmap.clone(),
            info: self.info.clone(),
            current_offset: 0,
        }
    }
}

pub struct ChunkedMmapReader {
    file: File,
    info: DiskInfo,
    chunk_size: u64,
}

impl ChunkedMmapReader {
    pub fn new<P: AsRef<Path>>(path: P, chunk_size_gb: Option<u64>) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path)
            .with_context(|| format!("无法打开文件: {}", path.display()))?;

        let metadata = file.metadata()?;
        let total_size = metadata.len();

        let info = DiskInfo {
            format: ImageFormat::RawDd,
            total_size,
            total_sectors: total_size / SECTOR_SIZE,
            sector_size: SECTOR_SIZE,
            path: path.to_path_buf(),
        };

        let chunk_size = chunk_size_gb.unwrap_or(4) * 1024 * 1024 * 1024;

        Ok(ChunkedMmapReader {
            file,
            info,
            chunk_size,
        })
    }

    pub fn map_chunk(&self, offset: u64) -> Result<Arc<Mmap>> {
        let total_size = self.info.total_size;
        if offset >= total_size {
            return Err(anyhow::anyhow!("偏移量超出文件大小"));
        }

        let chunk_start = (offset / self.chunk_size) * self.chunk_size;
        let mapped_len = self.chunk_size.min(total_size.saturating_sub(chunk_start));

        let mmap = unsafe {
            MmapOptions::new()
                .offset(chunk_start)
                .len(mapped_len as usize)
                .map(&self.file)
        }.with_context(|| format!("无法映射内存区域 offset=0x{:X} len={}", chunk_start, mapped_len))?;

        Ok(Arc::new(mmap))
    }

    pub fn get_chunk_slice<'a>(&self, mmap: &'a Mmap, offset: u64) -> &'a [u8] {
        let chunk_start = (offset / self.chunk_size) * self.chunk_size;
        let rel_offset = (offset - chunk_start) as usize;
        &mmap[rel_offset..]
    }

    pub fn total_size(&self) -> u64 {
        self.info.total_size
    }

    pub fn chunk_size(&self) -> u64 {
        self.chunk_size
    }

    pub fn info(&self) -> &DiskInfo {
        &self.info
    }
}

pub struct MmapScanEngine {
    mmap: Arc<Mmap>,
    total_size: u64,
}

impl MmapScanEngine {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(path)?;
        let metadata = file.metadata()?;
        let total_size = metadata.len();
        let mmap = unsafe { Mmap::map(&file) }
            .with_context(|| "无法创建内存映射".to_string())?;

        Ok(MmapScanEngine {
            mmap: Arc::new(mmap),
            total_size,
        })
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.mmap
    }

    pub fn total_size(&self) -> u64 {
        self.total_size
    }

    pub fn scan_chunk<F>(
        &self,
        chunk_start: u64,
        chunk_end: u64,
        _handler: F,
    ) -> usize
    where
        F: FnMut(usize),
    {
        let start = chunk_start.min(self.total_size) as usize;
        let end = chunk_end.min(self.total_size) as usize;
        end.saturating_sub(start)
    }
}

unsafe impl Send for MmapScanEngine {}
unsafe impl Sync for MmapScanEngine {}

impl Clone for MmapScanEngine {
    fn clone(&self) -> Self {
        MmapScanEngine {
            mmap: self.mmap.clone(),
            total_size: self.total_size,
        }
    }
}
