use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use anyhow::{Result, Context, anyhow};
use memchr::memmem;
use rayon::prelude::*;
use parking_lot::Mutex;

use crate::disk_reader::{DiskImageReader, SECTOR_SIZE};
use crate::file_types::{FileSignature, FileType, get_all_signatures, get_max_header_length, get_max_footer_length};
use crate::fs_parser::{SmartCarver, ClusterChain, FileSystemType, create_filesystem_parser};

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub file_type: FileType,
    pub start_offset: u64,
    pub end_offset: u64,
    pub size: u64,
    pub md5_hash: Option<String>,
    pub sha256_hash: Option<String>,
    pub recovered: bool,
    pub output_path: Option<PathBuf>,
    pub sector_start: u64,
    pub sector_end: u64,
    pub fs_rebuilt: bool,
    pub is_fragmented: bool,
    pub cluster_count: u64,
    pub fs_type: FileSystemType,
}

#[derive(Debug, Clone)]
pub struct CarveStats {
    pub total_bytes_processed: u64,
    pub total_matches: usize,
    pub matches_by_type: HashMap<FileType, usize>,
    pub successful_recoveries: usize,
    pub failed_recoveries: usize,
    pub fs_rebuilt_count: usize,
    pub fragmented_count: usize,
    pub contiguous_count: usize,
    pub elapsed_time: Duration,
    pub bytes_per_second: f64,
    pub start_time: Instant,
    pub detected_fs: FileSystemType,
}

impl CarveStats {
    pub fn new() -> Self {
        Self {
            total_bytes_processed: 0,
            total_matches: 0,
            matches_by_type: HashMap::new(),
            successful_recoveries: 0,
            failed_recoveries: 0,
            fs_rebuilt_count: 0,
            fragmented_count: 0,
            contiguous_count: 0,
            elapsed_time: Duration::from_secs(0),
            bytes_per_second: 0.0,
            start_time: Instant::now(),
            detected_fs: FileSystemType::Unknown,
        }
    }

    pub fn update(&mut self, bytes_processed: u64) {
        self.total_bytes_processed = bytes_processed;
        self.elapsed_time = self.start_time.elapsed();
        if self.elapsed_time.as_secs_f64() > 0.0 {
            self.bytes_per_second = bytes_processed as f64 / self.elapsed_time.as_secs_f64();
        }
    }

    pub fn add_match(&mut self, file_type: FileType) {
        self.total_matches += 1;
        *self.matches_by_type.entry(file_type).or_insert(0) += 1;
    }
}

#[derive(Debug, Clone)]
pub struct CarverConfig {
    pub chunk_size: usize,
    pub overlap_size: usize,
    pub thread_count: usize,
    pub min_match_interval: u64,
    pub output_dir: PathBuf,
    pub calculate_hashes: bool,
    pub sector_aligned: bool,
    pub selected_types: Option<Vec<FileType>>,
    pub use_filesystem_metadata: bool,
    pub partition_offset: u64,
    pub fs_fallback: bool,
}

impl Default for CarverConfig {
    fn default() -> Self {
        let max_header = get_max_header_length();
        let max_footer = get_max_footer_length();
        let overlap = (max_header.max(max_footer) + 4096).next_power_of_two();
        
        Self {
            chunk_size: 64 * 1024 * 1024,
            overlap_size: overlap,
            thread_count: num_cpus::get(),
            min_match_interval: 512,
            output_dir: PathBuf::from("./recovered"),
            calculate_hashes: true,
            sector_aligned: true,
            selected_types: None,
            use_filesystem_metadata: true,
            partition_offset: 0,
            fs_fallback: true,
        }
    }
}

pub struct DiskCarver {
    config: CarverConfig,
    signatures: Vec<FileSignature>,
    pub stats: Arc<Mutex<CarveStats>>,
    matches: Arc<Mutex<Vec<MatchResult>>>,
    smart_carver: Option<SmartCarver>,
}

impl DiskCarver {
    pub fn new(config: CarverConfig) -> Self {
        let all_sigs = get_all_signatures();
        let signatures = match &config.selected_types {
            Some(types) => {
                all_sigs.into_iter()
                    .filter(|s| types.contains(&s.file_type))
                    .collect()
            }
            None => all_sigs,
        };

        Self {
            config,
            signatures,
            stats: Arc::new(Mutex::new(CarveStats::new())),
            matches: Arc::new(Mutex::new(Vec::new())),
            smart_carver: None,
        }
    }

    pub fn init_filesystem<R: DiskImageReader>(&mut self, reader: &mut R) {
        if self.config.use_filesystem_metadata {
            let parser = create_filesystem_parser(reader, self.config.partition_offset);
            if let Some(ref p) = parser {
                let fs_type = p.fs_type();
                {
                    let mut stats = self.stats.lock();
                    stats.detected_fs = fs_type;
                }
            }
            self.smart_carver = Some(SmartCarver::new(parser));
        }
    }

    pub fn has_filesystem(&self) -> bool {
        self.smart_carver.as_ref().map(|s| s.has_filesystem()).unwrap_or(false)
    }

    pub fn get_stats(&self) -> CarveStats {
        self.stats.lock().clone()
    }

    pub fn get_matches(&self) -> Vec<MatchResult> {
        self.matches.lock().clone()
    }

    pub fn scan<R: DiskImageReader + Clone + 'static>(&mut self, reader: &mut R) -> Result<Vec<MatchResult>> {
        let total_size = reader.get_total_size();
        let chunk_size = self.config.chunk_size as u64;
        let overlap = self.config.overlap_size as u64;
        let thread_count = self.config.thread_count;

        self.init_filesystem(reader);

        if self.has_filesystem() {
            let fs_type = self.stats.lock().detected_fs;
            let fs_name = match fs_type {
                FileSystemType::Ntfs => "NTFS",
                FileSystemType::Fat32 => "FAT32",
                FileSystemType::Fat16 => "FAT16",
                FileSystemType::Unknown => "Unknown",
            };
            println!("{}", format!("检测到文件系统: {}，将使用元数据进行智能雕刻", fs_name).green());
        }

        let mut offsets: Vec<u64> = Vec::new();
        let mut current = 0u64;
        
        while current < total_size {
            offsets.push(current);
            if current + chunk_size >= total_size {
                break;
            }
            current += chunk_size - overlap;
        }

        let num_threads = thread_count.min(offsets.len());
        if num_threads > 0 {
            rayon::ThreadPoolBuilder::new()
                .num_threads(num_threads)
                .build_global()
                .ok();
        }

        let signatures = self.signatures.clone();
        let config = self.config.clone();
        let stats = self.stats.clone();
        let matches = self.matches.clone();
        let reader = reader.clone();

        let offsets_clone = offsets.clone();
        
        println!("{}", format!("开始扫描，共 {} 个区块，使用 {} 个线程", offsets.len(), num_threads).cyan());

        offsets_clone.par_iter().for_each(|&offset| {
            let reader_clone = Box::new(reader.clone());
            let chunk_end = (offset + chunk_size).min(total_size);
            let read_size = chunk_end - offset;
            
            let read_result = Self::read_chunk_with_overlap(
                reader_clone,
                offset,
                read_size,
                overlap,
                total_size
            );

            match read_result {
                Ok((buffer, actual_start, actual_end)) => {
                    let local_matches = Self::scan_buffer(
                        &buffer,
                        actual_start,
                        actual_end,
                        &signatures,
                        &config,
                    );

                    if !local_matches.is_empty() {
                        let mut all_matches = matches.lock();
                        let mut stats_lock = stats.lock();
                        
                        for m in local_matches {
                            let is_duplicate = all_matches.iter().any(|existing| {
                                (existing.file_type == m.file_type) &&
                                (existing.start_offset.abs_diff(m.start_offset) < config.min_match_interval)
                            });
                            
                            if !is_duplicate {
                                stats_lock.add_match(m.file_type);
                                all_matches.push(m);
                            }
                        }
                    }

                    let mut stats_lock = stats.lock();
                    let processed = offset + read_size;
                    if processed > stats_lock.total_bytes_processed {
                        stats_lock.update(processed.min(total_size));
                    }
                }
                Err(e) => {
                    eprintln!("{}", format!("读取区块错误 (偏移 0x{:X}): {}", offset, e).red());
                }
            }
        });

        {
            let mut stats_lock = self.stats.lock();
            stats_lock.update(total_size);
        }

        let final_matches = self.get_matches();
        Ok(final_matches)
    }

    fn read_chunk_with_overlap(
        mut reader: Box<dyn DiskImageReader>,
        offset: u64,
        size: u64,
        overlap: u64,
        total_size: u64,
    ) -> Result<(Vec<u8>, u64, u64)> {
        let actual_start = offset.saturating_sub(overlap);
        let actual_end = (offset + size + overlap).min(total_size);
        let actual_size = actual_end - actual_start;

        let mut buffer = vec![0u8; actual_size as usize];
        
        let bytes_read = reader.read_offset(actual_start, actual_size, &mut buffer)?;
        
        if bytes_read < actual_size as usize {
            buffer.truncate(bytes_read);
        }

        Ok((buffer, actual_start, actual_start + bytes_read as u64))
    }

    fn scan_buffer(
        buffer: &[u8],
        buffer_start: u64,
        buffer_end: u64,
        signatures: &[FileSignature],
        config: &CarverConfig,
    ) -> Vec<MatchResult> {
        let mut results = Vec::new();
        
        for sig in signatures {
            let header_finder = memmem::Finder::new(sig.header);
            
            for header_pos in header_finder.find_iter(buffer) {
                let global_start = buffer_start + header_pos as u64;
                
                if config.sector_aligned {
                    let sector_offset = global_start % SECTOR_SIZE;
                    if sector_offset != 0 && sector_offset > 64 {
                        continue;
                    }
                }

                let mut end_offset = global_start + sig.min_size;
                let mut found_footer = false;

                if let Some(footer) = sig.footer {
                    let search_start = header_pos + sig.min_size as usize;
                    let search_end = (header_pos + sig.max_size as usize).min(buffer.len());
                    
                    if search_start < search_end {
                        let footer_finder = memmem::Finder::new(footer);
                        if let Some(footer_pos) = footer_finder.find(&buffer[search_start..search_end]) {
                            end_offset = global_start + (search_start - header_pos + footer_pos + footer.len()) as u64;
                            found_footer = true;
                        }
                    }
                }

                if !found_footer {
                    if global_start + sig.max_size <= buffer_end {
                        end_offset = global_start + sig.max_size;
                    } else {
                        end_offset = buffer_end.min(global_start + sig.max_size);
                    }
                }

                let size = end_offset - global_start;
                if size < sig.min_size || size > sig.max_size {
                    continue;
                }

                results.push(MatchResult {
                    file_type: sig.file_type,
                    start_offset: global_start,
                    end_offset,
                    size,
                    md5_hash: None,
                    sha256_hash: None,
                    recovered: false,
                    output_path: None,
                    sector_start: global_start / SECTOR_SIZE,
                    sector_end: end_offset / SECTOR_SIZE,
                    fs_rebuilt: false,
                    is_fragmented: false,
                    cluster_count: 0,
                    fs_type: FileSystemType::Unknown,
                });
            }
        }

        results.sort_by_key(|m| m.start_offset);
        results.dedup_by_key(|m| (m.file_type, m.start_offset));
        
        results
    }

    pub fn carve_files<R: DiskImageReader>(
        &mut self,
        reader: &mut R,
    ) -> Result<Vec<MatchResult>> {
        let matches = self.get_matches();
        let config = self.config.clone();
        
        std::fs::create_dir_all(&config.output_dir)
            .with_context(|| format!("无法创建输出目录: {:?}", config.output_dir))?;

        let mut type_counters: HashMap<FileType, u32> = HashMap::new();
        
        let mut carved_results = Vec::new();

        for mut m in matches {
            let counter = type_counters.entry(m.file_type).or_insert(0);
            *counter += 1;
            
            let sig = m.file_type.get_signature();
            let filename = format!("{}_{:06}.{}", sig.extension, *counter, sig.extension);
            let output_path = config.output_dir.join(filename);

            let mut used_fs_rebuild = false;
            let mut carve_result: Result<(Option<String>, Option<String>)> = Err(anyhow!("未尝试"));

            if let Some(ref sc) = self.smart_carver {
                if sc.has_filesystem() {
                    if let Some(chain) = sc.try_rebuild_file(reader, m.start_offset, sig.max_size) {
                        if !chain.is_empty() && chain.total_bytes >= sig.min_size {
                            m.fs_rebuilt = true;
                            m.is_fragmented = chain.is_fragmented;
                            m.cluster_count = chain.total_clusters;
                            m.size = chain.total_bytes;
                            m.end_offset = m.start_offset + chain.total_bytes;
                            m.fs_type = sc.fs_type();
                            m.sector_end = m.end_offset / SECTOR_SIZE;

                            carve_result = sc.carve_with_cluster_chain(
                                reader,
                                &chain,
                                &output_path,
                                config.calculate_hashes,
                            );
                            used_fs_rebuild = true;
                        }
                    }
                }
            }

            if !used_fs_rebuild {
                if !config.fs_fallback {
                    carved_results.push(m);
                    continue;
                }
                carve_result = Self::carve_single_file(
                    reader,
                    m.start_offset,
                    m.size,
                    &output_path,
                    config.calculate_hashes,
                );
            }

            match carve_result {
                Ok((md5, sha256)) => {
                    m.recovered = true;
                    m.output_path = Some(output_path);
                    m.md5_hash = md5;
                    m.sha256_hash = sha256;
                    
                    {
                        let mut stats_lock = self.stats.lock();
                        stats_lock.successful_recoveries += 1;
                        if m.fs_rebuilt {
                            stats_lock.fs_rebuilt_count += 1;
                            if m.is_fragmented {
                                stats_lock.fragmented_count += 1;
                            } else {
                                stats_lock.contiguous_count += 1;
                            }
                        }
                    }
                }
                Err(e) => {
                    if used_fs_rebuild && config.fs_fallback {
                        let fallback_result = Self::carve_single_file(
                            reader,
                            m.start_offset,
                            m.size,
                            &output_path,
                            config.calculate_hashes,
                        );
                        match fallback_result {
                            Ok((md5, sha256)) => {
                                m.fs_rebuilt = false;
                                m.is_fragmented = false;
                                m.cluster_count = 0;
                                m.fs_type = FileSystemType::Unknown;
                                m.recovered = true;
                                m.output_path = Some(output_path);
                                m.md5_hash = md5;
                                m.sha256_hash = sha256;
                                
                                {
                                    let mut stats_lock = self.stats.lock();
                                    stats_lock.successful_recoveries += 1;
                                }
                            }
                            Err(e2) => {
                                eprintln!("{}", format!("恢复文件失败 (偏移 0x{:X}): {}", m.start_offset, e2).red());
                                {
                                    let mut stats_lock = self.stats.lock();
                                    stats_lock.failed_recoveries += 1;
                                }
                            }
                        }
                    } else {
                        eprintln!("{}", format!("恢复文件失败 (偏移 0x{:X}): {}", m.start_offset, e).red());
                        {
                            let mut stats_lock = self.stats.lock();
                            stats_lock.failed_recoveries += 1;
                        }
                    }
                }
            }

            carved_results.push(m);
        }

        *self.matches.lock() = carved_results.clone();
        Ok(carved_results)
    }

    fn carve_single_file<R: DiskImageReader>(
        reader: &mut R,
        offset: u64,
        size: u64,
        output_path: &std::path::Path,
        calculate_hashes: bool,
    ) -> Result<(Option<String>, Option<String>)> {
        use std::fs::File;
        use std::io::Write;
        use md5::Context as Md5Context;
        use sha2::{Sha256, Digest};

        let mut file = File::create(output_path)
            .with_context(|| format!("无法创建输出文件: {:?}", output_path))?;

        let mut remaining = size;
        let mut current_offset = offset;
        let buffer_size = 16 * 1024 * 1024;
        
        let mut md5 = calculate_hashes.then(Md5Context::new);
        let mut sha256 = calculate_hashes.then(Sha256::new);

        while remaining > 0 {
            let read_size = remaining.min(buffer_size as u64);
            let mut buffer = vec![0u8; read_size as usize];
            
            let bytes_read = reader.read_offset(current_offset, read_size, &mut buffer)?;
            
            if bytes_read == 0 {
                break;
            }

            buffer.truncate(bytes_read);
            file.write_all(&buffer)?;
            
            if let Some(md5) = &mut md5 {
                md5.consume(&buffer);
            }
            if let Some(sha256) = &mut sha256 {
                sha256.update(&buffer);
            }

            remaining -= bytes_read as u64;
            current_offset += bytes_read as u64;
        }

        file.flush()?;

        let md5_hash = md5.map(|d| hex::encode(d.compute().as_ref()));
        let sha256_hash = sha256.map(|d| hex::encode(d.finalize()));

        Ok((md5_hash, sha256_hash))
    }
}

use colored::*;
