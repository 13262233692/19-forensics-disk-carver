use anyhow::{Result, anyhow, Context};

use crate::disk_reader::{DiskImageReader, SECTOR_SIZE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileSystemType {
    Ntfs,
    Fat32,
    Fat16,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct DataRun {
    pub start_cluster: u64,
    pub cluster_count: u64,
}

#[derive(Debug, Clone)]
pub struct ClusterChain {
    pub runs: Vec<DataRun>,
    pub total_clusters: u64,
    pub total_bytes: u64,
    pub is_fragmented: bool,
}

impl ClusterChain {
    pub fn new() -> Self {
        ClusterChain {
            runs: Vec::new(),
            total_clusters: 0,
            total_bytes: 0,
            is_fragmented: false,
        }
    }

    pub fn add_run(&mut self, start_cluster: u64, cluster_count: u64, bytes_per_cluster: u64) {
        self.runs.push(DataRun {
            start_cluster,
            cluster_count,
        });
        self.total_clusters += cluster_count;
        self.total_bytes += cluster_count * bytes_per_cluster;
        self.is_fragmented = self.runs.len() > 1;
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

pub trait FileSystemParser {
    fn fs_type(&self) -> FileSystemType;
    fn cluster_size(&self) -> u64;
    fn partition_offset(&self) -> u64;
    fn build_cluster_chain(&self, reader: &mut dyn DiskImageReader, start_cluster: u64, max_clusters: u64) -> Result<ClusterChain>;
    fn find_file_by_offset(&self, reader: &mut dyn DiskImageReader, offset: u64, max_size: u64) -> Option<ClusterChain>;
}

pub fn detect_filesystem(reader: &mut dyn DiskImageReader, partition_offset: u64) -> FileSystemType {
    let mut boot_sector = vec![0u8; SECTOR_SIZE as usize];
    if reader.read_offset(partition_offset, SECTOR_SIZE, &mut boot_sector).is_err() {
        return FileSystemType::Unknown;
    }

    if boot_sector.len() >= 11 {
        let oem_name = &boot_sector[3..11];
        if oem_name == b"NTFS    " {
            return FileSystemType::Ntfs;
        }
    }

    if boot_sector.len() >= 90 {
        let fs_type = &boot_sector[82..90];
        if fs_type.starts_with(b"FAT32") {
            return FileSystemType::Fat32;
        }
        if fs_type.starts_with(b"FAT16") {
            return FileSystemType::Fat16;
        }
    }

    if boot_sector.len() >= 36 {
        let sectors_per_fat = u16::from_le_bytes([boot_sector[22], boot_sector[23]]) as u32;
        let root_entries = u16::from_le_bytes([boot_sector[17], boot_sector[18]]) as u32;
        let total_sectors_16 = u16::from_le_bytes([boot_sector[19], boot_sector[20]]) as u32;
        let total_sectors = if total_sectors_16 == 0 {
            u32::from_le_bytes([boot_sector[32], boot_sector[33], boot_sector[34], boot_sector[35]])
        } else {
            total_sectors_16 as u32
        };
        let bytes_per_sector = u16::from_le_bytes([boot_sector[11], boot_sector[12]]) as u32;
        let sectors_per_cluster = boot_sector[13] as u32;
        let fats = boot_sector[16] as u32;

        let root_dir_sectors = ((root_entries * 32) + (bytes_per_sector - 1)) / bytes_per_sector;
        let data_sectors = total_sectors - (boot_sector[14] as u32 + (fats * sectors_per_fat) + root_dir_sectors);
        let total_clusters = data_sectors / sectors_per_cluster;

        if total_clusters < 4085 {
        } else if total_clusters < 65525 {
            return FileSystemType::Fat16;
        } else {
            return FileSystemType::Fat32;
        }
    }

    FileSystemType::Unknown
}

pub fn create_filesystem_parser(
    reader: &mut dyn DiskImageReader,
    partition_offset: u64,
) -> Option<Box<dyn FileSystemParser>> {
    let fs_type = detect_filesystem(reader, partition_offset);
    match fs_type {
        FileSystemType::Ntfs => {
            NtfsParser::new(reader, partition_offset)
                .ok()
                .map(|p| Box::new(p) as Box<dyn FileSystemParser>)
        }
        FileSystemType::Fat32 => {
            Fat32Parser::new(reader, partition_offset)
                .ok()
                .map(|p| Box::new(p) as Box<dyn FileSystemParser>)
        }
        FileSystemType::Fat16 => {
            Fat16Parser::new(reader, partition_offset)
                .ok()
                .map(|p| Box::new(p) as Box<dyn FileSystemParser>)
        }
        FileSystemType::Unknown => None,
    }
}

pub struct NtfsParser {
    partition_offset: u64,
    bytes_per_cluster: u64,
    mft_start_cluster: u64,
    mft_record_size: u64,
    sectors_per_cluster: u32,
}

impl NtfsParser {
    pub fn new(reader: &mut dyn DiskImageReader, partition_offset: u64) -> Result<Self> {
        let mut boot_sector = vec![0u8; SECTOR_SIZE as usize];
        reader.read_offset(partition_offset, SECTOR_SIZE, &mut boot_sector)?;

        if &boot_sector[3..11] != b"NTFS    " {
            return Err(anyhow!("不是有效的 NTFS 引导扇区"));
        }

        let bytes_per_sector = u16::from_le_bytes([boot_sector[11], boot_sector[12]]) as u32;
        let sectors_per_cluster = boot_sector[13] as u32;
        let bytes_per_cluster = (bytes_per_sector * sectors_per_cluster) as u64;

        let mft_start_cluster = u64::from_le_bytes([
            boot_sector[48], boot_sector[49], boot_sector[50], boot_sector[51],
            boot_sector[52], boot_sector[53], boot_sector[54], boot_sector[55],
        ]);

        let mft_record_size_raw = boot_sector[64] as i8;
        let mft_record_size = if mft_record_size_raw > 0 {
            (mft_record_size_raw as u64) * bytes_per_cluster
        } else {
            1u64 << (-mft_record_size_raw as u64)
        };

        Ok(NtfsParser {
            partition_offset,
            bytes_per_cluster,
            mft_start_cluster,
            mft_record_size,
            sectors_per_cluster,
        })
    }

    fn mft_record_offset(&self, record_number: u64) -> u64 {
        self.partition_offset
            + self.mft_start_cluster * self.bytes_per_cluster
            + record_number * self.mft_record_size
    }

    pub fn read_mft_record(&self, reader: &mut dyn DiskImageReader, record_number: u64) -> Result<Vec<u8>> {
        let offset = self.mft_record_offset(record_number);
        let mut buffer = vec![0u8; self.mft_record_size as usize];
        reader.read_offset(offset, self.mft_record_size, &mut buffer)?;

        if &buffer[0..4] != b"FILE" {
            return Err(anyhow!("无效的 MFT 记录签名"));
        }

        Ok(buffer)
    }

    pub fn find_data_attributes(&self, record: &[u8]) -> Vec<(u64, Vec<u8>)> {
        let mut attrs = Vec::new();

        if record.len() < 0x14 {
            return attrs;
        }

        let attr_offset = u16::from_le_bytes([record[0x14], record[0x15]]) as usize;
        let mut offset = attr_offset;

        while offset < record.len() - 8 {
            let attr_type = u32::from_le_bytes([
                record[offset], record[offset + 1], record[offset + 2], record[offset + 3],
            ]);
            let attr_size = u32::from_le_bytes([
                record[offset + 4], record[offset + 5], record[offset + 6], record[offset + 7],
            ]) as usize;

            if attr_type == 0xFFFFFFFF || attr_size == 0 || offset + attr_size > record.len() {
                break;
            }

            if attr_type == 0x80 {
                let non_resident = record[offset + 0x08] != 0;
                if non_resident {
                    let data = &record[offset..offset + attr_size];
                    if data.len() >= 0x40 {
                        let data_size = u64::from_le_bytes([
                            data[0x30], data[0x31], data[0x32], data[0x33],
                            data[0x34], data[0x35], data[0x36], data[0x37],
                        ]);
                        attrs.push((data_size, data.to_vec()));
                    }
                } else {
                    if attr_size >= 0x18 {
                        let data_size = u32::from_le_bytes([
                            record[offset + 0x10], record[offset + 0x11],
                            record[offset + 0x12], record[offset + 0x13],
                        ]) as u64;
                        attrs.push((data_size, record[offset..offset + attr_size].to_vec()));
                    }
                }
            }

            offset += attr_size;
        }

        attrs
    }

    pub fn parse_data_runlist(&self, data_attr: &[u8]) -> Result<Vec<DataRun>> {
        let mut runs = Vec::new();

        if data_attr.len() < 0x40 {
            return Ok(runs);
        }

        let runlist_offset = u16::from_le_bytes([data_attr[0x20], data_attr[0x21]]) as usize;
        if runlist_offset >= data_attr.len() {
            return Ok(runs);
        }

        let mut offset = runlist_offset;
        let mut current_cluster: i64 = 0;

        while offset < data_attr.len() {
            let header = data_attr[offset];
            offset += 1;

            if header == 0 {
                break;
            }

            let length_size = (header & 0x0F) as usize;
            let offset_size = ((header >> 4) & 0x0F) as usize;

            if length_size == 0 || offset + length_size + offset_size > data_attr.len() {
                break;
            }

            let mut run_length: u64 = 0;
            for i in 0..length_size {
                run_length |= (data_attr[offset + i] as u64) << (i * 8);
            }
            offset += length_size;

            let mut run_offset: i64 = 0;
            if offset_size > 0 {
                for i in 0..offset_size {
                    run_offset |= (data_attr[offset + i] as i64) << (i * 8);
                }
                let shift = (8 - offset_size) * 8;
                run_offset = (run_offset << shift) >> shift;
                offset += offset_size;
            }

            current_cluster += run_offset;

            if current_cluster >= 0 && run_length > 0 {
                runs.push(DataRun {
                    start_cluster: current_cluster as u64,
                    cluster_count: run_length,
                });
            }
        }

        Ok(runs)
    }

    pub fn cluster_to_offset(&self, cluster: u64) -> u64 {
        self.partition_offset + cluster * self.bytes_per_cluster
    }

    pub fn offset_to_cluster(&self, offset: u64) -> u64 {
        if offset < self.partition_offset {
            0
        } else {
            (offset - self.partition_offset) / self.bytes_per_cluster
        }
    }

    pub fn find_mft_entry_by_offset(&self, reader: &mut dyn DiskImageReader, offset: u64, max_scan_records: u64) -> Option<u64> {
        let target_cluster = self.offset_to_cluster(offset);
        let target_within = offset - self.partition_offset - target_cluster * self.bytes_per_cluster;

        for mft_num in 0..max_scan_records {
            if let Ok(record) = self.read_mft_record(reader, mft_num) {
                let data_attrs = self.find_data_attributes(&record);
                for (data_size, data_attr) in &data_attrs {
                    if *data_size == 0 {
                        continue;
                    }
                    if let Ok(runs) = self.parse_data_runlist(&data_attr) {
                        for run in &runs {
                            if run.start_cluster <= target_cluster
                                && target_cluster < run.start_cluster + run.cluster_count
                            {
                                if target_cluster == run.start_cluster && target_within < 512 {
                                    return Some(mft_num);
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}

impl FileSystemParser for NtfsParser {
    fn fs_type(&self) -> FileSystemType {
        FileSystemType::Ntfs
    }

    fn cluster_size(&self) -> u64 {
        self.bytes_per_cluster
    }

    fn partition_offset(&self) -> u64 {
        self.partition_offset
    }

    fn build_cluster_chain(&self, _reader: &mut dyn DiskImageReader, start_cluster: u64, _max_clusters: u64) -> Result<ClusterChain> {
        let mut chain = ClusterChain::new();
        chain.add_run(start_cluster, 1, self.bytes_per_cluster);
        Ok(chain)
    }

    fn find_file_by_offset(&self, reader: &mut dyn DiskImageReader, offset: u64, max_size: u64) -> Option<ClusterChain> {
        let target_cluster = self.offset_to_cluster(offset);
        let max_scan = (max_size / self.mft_record_size).min(10000).max(100);

        for mft_num in 0..max_scan {
            if let Ok(record) = self.read_mft_record(reader, mft_num) {
                let data_attrs = self.find_data_attributes(&record);
                for (data_size, data_attr) in &data_attrs {
                    if *data_size == 0 || *data_size > max_size * 10 {
                        continue;
                    }
                    if let Ok(runs) = self.parse_data_runlist(&data_attr) {
                        if runs.is_empty() {
                            continue;
                        }
                        if let Some(first_run) = runs.first() {
                            if first_run.start_cluster == target_cluster {
                                let mut chain = ClusterChain::new();
                                for run in &runs {
                                    chain.add_run(run.start_cluster, run.cluster_count, self.bytes_per_cluster);
                                }
                                if *data_size < chain.total_bytes {
                                    chain.total_bytes = *data_size;
                                }
                                return Some(chain);
                            }
                        }
                    }
                }
            }
        }

        None
    }
}

pub struct Fat32Parser {
    partition_offset: u64,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    number_of_fats: u8,
    fat_size_sectors: u32,
    root_cluster: u32,
    total_clusters: u32,
    bytes_per_cluster: u64,
    fat_start_sector: u32,
    data_start_sector: u32,
}

impl Fat32Parser {
    pub fn new(reader: &mut dyn DiskImageReader, partition_offset: u64) -> Result<Self> {
        let mut boot_sector = vec![0u8; SECTOR_SIZE as usize];
        reader.read_offset(partition_offset, SECTOR_SIZE, &mut boot_sector)?;

        let bytes_per_sector = u16::from_le_bytes([boot_sector[11], boot_sector[12]]);
        let sectors_per_cluster = boot_sector[13];
        let reserved_sectors = u16::from_le_bytes([boot_sector[14], boot_sector[15]]);
        let number_of_fats = boot_sector[16];
        let fat_size_sectors = u32::from_le_bytes([
            boot_sector[36], boot_sector[37], boot_sector[38], boot_sector[39],
        ]);
        let root_cluster = u32::from_le_bytes([
            boot_sector[44], boot_sector[45], boot_sector[46], boot_sector[47],
        ]);
        let total_sectors_32 = u32::from_le_bytes([
            boot_sector[32], boot_sector[33], boot_sector[34], boot_sector[35],
        ]);

        let bytes_per_cluster = (bytes_per_sector as u64) * (sectors_per_cluster as u64);
        let fat_start_sector = reserved_sectors as u32;
        let total_fat_sectors = number_of_fats as u32 * fat_size_sectors;
        let data_start_sector = reserved_sectors as u32 + total_fat_sectors;
        let data_sectors = total_sectors_32 - data_start_sector;
        let total_clusters = data_sectors / sectors_per_cluster as u32;

        Ok(Fat32Parser {
            partition_offset,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            number_of_fats,
            fat_size_sectors,
            root_cluster,
            total_clusters,
            bytes_per_cluster,
            fat_start_sector,
            data_start_sector,
        })
    }

    pub fn cluster_to_offset(&self, cluster: u32) -> u64 {
        let cluster_sector = self.data_start_sector + (cluster - 2) * self.sectors_per_cluster as u32;
        self.partition_offset + cluster_sector as u64 * self.bytes_per_sector as u64
    }

    pub fn offset_to_cluster(&self, offset: u64) -> Option<u32> {
        if offset < self.partition_offset {
            return None;
        }
        let rel_offset = offset - self.partition_offset;
        let rel_sector = rel_offset / self.bytes_per_sector as u64;
        if rel_sector < self.data_start_sector as u64 {
            return None;
        }
        let data_sector = rel_sector - self.data_start_sector as u64;
        let cluster = (data_sector / self.sectors_per_cluster as u64) + 2;
        if cluster < 2 || cluster as u32 > self.total_clusters {
            return None;
        }
        Some(cluster as u32)
    }

    pub fn read_fat_entry(&self, reader: &mut dyn DiskImageReader, cluster: u32) -> Result<u32> {
        if cluster < 2 || cluster > self.total_clusters {
            return Err(anyhow!("无效的簇号: {}", cluster));
        }

        let fat_offset = cluster as u64 * 4;
        let fat_sector = self.fat_start_sector as u64 + (fat_offset / self.bytes_per_sector as u64);
        let sector_offset = fat_offset % self.bytes_per_sector as u64;

        let mut sector_buf = vec![0u8; self.bytes_per_sector as usize];
        let sector_offset_in_disk = self.partition_offset + fat_sector * self.bytes_per_sector as u64;
        reader.read_offset(sector_offset_in_disk, self.bytes_per_sector as u64, &mut sector_buf)?;

        if sector_offset as usize + 4 > sector_buf.len() {
            return Err(anyhow!("读取 FAT 条目越界"));
        }

        let entry = u32::from_le_bytes([
            sector_buf[sector_offset as usize],
            sector_buf[sector_offset as usize + 1],
            sector_buf[sector_offset as usize + 2],
            sector_buf[sector_offset as usize + 3],
        ]) & 0x0FFFFFFF;

        Ok(entry)
    }

    pub fn build_cluster_chain_from_fat(&self, reader: &mut dyn DiskImageReader, start_cluster: u32, max_clusters: u32) -> Result<ClusterChain> {
        let mut chain = ClusterChain::new();
        let mut current_cluster = start_cluster;
        let mut count = 0u32;
        let mut run_start = start_cluster;
        let mut run_length = 0u32;

        while count < max_clusters {
            let next = self.read_fat_entry(reader, current_cluster)?;

            let is_eoc = next >= 0x0FFFFFF8;
            let is_bad = next == 0x0FFFFFF7;

            if is_eoc || is_bad {
                if run_length > 0 {
                    chain.add_run(run_start as u64, run_length as u64, self.bytes_per_cluster);
                }
                if is_eoc {
                    break;
                } else {
                    return Err(anyhow!("遇到坏簇"));
                }
            }

            if next == current_cluster + 1 {
                run_length += 1;
            } else {
                if run_length > 0 {
                    chain.add_run(run_start as u64, run_length as u64, self.bytes_per_cluster);
                }
                run_start = next;
                run_length = 1;
            }

            current_cluster = next;
            count += 1;

            if next < 2 || next > self.total_clusters {
                if run_length > 0 {
                    chain.add_run(run_start as u64, run_length as u64, self.bytes_per_cluster);
                }
                break;
            }
        }

        Ok(chain)
    }
}

impl FileSystemParser for Fat32Parser {
    fn fs_type(&self) -> FileSystemType {
        FileSystemType::Fat32
    }

    fn cluster_size(&self) -> u64 {
        self.bytes_per_cluster
    }

    fn partition_offset(&self) -> u64 {
        self.partition_offset
    }

    fn build_cluster_chain(&self, reader: &mut dyn DiskImageReader, start_cluster: u64, max_clusters: u64) -> Result<ClusterChain> {
        self.build_cluster_chain_from_fat(reader, start_cluster as u32, max_clusters as u32)
    }

    fn find_file_by_offset(&self, reader: &mut dyn DiskImageReader, offset: u64, max_size: u64) -> Option<ClusterChain> {
        let start_cluster = self.offset_to_cluster(offset)?;
        let max_clusters = (max_size / self.bytes_per_cluster).max(1).min(self.total_clusters as u64) as u32;

        self.build_cluster_chain_from_fat(reader, start_cluster, max_clusters).ok()
    }
}

pub struct Fat16Parser {
    partition_offset: u64,
    bytes_per_sector: u16,
    sectors_per_cluster: u8,
    reserved_sectors: u16,
    number_of_fats: u8,
    fat_size_sectors: u16,
    root_entries: u16,
    total_sectors: u32,
    total_clusters: u16,
    bytes_per_cluster: u64,
    fat_start_sector: u16,
    root_dir_start_sector: u16,
    data_start_sector: u16,
}

impl Fat16Parser {
    pub fn new(reader: &mut dyn DiskImageReader, partition_offset: u64) -> Result<Self> {
        let mut boot_sector = vec![0u8; SECTOR_SIZE as usize];
        reader.read_offset(partition_offset, SECTOR_SIZE, &mut boot_sector)?;

        let bytes_per_sector = u16::from_le_bytes([boot_sector[11], boot_sector[12]]);
        let sectors_per_cluster = boot_sector[13];
        let reserved_sectors = u16::from_le_bytes([boot_sector[14], boot_sector[15]]);
        let number_of_fats = boot_sector[16];
        let fat_size_sectors = u16::from_le_bytes([boot_sector[22], boot_sector[23]]);
        let root_entries = u16::from_le_bytes([boot_sector[17], boot_sector[18]]);
        let total_sectors_16 = u16::from_le_bytes([boot_sector[19], boot_sector[20]]);
        let total_sectors = if total_sectors_16 == 0 {
            u32::from_le_bytes([boot_sector[32], boot_sector[33], boot_sector[34], boot_sector[35]])
        } else {
            total_sectors_16 as u32
        };

        let root_dir_sectors = ((root_entries as u32 * 32) + (bytes_per_sector as u32 - 1)) / bytes_per_sector as u32;
        let fat_start_sector = reserved_sectors;
        let total_fat_sectors = number_of_fats as u16 * fat_size_sectors;
        let root_dir_start_sector = fat_start_sector + total_fat_sectors;
        let data_start_sector = root_dir_start_sector + root_dir_sectors as u16;

        let data_sectors = total_sectors - data_start_sector as u32;
        let total_clusters = (data_sectors / sectors_per_cluster as u32) as u16;

        let bytes_per_cluster = (bytes_per_sector as u64) * (sectors_per_cluster as u64);

        Ok(Fat16Parser {
            partition_offset,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            number_of_fats,
            fat_size_sectors,
            root_entries,
            total_sectors,
            total_clusters,
            bytes_per_cluster,
            fat_start_sector,
            root_dir_start_sector,
            data_start_sector,
        })
    }

    pub fn cluster_to_offset(&self, cluster: u16) -> u64 {
        let cluster_sector = self.data_start_sector as u64 + (cluster as u64 - 2) * self.sectors_per_cluster as u64;
        self.partition_offset + cluster_sector * self.bytes_per_sector as u64
    }

    pub fn offset_to_cluster(&self, offset: u64) -> Option<u16> {
        if offset < self.partition_offset {
            return None;
        }
        let rel_offset = offset - self.partition_offset;
        let rel_sector = rel_offset / self.bytes_per_sector as u64;
        if rel_sector < self.data_start_sector as u64 {
            return None;
        }
        let data_sector = rel_sector - self.data_start_sector as u64;
        let cluster = (data_sector / self.sectors_per_cluster as u64) + 2;
        if cluster < 2 || cluster as u16 > self.total_clusters {
            return None;
        }
        Some(cluster as u16)
    }

    pub fn read_fat_entry(&self, reader: &mut dyn DiskImageReader, cluster: u16) -> Result<u16> {
        if cluster < 2 || cluster > self.total_clusters {
            return Err(anyhow!("无效的簇号: {}", cluster));
        }

        let fat_offset = cluster as u64 * 2;
        let fat_sector = self.fat_start_sector as u64 + (fat_offset / self.bytes_per_sector as u64);
        let sector_offset = fat_offset % self.bytes_per_sector as u64;

        let mut sector_buf = vec![0u8; self.bytes_per_sector as usize];
        let sector_offset_in_disk = self.partition_offset + fat_sector * self.bytes_per_sector as u64;
        reader.read_offset(sector_offset_in_disk, self.bytes_per_sector as u64, &mut sector_buf)?;

        if sector_offset as usize + 2 > sector_buf.len() {
            return Err(anyhow!("读取 FAT 条目越界"));
        }

        let entry = u16::from_le_bytes([
            sector_buf[sector_offset as usize],
            sector_buf[sector_offset as usize + 1],
        ]);

        Ok(entry)
    }

    pub fn build_cluster_chain_from_fat(&self, reader: &mut dyn DiskImageReader, start_cluster: u16, max_clusters: u16) -> Result<ClusterChain> {
        let mut chain = ClusterChain::new();
        let mut current_cluster = start_cluster;
        let mut count = 0u16;
        let mut run_start = start_cluster;
        let mut run_length = 0u16;

        while count < max_clusters {
            let next = self.read_fat_entry(reader, current_cluster)?;

            let is_eoc = next >= 0xFFF8;
            let is_bad = next == 0xFFF7;

            if is_eoc || is_bad {
                if run_length > 0 {
                    chain.add_run(run_start as u64, run_length as u64, self.bytes_per_cluster);
                }
                if is_eoc {
                    break;
                } else {
                    return Err(anyhow!("遇到坏簇"));
                }
            }

            if next == current_cluster + 1 {
                run_length += 1;
            } else {
                if run_length > 0 {
                    chain.add_run(run_start as u64, run_length as u64, self.bytes_per_cluster);
                }
                run_start = next;
                run_length = 1;
            }

            current_cluster = next;
            count += 1;

            if next < 2 || next > self.total_clusters {
                if run_length > 0 {
                    chain.add_run(run_start as u64, run_length as u64, self.bytes_per_cluster);
                }
                break;
            }
        }

        Ok(chain)
    }
}

impl FileSystemParser for Fat16Parser {
    fn fs_type(&self) -> FileSystemType {
        FileSystemType::Fat16
    }

    fn cluster_size(&self) -> u64 {
        self.bytes_per_cluster
    }

    fn partition_offset(&self) -> u64 {
        self.partition_offset
    }

    fn build_cluster_chain(&self, reader: &mut dyn DiskImageReader, start_cluster: u64, max_clusters: u64) -> Result<ClusterChain> {
        self.build_cluster_chain_from_fat(reader, start_cluster as u16, max_clusters as u16)
    }

    fn find_file_by_offset(&self, reader: &mut dyn DiskImageReader, offset: u64, max_size: u64) -> Option<ClusterChain> {
        let start_cluster = self.offset_to_cluster(offset)?;
        let max_clusters = (max_size / self.bytes_per_cluster).max(1).min(self.total_clusters as u64) as u16;

        self.build_cluster_chain_from_fat(reader, start_cluster, max_clusters).ok()
    }
}

#[derive(Debug, Clone)]
pub struct PartitionInfo {
    pub boot_indicator: u8,
    pub partition_type: u8,
    pub start_lba: u32,
    pub size_lba: u32,
}

pub fn detect_partitions(reader: &mut dyn DiskImageReader, disk_offset: u64) -> Vec<PartitionInfo> {
    let mut mbr = vec![0u8; 512];
    if reader.read_offset(disk_offset, 512, &mut mbr).is_err() {
        return Vec::new();
    }

    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Vec::new();
    }

    let mut partitions = Vec::new();

    for i in 0..4 {
        let offset = 0x01BE + i * 16;
        if offset + 16 > mbr.len() {
            break;
        }

        let boot_indicator = mbr[offset];
        let partition_type = mbr[offset + 4];
        let start_lba = u32::from_le_bytes([mbr[offset + 8], mbr[offset + 9], mbr[offset + 10], mbr[offset + 11]]);
        let size_lba = u32::from_le_bytes([mbr[offset + 12], mbr[offset + 13], mbr[offset + 14], mbr[offset + 15]]);

        if partition_type != 0 && size_lba > 0 {
            partitions.push(PartitionInfo {
                boot_indicator,
                partition_type,
                start_lba,
                size_lba,
            });
        }
    }

    partitions
}

pub struct SmartCarver {
    parser: Option<Box<dyn FileSystemParser + 'static>>,
    fs_type: FileSystemType,
    reader_snapshot: Option<std::sync::Arc<std::sync::Mutex<Box<dyn DiskImageReader>>>>,
}

impl SmartCarver {
    pub fn new(parser: Option<Box<dyn FileSystemParser + 'static>>) -> Self {
        let fs_type = parser.as_ref().map(|p| p.fs_type()).unwrap_or(FileSystemType::Unknown);
        SmartCarver {
            parser,
            fs_type,
            reader_snapshot: None,
        }
    }

    pub fn has_filesystem(&self) -> bool {
        self.parser.is_some()
    }

    pub fn fs_type(&self) -> FileSystemType {
        self.fs_type
    }

    pub fn try_rebuild_file(&self, reader: &mut dyn DiskImageReader, start_offset: u64, max_size: u64) -> Option<ClusterChain> {
        if let Some(ref parser) = self.parser {
            parser.find_file_by_offset(reader, start_offset, max_size)
        } else {
            None
        }
    }

    pub fn carve_with_cluster_chain(
        &self,
        reader: &mut dyn DiskImageReader,
        chain: &ClusterChain,
        output_path: &std::path::Path,
        calculate_hashes: bool,
    ) -> Result<(Option<String>, Option<String>)> {
        use std::fs::File;
        use std::io::Write;
        use md5::Context as Md5Context;
        use sha2::{Sha256, Digest};

        let parser = self.parser.as_ref().ok_or_else(|| anyhow!("没有文件系统解析器"))?;
        let cluster_size = parser.cluster_size();
        let partition_offset = parser.partition_offset();

        let mut file = File::create(output_path)
            .with_context(|| format!("无法创建输出文件: {:?}", output_path))?;

        let mut md5 = calculate_hashes.then(Md5Context::new);
        let mut sha256 = calculate_hashes.then(Sha256::new);

        let mut total_written = 0u64;
        let buffer_size = (16 * 1024 * 1024u64).min(cluster_size * 1024).max(4096);
        let mut buffer = vec![0u8; buffer_size as usize];

        for run in &chain.runs {
            let run_start_offset = partition_offset + run.start_cluster * cluster_size;
            let run_bytes = run.cluster_count * cluster_size;

            let mut run_remaining = run_bytes;
            let mut run_offset = run_start_offset;

            while run_remaining > 0 {
                let read_size = run_remaining.min(buffer_size);
                let bytes_to_read = read_size as usize;

                let bytes_read = reader.read_offset(run_offset, read_size, &mut buffer[..bytes_to_read])?;
                if bytes_read == 0 {
                    break;
                }

                let data = &buffer[..bytes_read];
                file.write_all(data)?;

                if let Some(ref mut m) = md5 {
                    m.consume(data);
                }
                if let Some(ref mut s) = sha256 {
                    s.update(data);
                }

                total_written += bytes_read as u64;
                run_remaining -= bytes_read as u64;
                run_offset += bytes_read as u64;

                if total_written >= chain.total_bytes {
                    break;
                }
            }

            if total_written >= chain.total_bytes {
                break;
            }
        }

        file.flush()?;

        let md5_hex = md5.map(|m| {
            let result = m.compute();
            format!("{:x}", result)
        });
        let sha256_hex = sha256.map(|s| {
            let result = s.finalize();
            format!("{:x}", result)
        });

        Ok((md5_hex, sha256_hex))
    }
}

impl Clone for SmartCarver {
    fn clone(&self) -> Self {
        SmartCarver {
            parser: None,
            fs_type: self.fs_type,
            reader_snapshot: None,
        }
    }
}
