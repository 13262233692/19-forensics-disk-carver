use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use colored::*;
use indicatif::{ProgressBar, ProgressStyle, MultiProgress};
use comfy_table::{Table, Cell, Color, Attribute, ContentArrangement};
use parking_lot::Mutex;

use crate::carver::CarveStats;
use crate::disk_reader::DiskInfo;
use crate::file_types::FileType;

pub struct ProgressManager {
    multi_progress: MultiProgress,
    main_progress: ProgressBar,
    stats: Arc<Mutex<CarveStats>>,
    disk_info: DiskInfo,
    stop_flag: Arc<Mutex<bool>>,
    monitor_thread: Option<thread::JoinHandle<()>>,
    start_time: Instant,
}

impl ProgressManager {
    pub fn new(disk_info: DiskInfo, stats: Arc<Mutex<CarveStats>>) -> Self {
        let multi_progress = MultiProgress::new();
        
        let total_size = disk_info.total_size;
        
        let main_progress = ProgressBar::new(total_size);
        main_progress.set_style(
            ProgressStyle::default_bar()
                .template(&format!(
                    "{} {}",
                    "🔍 扫描进度:".cyan().bold(),
                    "{bar:50.cyan/blue} {pos:>7}/{len:7} [{percent:>3}%] {msg}"
                ))
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏  ")
        );
        
        let main_progress = multi_progress.add(main_progress);
        
        Self {
            multi_progress,
            main_progress,
            stats,
            disk_info,
            stop_flag: Arc::new(Mutex::new(false)),
            monitor_thread: None,
            start_time: Instant::now(),
        }
    }

    pub fn start_monitor(&mut self) {
        let stats = self.stats.clone();
        let stop_flag = self.stop_flag.clone();
        let main_progress = self.main_progress.clone();
        let total_size = self.disk_info.total_size;

        self.monitor_thread = Some(thread::spawn(move || {
            while !*stop_flag.lock() {
                {
                    let stats_lock = stats.lock();
                    let processed = stats_lock.total_bytes_processed;
                    let matches = stats_lock.total_matches;
                    let speed = format_speed(stats_lock.bytes_per_second);
                    let elapsed = format_duration(stats_lock.elapsed_time);
                    
                    main_progress.set_position(processed.min(total_size));
                    main_progress.set_message(format!(
                        "| 匹配: {} | 速度: {} | 用时: {}",
                        matches.to_string().yellow().bold(),
                        speed.green().bold(),
                        elapsed.blue().bold()
                    ));
                }
                
                thread::sleep(Duration::from_millis(100));
            }
        }));
    }

    pub fn stop(&mut self) {
        *self.stop_flag.lock() = true;
        if let Some(handle) = self.monitor_thread.take() {
            handle.join().ok();
        }
        self.main_progress.finish_with_message("✓ 扫描完成".green().bold().to_string());
    }

    pub fn update(&self, processed: u64) {
        self.main_progress.set_position(processed.min(self.disk_info.total_size));
    }

    pub fn finish(&self) {
        self.main_progress.finish_with_message("✓ 完成".green().bold().to_string());
    }
}

impl Drop for ProgressManager {
    fn drop(&mut self) {
        *self.stop_flag.lock() = true;
        if let Some(handle) = self.monitor_thread.take() {
            handle.join().ok();
        }
    }
}

pub fn print_header() {
    println!("\n{}", "═".repeat(80).bright_black());
    println!("{}", 
        " ██████╗ ██╗███████╗██╗  ██╗     ██████╗ █████╗ ██████╗ ██╗   ██╗███████╗██████╗ ".cyan().bold()
    );
    println!("{}", 
        " ██╔══██╗██║██╔════╝██║ ██╔╝    ██╔════╝██╔══██╗██╔══██╗██║   ██║██╔════╝██╔══██╗".cyan().bold()
    );
    println!("{}", 
        " ██║  ██║██║███████╗█████╔╝     ██║     ███████║██████╔╝██║   ██║█████╗  ██████╔╝".cyan().bold()
    );
    println!("{}", 
        " ██║  ██║██║╚════██║██╔═██╗     ██║     ██╔══██║██╔══██╗╚██╗ ██╔╝██╔══╝  ██╔══██╗".cyan().bold()
    );
    println!("{}", 
        " ██████╔╝██║███████║██║  ██╗    ╚██████╗██║  ██║██║  ██║ ╚████╔╝ ███████╗██║  ██║".cyan().bold()
    );
    println!("{}", 
        " ╚═════╝ ╚═╝╚══════╝╚═╝  ╚═╝     ╚═════╝╚═╝  ╚═╝╚═╝  ╚═╝  ╚═══╝  ╚══════╝╚═╝  ╚═╝".cyan().bold()
    );
    println!("{}", "═".repeat(80).bright_black());
    println!("{}", 
        "         跨平台磁盘数据恢复工具 - 为网络警察和取证专家设计".yellow().bold()
    );
    println!("{}", "═".repeat(80).bright_black());
    println!();
}

pub fn print_disk_info(disk_info: &DiskInfo) {
    println!("{}", "📋 磁盘镜像信息:".white().bold());
    
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("属性").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("值").add_attribute(Attribute::Bold).fg(Color::Yellow),
    ]);

    let format_str = match disk_info.format {
        crate::disk_reader::ImageFormat::RawDd => "原始镜像 (.dd/.img)",
        crate::disk_reader::ImageFormat::E01 => "EnCase 证据镜像 (.e01)",
    };

    table.add_row(vec![
        Cell::new("文件路径"),
        Cell::new(disk_info.path.display().to_string()).fg(Color::Green),
    ]);
    table.add_row(vec![
        Cell::new("镜像格式"),
        Cell::new(format_str).fg(Color::Magenta),
    ]);
    table.add_row(vec![
        Cell::new("总大小"),
        Cell::new(format_size(disk_info.total_size)).fg(Color::Yellow),
    ]);
    table.add_row(vec![
        Cell::new("扇区大小"),
        Cell::new(format!("{} 字节", disk_info.sector_size)),
    ]);
    table.add_row(vec![
        Cell::new("总扇区数"),
        Cell::new(format!("{} (0x{:X})", disk_info.total_sectors, disk_info.total_sectors)),
    ]);

    println!("{table}");
    println!();
}

pub fn print_scan_summary(stats: &CarveStats, matches_by_type: &HashMap<FileType, usize>) {
    println!("\n{}", "📊 扫描结果统计:".white().bold());
    
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("文件类型").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("描述").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("匹配数量").add_attribute(Attribute::Bold).fg(Color::Yellow),
        Cell::new("文件类型").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("描述").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("匹配数量").add_attribute(Attribute::Bold).fg(Color::Yellow),
    ]);

    let all_types = FileType::all_types();
    let mut rows: Vec<(FileType, usize)> = all_types.iter()
        .map(|&ft| (ft, *matches_by_type.get(&ft).unwrap_or(&0)))
        .filter(|(_, count)| *count > 0)
        .collect();
    
    rows.sort_by_key(|(ft, _)| ft.get_signature().description.to_string());

    for chunk in rows.chunks(2) {
        let mut row_cells = Vec::new();
        
        for (ft, count) in chunk {
            let sig = ft.get_signature();
            row_cells.push(Cell::new(sig.extension.to_uppercase()).fg(Color::Magenta));
            row_cells.push(Cell::new(sig.description));
            row_cells.push(Cell::new(count.to_string()).fg(
                if *count > 0 { Color::Green } else { Color::DarkGrey }
            ));
        }
        
        while row_cells.len() < 6 {
            row_cells.push(Cell::new(""));
        }
        
        table.add_row(row_cells);
    }

    println!("{table}");
    println!();

    let mut summary_table = Table::new();
    summary_table.set_content_arrangement(ContentArrangement::Dynamic);
    summary_table.set_header(vec![
        Cell::new("统计项").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("数值").add_attribute(Attribute::Bold).fg(Color::Yellow),
    ]);

    let total_matched_files: usize = matches_by_type.values().sum();
    
    summary_table.add_row(vec![
        Cell::new("总匹配数"),
        Cell::new(total_matched_files.to_string()).fg(Color::Green).add_attribute(Attribute::Bold),
    ]);
    summary_table.add_row(vec![
        Cell::new("处理数据总量"),
        Cell::new(format_size(stats.total_bytes_processed)).fg(Color::Yellow),
    ]);
    summary_table.add_row(vec![
        Cell::new("扫描用时"),
        Cell::new(format_duration(stats.elapsed_time)).fg(Color::Blue),
    ]);
    summary_table.add_row(vec![
        Cell::new("平均处理速度"),
        Cell::new(format_speed(stats.bytes_per_second)).fg(Color::Green),
    ]);
    summary_table.add_row(vec![
        Cell::new("成功恢复"),
        Cell::new(stats.successful_recoveries.to_string()).fg(Color::Green).add_attribute(Attribute::Bold),
    ]);
    summary_table.add_row(vec![
        Cell::new("恢复失败"),
        Cell::new(stats.failed_recoveries.to_string()).fg(Color::Red),
    ]);

    if stats.detected_fs != crate::fs_parser::FileSystemType::Unknown {
        let fs_name = match stats.detected_fs {
            crate::fs_parser::FileSystemType::Ntfs => "NTFS",
            crate::fs_parser::FileSystemType::Fat32 => "FAT32",
            crate::fs_parser::FileSystemType::Fat16 => "FAT16",
            crate::fs_parser::FileSystemType::Unknown => "未知",
        };
        summary_table.add_row(vec![
            Cell::new("检测文件系统"),
            Cell::new(fs_name).fg(Color::Magenta).add_attribute(Attribute::Bold),
        ]);
        summary_table.add_row(vec![
            Cell::new("元数据重建"),
            Cell::new(stats.fs_rebuilt_count.to_string()).fg(Color::Cyan),
        ]);
        summary_table.add_row(vec![
            Cell::new("碎片化文件"),
            Cell::new(stats.fragmented_count.to_string()).fg(Color::Yellow),
        ]);
        summary_table.add_row(vec![
            Cell::new("连续文件"),
            Cell::new(stats.contiguous_count.to_string()).fg(Color::Green),
        ]);
    }

    println!("{summary_table}");
    println!();
}

pub fn print_carving_progress(current: usize, total: usize, file_type: &str, filename: &str) {
    let percent = (current as f64 / total as f64) * 100.0;
    println!(
        "{} [{:>3}/{:>3}] {:>5.1}% {} -> {}",
        "🔨".yellow(),
        current,
        total,
        percent,
        file_type.magenta().bold(),
        filename.green()
    );
}

pub fn print_match_list(matches: &[crate::carver::MatchResult]) {
    println!("\n{}", "📋 恢复文件列表:".white().bold());
    
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("#").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("类型").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("起始扇区").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("结束扇区").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("大小").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("状态").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("输出文件").add_attribute(Attribute::Bold).fg(Color::Cyan),
    ]);

    for (i, m) in matches.iter().enumerate() {
        let sig = m.file_type.get_signature();
        let status = if m.recovered {
            "✓ 成功".green().to_string()
        } else {
            "✗ 失败".red().to_string()
        };
        
        let output_file = m.output_path
            .as_ref()
            .map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string())
            .unwrap_or_else(|| "-".to_string());

        table.add_row(vec![
            Cell::new((i + 1).to_string()).fg(Color::Yellow),
            Cell::new(sig.extension.to_uppercase()).fg(Color::Magenta),
            Cell::new(format!("{} (0x{:X})", m.sector_start, m.sector_start)),
            Cell::new(format!("{} (0x{:X})", m.sector_end, m.sector_end)),
            Cell::new(format_size(m.size)).fg(Color::Yellow),
            Cell::new(status),
            Cell::new(output_file).fg(Color::Green),
        ]);
    }

    println!("{table}");
    println!();
}

pub fn print_detailed_match(m: &crate::carver::MatchResult, index: usize) {
    let sig = m.file_type.get_signature();
    
    println!("{}", "━".repeat(60).bright_black());
    println!("{} {}", "📄 文件 #".white().bold(), index.to_string().yellow().bold());
    println!("{}", "━".repeat(60).bright_black());
    
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_width(60);
    
    table.add_row(vec![
        Cell::new("文件类型").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new(format!("{} ({})", sig.description, sig.extension.to_uppercase())).fg(Color::Magenta),
    ]);
    table.add_row(vec![
        Cell::new("起始偏移").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new(format!("0x{:016X}", m.start_offset)).fg(Color::Yellow),
    ]);
    table.add_row(vec![
        Cell::new("结束偏移").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new(format!("0x{:016X}", m.end_offset)).fg(Color::Yellow),
    ]);
    table.add_row(vec![
        Cell::new("文件大小").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new(format_size(m.size)).fg(Color::Green),
    ]);
    table.add_row(vec![
        Cell::new("扇区范围").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new(format!("{} - {}", m.sector_start, m.sector_end)),
    ]);
    table.add_row(vec![
        Cell::new("恢复状态").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new(if m.recovered { "✓ 成功恢复" } else { "✗ 恢复失败" }).fg(
            if m.recovered { Color::Green } else { Color::Red }
        ),
    ]);

    if let Some(path) = &m.output_path {
        table.add_row(vec![
            Cell::new("输出路径").add_attribute(Attribute::Bold).fg(Color::Cyan),
            Cell::new(path.display().to_string()).fg(Color::Green),
        ]);
    }

    if let Some(md5) = &m.md5_hash {
        table.add_row(vec![
            Cell::new("MD5 哈希").add_attribute(Attribute::Bold).fg(Color::Cyan),
            Cell::new(md5).fg(Color::Blue),
        ]);
    }

    if let Some(sha256) = &m.sha256_hash {
        table.add_row(vec![
            Cell::new("SHA256 哈希").add_attribute(Attribute::Bold).fg(Color::Cyan),
            Cell::new(sha256).fg(Color::Blue),
        ]);
    }

    println!("{table}");
    println!();
}

pub fn print_report_header(case_info: Option<&str>) {
    println!("{}", "═".repeat(80).bright_black());
    println!("{}", 
        "                    ░▒▓█ 司法鉴定报告 █▓▒░".yellow().bold()
    );
    println!("{}", "═".repeat(80).bright_black());
    
    if let Some(info) = case_info {
        println!("{}: {}", "案件信息".cyan().bold(), info);
    }
    println!("{}: {}", "生成时间".cyan().bold(), 
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    println!();
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} 字节", bytes)
    }
}

fn format_speed(bytes_per_second: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    if bytes_per_second >= GB {
        format!("{:.2} GB/s", bytes_per_second / GB)
    } else if bytes_per_second >= MB {
        format!("{:.2} MB/s", bytes_per_second / MB)
    } else if bytes_per_second >= KB {
        format!("{:.2} KB/s", bytes_per_second / KB)
    } else {
        format!("{:.0} 字节/秒", bytes_per_second)
    }
}

fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    let millis = duration.subsec_millis();

    if hours > 0 {
        format!("{}小时{}分{}秒", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}分{}秒", minutes, secs)
    } else if secs > 0 {
        format!("{}.{:03}秒", secs, millis)
    } else {
        format!("{}毫秒", duration.as_millis())
    }
}
