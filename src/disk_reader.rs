use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use anyhow::{Result, Context, anyhow};
use thiserror::Error;
use parking_lot::Mutex;

pub const SECTOR_SIZE: u64 = 512;

#[derive(Debug, Error)]
pub enum DiskReaderError {
    #[error("不支持的镜像格式: {0}")]
    UnsupportedFormat(String),
    #[error("IO 错误: {0}")]
    IoError(#[from] std::io::Error),
    #[error("E01 解析错误: {0}")]
    E01Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    RawDd,
    E01,
}

#[derive(Debug, Clone)]
pub struct DiskInfo {
    pub format: ImageFormat,
    pub total_size: u64,
    pub total_sectors: u64,
    pub sector_size: u64,
    pub path: PathBuf,
}

pub trait DiskImageReader: Send + Sync {
    fn read_sectors(&mut self, start_sector: u64, count: u64, buffer: &mut [u8]) -> Result<usize>;
    fn read_offset(&mut self, offset: u64, size: u64, buffer: &mut [u8]) -> Result<usize>;
    fn get_disk_info(&self) -> &DiskInfo;
    fn get_total_size(&self) -> u64 {
        self.get_disk_info().total_size
    }
    fn box_clone(&self) -> Box<dyn DiskImageReader>;
}

impl Clone for Box<dyn DiskImageReader> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}

impl DiskImageReader for Box<dyn DiskImageReader> {
    fn read_sectors(&mut self, start_sector: u64, count: u64, buffer: &mut [u8]) -> Result<usize> {
        (**self).read_sectors(start_sector, count, buffer)
    }

    fn read_offset(&mut self, offset: u64, size: u64, buffer: &mut [u8]) -> Result<usize> {
        (**self).read_offset(offset, size, buffer)
    }

    fn get_disk_info(&self) -> &DiskInfo {
        (**self).get_disk_info()
    }

    fn box_clone(&self) -> Box<dyn DiskImageReader> {
        (**self).box_clone()
    }
}

pub fn create_reader<P: AsRef<Path>>(path: P) -> Result<Box<dyn DiskImageReader>> {
    let path = path.as_ref();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let reader: Box<dyn DiskImageReader> = match extension.as_str() {
        "dd" | "img" | "raw" | "bin" => Box::new(RawDiskReader::open(path)?),
        "e01" | "s01" | "l01" => Box::new(E01Reader::open(path)?),
        _ => {
            let raw_reader = RawDiskReader::open(path);
            if let Ok(reader) = raw_reader {
                Box::new(reader)
            } else {
                let e01_reader = E01Reader::open(path);
                if let Ok(reader) = e01_reader {
                    Box::new(reader)
                } else {
                    return Err(DiskReaderError::UnsupportedFormat(
                        path.display().to_string()
                    ).into());
                }
            }
        }
    };

    Ok(reader)
}

pub struct RawDiskReader {
    path: PathBuf,
    file: Arc<Mutex<File>>,
    info: DiskInfo,
}

impl RawDiskReader {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)
            .with_context(|| format!("无法打开镜像文件: {}", path.display()))?;
        
        let total_size = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;

        let info = DiskInfo {
            format: ImageFormat::RawDd,
            total_size,
            total_sectors: total_size / SECTOR_SIZE,
            sector_size: SECTOR_SIZE,
            path: path.clone(),
        };

        Ok(Self {
            path,
            file: Arc::new(Mutex::new(file)),
            info,
        })
    }
}

impl Clone for RawDiskReader {
    fn clone(&self) -> Self {
        let mut file = File::open(&self.path)
            .expect(&format!("无法重新打开镜像文件: {}", self.path.display()));
        file.seek(SeekFrom::Start(0)).ok();
        
        Self {
            path: self.path.clone(),
            file: Arc::new(Mutex::new(file)),
            info: self.info.clone(),
        }
    }
}

impl DiskImageReader for RawDiskReader {
    fn read_sectors(&mut self, start_sector: u64, count: u64, buffer: &mut [u8]) -> Result<usize> {
        let offset = start_sector * SECTOR_SIZE;
        let bytes_to_read = count * SECTOR_SIZE;
        
        if buffer.len() < bytes_to_read as usize {
            return Err(anyhow!("缓冲区太小，需要 {} 字节，实际只有 {} 字节", 
                bytes_to_read, buffer.len()));
        }

        let mut file = self.file.lock();
        file.seek(SeekFrom::Start(offset))?;
        let bytes_read = file.read(&mut buffer[..bytes_to_read as usize])?;
        
        Ok(bytes_read)
    }

    fn read_offset(&mut self, offset: u64, size: u64, buffer: &mut [u8]) -> Result<usize> {
        if buffer.len() < size as usize {
            return Err(anyhow!("缓冲区太小，需要 {} 字节，实际只有 {} 字节", 
                size, buffer.len()));
        }

        let mut file = self.file.lock();
        file.seek(SeekFrom::Start(offset))?;
        let bytes_read = file.read(&mut buffer[..size as usize])?;
        
        Ok(bytes_read)
    }

    fn get_disk_info(&self) -> &DiskInfo {
        &self.info
    }

    fn box_clone(&self) -> Box<dyn DiskImageReader> {
        Box::new(self.clone())
    }
}

#[derive(Debug, Clone)]
struct E01Chunk {
    offset: u64,
    size: u32,
    is_compressed: bool,
    data_offset: u64,
}

pub struct E01Reader {
    path: PathBuf,
    file: Arc<Mutex<File>>,
    info: DiskInfo,
    chunks: Arc<Vec<E01Chunk>>,
}

impl E01Reader {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)
            .with_context(|| format!("无法打开 E01 镜像文件: {}", path.display()))?;
        
        let total_file_size = file.seek(SeekFrom::End(0))?;
        file.seek(SeekFrom::Start(0))?;

        let chunks = Self::parse_e01_structure(&mut file, total_file_size)?;
        
        let total_size = chunks.iter().map(|c| c.size as u64).sum();

        let info = DiskInfo {
            format: ImageFormat::E01,
            total_size,
            total_sectors: total_size / SECTOR_SIZE,
            sector_size: SECTOR_SIZE,
            path: path.clone(),
        };

        Ok(Self {
            path,
            file: Arc::new(Mutex::new(file)),
            info,
            chunks: Arc::new(chunks),
        })
    }

    fn parse_e01_structure(file: &mut File, file_size: u64) -> Result<Vec<E01Chunk>> {
        let mut header = [0u8; 5];
        file.seek(SeekFrom::Start(0))?;
        file.read_exact(&mut header)?;

        if &header != b"EVF\x09\x0d" {
            return Err(DiskReaderError::E01Error(
                "无效的 E01 头部签名".to_string()
            ).into());
        }

        let mut chunks = Vec::new();
        let mut current_offset = 0u64;
        let mut logical_offset = 0u64;

        while current_offset < file_size {
            file.seek(SeekFrom::Start(current_offset))?;
            
            let mut section_header = [0u8; 16];
            if file.read(&mut section_header)? < 16 {
                break;
            }

            let section_type = &section_header[0..1];
            let _section_size = u64::from_le_bytes(
                section_header[8..16].try_into().unwrap()
            );

            if section_type == b"S" || section_type == b"s" {
                let mut chunk_header = [0u8; 40];
                file.read_exact(&mut chunk_header)?;

                let data_size = u32::from_le_bytes(chunk_header[0..4].try_into().unwrap());
                let logical_size = u32::from_le_bytes(chunk_header[8..12].try_into().unwrap());
                let flags = u32::from_le_bytes(chunk_header[12..16].try_into().unwrap());
                
                let is_compressed = (flags & 0x01) != 0;
                let data_offset = current_offset + 56;

                if logical_size > 0 {
                    chunks.push(E01Chunk {
                        offset: logical_offset,
                        size: logical_size,
                        is_compressed,
                        data_offset,
                    });
                    logical_offset += logical_size as u64;
                }

                let actual_data_size = if is_compressed { data_size } else { logical_size };
                current_offset += 56 + actual_data_size as u64;
            } else if section_type == b"D" {
                break;
            } else {
                let mut next_offset_bytes = [0u8; 8];
                if file.read(&mut next_offset_bytes)? == 8 {
                    let next_offset = u64::from_le_bytes(next_offset_bytes);
                    if next_offset > current_offset && next_offset < file_size {
                        current_offset = next_offset;
                        continue;
                    }
                }
                break;
            }
        }

        if chunks.is_empty() {
            return Self::parse_e01_fallback(file, file_size, &mut chunks);
        }

        Ok(chunks)
    }

    fn parse_e01_fallback(file: &mut File, file_size: u64, chunks: &mut Vec<E01Chunk>) -> Result<Vec<E01Chunk>> {
        let mut logical_offset = 0u64;
        let mut current_offset = 1024u64;

        while current_offset < file_size - 16 {
            file.seek(SeekFrom::Start(current_offset))?;
            
            let mut marker = [0u8; 4];
            if file.read(&mut marker)? < 4 {
                break;
            }

            if &marker == b"\x01\x00\x00\x00" || &marker == b"\x00\x00\x00\x00" {
                let mut header = [0u8; 16];
                if file.read(&mut header)? < 16 {
                    break;
                }
                
                let data_size = u32::from_le_bytes(header[0..4].try_into().unwrap_or([0; 4]));
                let is_compressed = (data_size & 0x80000000) != 0;
                let actual_size = data_size & 0x7FFFFFFF;
                
                if actual_size > 0 && actual_size < 100 * 1024 * 1024 {
                    chunks.push(E01Chunk {
                        offset: logical_offset,
                        size: actual_size,
                        is_compressed,
                        data_offset: current_offset + 16,
                    });
                    logical_offset += actual_size as u64;
                    current_offset += 16 + actual_size as u64;
                } else {
                    current_offset += 512;
                }
            } else {
                current_offset += 512;
            }
        }

        if chunks.is_empty() {
            file.seek(SeekFrom::End(0))?;
            let total_size = file.stream_position()?;
            
            chunks.push(E01Chunk {
                offset: 0,
                size: (total_size - 4096) as u32,
                is_compressed: false,
                data_offset: 4096,
            });
        }

        Ok(chunks.clone())
    }

    fn read_chunk(&mut self, chunk: &E01Chunk, buffer: &mut [u8]) -> Result<usize> {
        let mut file = self.file.lock();
        file.seek(SeekFrom::Start(chunk.data_offset))?;
        
        if chunk.is_compressed {
            let mut compressed_data = vec![0u8; chunk.size as usize];
            file.read_exact(&mut compressed_data)?;
            
            let decompressed = zlib_decompress(&compressed_data)?;
            let bytes_to_copy = decompressed.len().min(buffer.len());
            buffer[..bytes_to_copy].copy_from_slice(&decompressed[..bytes_to_copy]);
            
            Ok(bytes_to_copy)
        } else {
            let bytes_to_read = chunk.size.min(buffer.len() as u32);
            file.read_exact(&mut buffer[..bytes_to_read as usize])?;
            
            Ok(bytes_to_read as usize)
        }
    }
}

impl Clone for E01Reader {
    fn clone(&self) -> Self {
        let mut file = File::open(&self.path)
            .expect(&format!("无法重新打开镜像文件: {}", self.path.display()));
        file.seek(SeekFrom::Start(0)).ok();
        
        Self {
            path: self.path.clone(),
            file: Arc::new(Mutex::new(file)),
            info: self.info.clone(),
            chunks: self.chunks.clone(),
        }
    }
}

impl DiskImageReader for E01Reader {
    fn read_sectors(&mut self, start_sector: u64, count: u64, buffer: &mut [u8]) -> Result<usize> {
        let offset = start_sector * SECTOR_SIZE;
        let bytes_to_read = count * SECTOR_SIZE;
        
        if buffer.len() < bytes_to_read as usize {
            return Err(anyhow!("缓冲区太小"));
        }

        self.read_offset(offset, bytes_to_read, buffer)
    }

    fn read_offset(&mut self, offset: u64, size: u64, buffer: &mut [u8]) -> Result<usize> {
        let mut bytes_read_total = 0usize;
        let mut remaining_size = size as usize;
        let mut current_logical_offset = offset;
        let mut buffer_offset = 0usize;

        let chunks = self.chunks.clone();
        for chunk in chunks.iter() {
            if remaining_size == 0 {
                break;
            }

            let chunk_end = chunk.offset + chunk.size as u64;
            
            if current_logical_offset >= chunk_end {
                continue;
            }

            if current_logical_offset < chunk.offset {
                let gap = (chunk.offset - current_logical_offset) as usize;
                let fill_gap = gap.min(remaining_size);
                for i in 0..fill_gap {
                    buffer[buffer_offset + i] = 0;
                }
                buffer_offset += fill_gap;
                bytes_read_total += fill_gap;
                remaining_size -= fill_gap;
                current_logical_offset += fill_gap as u64;
                
                if remaining_size == 0 {
                    break;
                }
            }

            let offset_in_chunk = (current_logical_offset - chunk.offset) as usize;
            let available_in_chunk = chunk.size as usize - offset_in_chunk;
            let bytes_to_read_from_chunk = available_in_chunk.min(remaining_size);

            if bytes_to_read_from_chunk > 0 {
                let mut chunk_buffer = vec![0u8; chunk.size as usize];
                let chunk_bytes_read = self.read_chunk(chunk, &mut chunk_buffer)?;
                
                let copy_len = bytes_to_read_from_chunk.min(chunk_bytes_read - offset_in_chunk);
                buffer[buffer_offset..buffer_offset + copy_len].copy_from_slice(
                    &chunk_buffer[offset_in_chunk..offset_in_chunk + copy_len]
                );
                
                buffer_offset += copy_len;
                bytes_read_total += copy_len;
                remaining_size -= copy_len;
                current_logical_offset += copy_len as u64;
            }
        }

        while remaining_size > 0 {
            buffer[buffer_offset] = 0;
            buffer_offset += 1;
            bytes_read_total += 1;
            remaining_size -= 1;
        }

        Ok(bytes_read_total)
    }

    fn get_disk_info(&self) -> &DiskInfo {
        &self.info
    }

    fn box_clone(&self) -> Box<dyn DiskImageReader> {
        Box::new(self.clone())
    }
}

fn zlib_decompress(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut buffer = Vec::new();
    decoder.read_to_end(&mut buffer)
        .map_err(|e| anyhow!("Zlib 解压失败: {}", e))?;
    Ok(buffer)
}

unsafe impl Send for E01Reader {}
unsafe impl Sync for E01Reader {}
unsafe impl Send for RawDiskReader {}
unsafe impl Sync for RawDiskReader {}
