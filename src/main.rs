use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;
use anyhow::{Result, Context};
use clap::Parser;
use colored::*;
use serde::Serialize;

mod file_types;
mod disk_reader;
mod carver;
mod progress;
mod cli;
mod fs_parser;

use cli::{Cli, Command, print_supported_types};
use disk_reader::create_reader;
use carver::{DiskCarver, CarverConfig, MatchResult, CarveStats};
use fs_parser::FileSystemType;
use progress::{
    print_header, print_disk_info, print_scan_summary, print_match_list,
    print_detailed_match, ProgressManager,
};

#[derive(Serialize)]
struct JsonReport {
    case_info: Option<String>,
    examiner: Option<String>,
    timestamp: String,
    image_path: String,
    image_format: String,
    total_size: u64,
    total_sectors: u64,
    scan_duration_secs: f64,
    processing_speed_bps: f64,
    total_matches: usize,
    successful_recoveries: usize,
    failed_recoveries: usize,
    matches_by_type: std::collections::HashMap<String, usize>,
    recovered_files: Vec<JsonFileEntry>,
}

#[derive(Serialize)]
struct JsonFileEntry {
    id: usize,
    file_type: String,
    description: String,
    extension: String,
    start_offset: u64,
    end_offset: u64,
    size: u64,
    sector_start: u64,
    sector_end: u64,
    recovered: bool,
    output_path: Option<String>,
    md5_hash: Option<String>,
    sha256_hash: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::ListTypes => {
            print_supported_types();
            Ok(())
        }
        Command::Info(args) => {
            print_header();
            
            let reader = create_reader(&args.image)
                .with_context(|| format!("无法打开镜像文件: {:?}", args.image))?;
            
            let disk_info = reader.get_disk_info();
            print_disk_info(disk_info);
            
            println!("{}", "✓ 镜像信息读取完成".green().bold());
            println!();
            
            Ok(())
        }
        Command::Report(args) => {
            generate_report(&args)?;
            Ok(())
        }
        Command::Scan(args) => {
            print_header();

            println!("{}", "初始化磁盘镜像...".cyan());
            let mut reader = create_reader(&args.image)
                .with_context(|| format!("无法打开镜像文件: {:?}", args.image))?;
            
            let disk_info = reader.get_disk_info().clone();
            print_disk_info(&disk_info);

            let selected_types = args.get_selected_types();
            if let Some(types) = &selected_types {
                println!("{}", format!("指定恢复文件类型: {}", types.iter()
                    .map(|t| t.get_signature().extension.to_uppercase())
                    .collect::<Vec<_>>()
                    .join(", ")).yellow());
                println!();
            }

            let mut config = CarverConfig::default();
            config.output_dir = args.output.clone();
            config.thread_count = args.threads.unwrap_or_else(num_cpus::get);
            config.chunk_size = (args.block_size_kb * 1024).max(1024 * 1024);
            config.calculate_hashes = !args.no_hash;
            config.sector_aligned = !args.no_sector_align;
            config.selected_types = selected_types;
            config.use_filesystem_metadata = !args.no_fs_metadata;
            config.fs_fallback = !args.no_fs_fallback;
            config.partition_offset = args.partition_offset;

            println!("{}", format!("配置参数: 线程数={}, 区块大小={}KB, 扇区对齐={}, 哈希计算={}, 文件系统元数据={}",
                config.thread_count,
                config.chunk_size / 1024,
                if config.sector_aligned { "是" } else { "否" },
                if config.calculate_hashes { "是" } else { "否" },
                if config.use_filesystem_metadata { "启用" } else { "禁用" }
            ).cyan());
            println!();

            let mut carver = DiskCarver::new(config.clone());
            
            let stats_arc = carver.stats.clone();
            let mut progress = ProgressManager::new(disk_info.clone(), stats_arc);
            progress.start_monitor();

            println!("{}", "开始扫描魔数匹配...".cyan().bold());
            let scan_start = Instant::now();
            
            let matches = carver.scan(&mut reader)
                .context("扫描过程中发生错误")?;
            
            progress.stop();
            let scan_duration = scan_start.elapsed();
            
            println!();
            println!("{}", format!("扫描完成，发现 {} 个匹配项，用时 {:?}", 
                matches.len(), scan_duration).green().bold());

            let stats = carver.get_stats();
            print_scan_summary(&stats, &stats.matches_by_type);

            if matches.is_empty() {
                println!("{}", "未找到任何匹配的文件".yellow());
                return Ok(());
            }

            if !args.scan_only {
                println!("{}", "开始雕刻恢复文件...".cyan().bold());
                println!();
                
                let carved = carver.carve_files(&mut reader)
                    .context("文件雕刻过程中发生错误")?;
                
                println!();
                println!("{}", format!("文件雕刻完成，成功恢复 {} 个，失败 {} 个",
                    carved.iter().filter(|m| m.recovered).count(),
                    carved.iter().filter(|m| !m.recovered).count()
                ).green().bold());

                print_match_list(&carved);

                if cli.verbose {
                    println!("\n{}", "详细恢复信息:".white().bold());
                    for (i, m) in carved.iter().enumerate() {
                        print_detailed_match(m, i + 1);
                    }
                }

                if let Some(csv_path) = &args.csv_report {
                    generate_csv_report(csv_path, &carved, &args.case_info, &args.examiner)
                        .with_context(|| format!("无法生成 CSV 报告: {:?}", csv_path))?;
                    println!("{}", format!("✓ CSV 报告已生成: {:?}", csv_path).green());
                }

                if let Some(json_path) = &args.json_report {
                    generate_json_report(
                        json_path, 
                        &carved, 
                        &stats,
                        &args, 
                        &disk_info
                    ).with_context(|| format!("无法生成 JSON 报告: {:?}", json_path))?;
                    println!("{}", format!("✓ JSON 报告已生成: {:?}", json_path).green());
                }
            } else {
                println!("{}", "--scan-only 模式，跳过文件恢复".yellow());
                print_match_list(&matches);
            }

            println!();
            println!("{}", "═".repeat(80).bright_black());
            println!("{}", "任务完成！".green().bold());
            println!("{}", "═".repeat(80).bright_black());
            println!();

            Ok(())
        }
    }
}

fn generate_csv_report(
    path: &std::path::Path,
    matches: &[MatchResult],
    case_info: &Option<String>,
    examiner: &Option<String>,
) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    if let Some(info) = case_info {
        writeln!(writer, "# 案件信息: {}", info)?;
    }
    if let Some(ex) = examiner {
        writeln!(writer, "# 审查员: {}", ex)?;
    }
    writeln!(writer, "# 生成时间: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))?;
    writeln!(writer)?;

    writeln!(writer, "序号,文件类型,描述,扩展名,起始偏移(HEX),结束偏移(HEX),大小(字节),起始扇区,结束扇区,恢复状态,输出路径,MD5,SHA256")?;

    for (i, m) in matches.iter().enumerate() {
        let sig = m.file_type.get_signature();
        let status = if m.recovered { "成功" } else { "失败" };
        let output = m.output_path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let md5 = m.md5_hash.as_deref().unwrap_or_default();
        let sha256 = m.sha256_hash.as_deref().unwrap_or_default();

        writeln!(writer,
            "{},{},{},{},0x{:016X},0x{:016X},{},{},{},{},{},{},{}",
            i + 1,
            sig.extension.to_uppercase(),
            sig.description,
            sig.extension,
            m.start_offset,
            m.end_offset,
            m.size,
            m.sector_start,
            m.sector_end,
            status,
            output,
            md5,
            sha256,
        )?;
    }

    writer.flush()?;
    Ok(())
}

fn generate_json_report(
    path: &std::path::Path,
    matches: &[MatchResult],
    stats: &CarveStats,
    args: &cli::ScanArgs,
    disk_info: &disk_reader::DiskInfo,
) -> Result<()> {
    let mut matches_by_type = std::collections::HashMap::new();
    for (ft, count) in &stats.matches_by_type {
        matches_by_type.insert(
            ft.get_signature().extension.to_uppercase(),
            *count,
        );
    }

    let recovered_files: Vec<JsonFileEntry> = matches.iter()
        .enumerate()
        .map(|(i, m)| {
            let sig = m.file_type.get_signature();
            JsonFileEntry {
                id: i + 1,
                file_type: sig.extension.to_uppercase(),
                description: sig.description.to_string(),
                extension: sig.extension.to_string(),
                start_offset: m.start_offset,
                end_offset: m.end_offset,
                size: m.size,
                sector_start: m.sector_start,
                sector_end: m.sector_end,
                recovered: m.recovered,
                output_path: m.output_path.as_ref().map(|p| p.display().to_string()),
                md5_hash: m.md5_hash.clone(),
                sha256_hash: m.sha256_hash.clone(),
            }
        })
        .collect();

    let report = JsonReport {
        case_info: args.case_info.clone(),
        examiner: args.examiner.clone(),
        timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        image_path: disk_info.path.display().to_string(),
        image_format: match disk_info.format {
            disk_reader::ImageFormat::RawDd => "Raw DD".to_string(),
            disk_reader::ImageFormat::E01 => "E01".to_string(),
        },
        total_size: disk_info.total_size,
        total_sectors: disk_info.total_sectors,
        scan_duration_secs: stats.elapsed_time.as_secs_f64(),
        processing_speed_bps: stats.bytes_per_second,
        total_matches: stats.total_matches,
        successful_recoveries: stats.successful_recoveries,
        failed_recoveries: stats.failed_recoveries,
        matches_by_type,
        recovered_files,
    };

    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, &report)?;
    Ok(())
}

fn generate_report(args: &cli::ReportArgs) -> Result<()> {
    use walkdir::WalkDir;
    use std::collections::HashMap;

    println!("\n{}", "生成司法鉴定报告...".cyan().bold());
    println!();

    let recovered_dir = &args.recovered_dir;
    if !recovered_dir.exists() {
        return Err(anyhow::anyhow!("恢复目录不存在: {:?}", recovered_dir));
    }

    let mut files: Vec<(String, u64, String, Option<String>, Option<String>)> = Vec::new();
    let mut type_counts: HashMap<String, usize> = HashMap::new();
    let mut total_size = 0u64;

    for entry in WalkDir::new(recovered_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if let Ok(metadata) = entry.metadata() {
            let size = metadata.len();
            total_size += size;

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("unknown")
                .to_string();
            
            *type_counts.entry(ext.clone()).or_insert(0) += 1;

            let (md5, sha256) = if !args.no_hash {
                match calculate_file_hashes(path) {
                    Ok((md5, sha256)) => (Some(md5), Some(sha256)),
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            };

            files.push((
                path.display().to_string(),
                size,
                ext,
                md5,
                sha256,
            ));
        }
    }

    match args.format {
        cli::ReportFormat::Text => {
            print_text_report(&files, &type_counts, total_size, args);
        }
        cli::ReportFormat::Csv => {
            let output_path = args.output.clone().unwrap_or_else(|| {
                recovered_dir.join("report.csv")
            });
            write_csv_report(&output_path, &files, args)?;
            println!("{}", format!("✓ CSV 报告已生成: {:?}", output_path).green());
        }
        cli::ReportFormat::Json => {
            let output_path = args.output.clone().unwrap_or_else(|| {
                recovered_dir.join("report.json")
            });
            write_json_report(&output_path, &files, &type_counts, total_size, args)?;
            println!("{}", format!("✓ JSON 报告已生成: {:?}", output_path).green());
        }
        cli::ReportFormat::Html => {
            let output_path = args.output.clone().unwrap_or_else(|| {
                recovered_dir.join("report.html")
            });
            write_html_report(&output_path, &files, &type_counts, total_size, args)?;
            println!("{}", format!("✓ HTML 报告已生成: {:?}", output_path).green());
        }
    }

    println!();
    Ok(())
}

fn calculate_file_hashes(path: &std::path::Path) -> Result<(String, String)> {
    use md5::Context as Md5Context;
    use sha2::{Sha256, Digest};
    use std::io::Read;

    let mut file = File::open(path)?;
    let mut buffer = vec![0u8; 16 * 1024 * 1024];
    let mut md5 = Md5Context::new();
    let mut sha256 = Sha256::new();

    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        md5.consume(&buffer[..bytes_read]);
        sha256.update(&buffer[..bytes_read]);
    }

    Ok((
        hex::encode(md5.compute().as_ref()),
        hex::encode(sha256.finalize()),
    ))
}

fn print_text_report(
    files: &[(String, u64, String, Option<String>, Option<String>)],
    type_counts: &std::collections::HashMap<String, usize>,
    total_size: u64,
    args: &cli::ReportArgs,
) {
    println!("{}", "═".repeat(80).bright_black());
    println!("{}", "                    ░▒▓█ 司法鉴定报告 █▓▒░".yellow().bold());
    println!("{}", "═".repeat(80).bright_black());
    
    if let Some(info) = &args.case_info {
        println!("{}: {}", "案件信息".cyan().bold(), info);
    }
    println!("{}: {}", "生成时间".cyan().bold(), 
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    println!("{}: {}", "恢复目录".cyan().bold(), args.recovered_dir.display());
    println!("{}: {}", "总文件数".cyan().bold(), files.len());
    println!("{}: {}", "总大小".cyan().bold(), format_size(total_size));
    println!();

    println!("{}", "按文件类型统计:".white().bold());
    let mut types: Vec<_> = type_counts.iter().collect();
    types.sort_by_key(|(k, _)| *k);
    for (ext, count) in types {
        println!("  {}: {} 个", ext.to_uppercase().magenta(), count);
    }
    println!();

    println!("{}", "文件列表:".white().bold());
    for (i, (path, size, _ext, md5, sha256)) in files.iter().enumerate() {
        println!("{:>3}. {:<30} {:>12}", 
            i + 1,
            std::path::Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .green(),
            format_size(*size).yellow()
        );
        if let Some(hash) = md5 {
            println!("     MD5:    {}", hash.blue());
        }
        if let Some(hash) = sha256 {
            println!("     SHA256: {}", hash.blue());
        }
        println!();
    }
}

fn write_csv_report(
    path: &std::path::Path,
    files: &[(String, u64, String, Option<String>, Option<String>)],
    args: &cli::ReportArgs,
) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    if let Some(info) = &args.case_info {
        writeln!(writer, "# 案件信息: {}", info)?;
    }
    writeln!(writer, "# 生成时间: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))?;
    writeln!(writer)?;

    writeln!(writer, "序号,文件名,路径,大小(字节),类型,MD5,SHA256")?;

    for (i, (path_str, size, ext, md5, sha256)) in files.iter().enumerate() {
        let filename = std::path::Path::new(path_str)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let md5 = md5.as_deref().unwrap_or_default();
        let sha256 = sha256.as_deref().unwrap_or_default();

        writeln!(writer,
            "{},{},{},{},{},{},{}",
            i + 1,
            filename,
            path_str,
            size,
            ext,
            md5,
            sha256,
        )?;
    }

    writer.flush()?;
    Ok(())
}

fn write_json_report(
    path: &std::path::Path,
    files: &[(String, u64, String, Option<String>, Option<String>)],
    type_counts: &std::collections::HashMap<String, usize>,
    total_size: u64,
    args: &cli::ReportArgs,
) -> Result<()> {
    #[derive(Serialize)]
    struct ReportFile {
        id: usize,
        filename: String,
        path: String,
        size: u64,
        extension: String,
        md5: Option<String>,
        sha256: Option<String>,
    }

    #[derive(Serialize)]
    struct Report {
        case_info: Option<String>,
        timestamp: String,
        recovered_directory: String,
        total_files: usize,
        total_size_bytes: u64,
        type_counts: std::collections::HashMap<String, usize>,
        files: Vec<ReportFile>,
    }

    let report_files: Vec<ReportFile> = files.iter()
        .enumerate()
        .map(|(i, (path_str, size, ext, md5, sha256))| {
            let filename = std::path::Path::new(path_str)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            ReportFile {
                id: i + 1,
                filename,
                path: path_str.clone(),
                size: *size,
                extension: ext.clone(),
                md5: md5.clone(),
                sha256: sha256.clone(),
            }
        })
        .collect();

    let report = Report {
        case_info: args.case_info.clone(),
        timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        recovered_directory: args.recovered_dir.display().to_string(),
        total_files: files.len(),
        total_size_bytes: total_size,
        type_counts: type_counts.clone(),
        files: report_files,
    };

    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, &report)?;
    Ok(())
}

fn write_html_report(
    path: &std::path::Path,
    files: &[(String, u64, String, Option<String>, Option<String>)],
    type_counts: &std::collections::HashMap<String, usize>,
    total_size: u64,
    args: &cli::ReportArgs,
) -> Result<()> {
    let file = File::create(path)?;
    let mut writer = BufWriter::new(file);

    writeln!(writer, "<!DOCTYPE html>")?;
    writeln!(writer, "<html lang=\"zh-CN\">")?;
    writeln!(writer, "<head>")?;
    writeln!(writer, "<meta charset=\"UTF-8\">")?;
    writeln!(writer, "<title>司法鉴定报告 - 磁盘数据恢复</title>")?;
    writeln!(writer, "<style>")?;
    writeln!(writer, "body {{ font-family: Arial, sans-serif; margin: 20px; background: #f5f5f5; }}")?;
    writeln!(writer, ".container {{ max-width: 1200px; margin: 0 auto; background: white; padding: 30px; border-radius: 10px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }}")?;
    writeln!(writer, "h1 {{ color: #2c3e50; border-bottom: 3px solid #3498db; padding-bottom: 10px; }}")?;
    writeln!(writer, "h2 {{ color: #34495e; margin-top: 30px; }}")?;
    writeln!(writer, "table {{ width: 100%; border-collapse: collapse; margin-top: 20px; }}")?;
    writeln!(writer, "th, td {{ padding: 12px; text-align: left; border-bottom: 1px solid #ddd; }}")?;
    writeln!(writer, "th {{ background: #3498db; color: white; }}")?;
    writeln!(writer, "tr:hover {{ background: #f8f9fa; }}")?;
    writeln!(writer, ".info-box {{ background: #e8f4f8; padding: 15px; border-left: 4px solid #3498db; margin: 20px 0; }}")?;
    writeln!(writer, ".stats {{ display: flex; gap: 20px; flex-wrap: wrap; }}")?;
    writeln!(writer, ".stat-item {{ background: #ecf0f1; padding: 15px; border-radius: 8px; flex: 1; min-width: 200px; }}")?;
    writeln!(writer, ".stat-value {{ font-size: 24px; font-weight: bold; color: #27ae60; }}")?;
    writeln!(writer, "code {{ background: #2c3e50; color: #2ecc71; padding: 2px 6px; border-radius: 3px; font-family: monospace; }}")?;
    writeln!(writer, "</style>")?;
    writeln!(writer, "</head>")?;
    writeln!(writer, "<body>")?;
    writeln!(writer, "<div class=\"container\">")?;
    writeln!(writer, "<h1>司法鉴定报告 - 磁盘数据恢复</h1>")?;

    writeln!(writer, "<div class=\"info-box\">")?;
    if let Some(info) = &args.case_info {
        writeln!(writer, "<p><strong>案件信息:</strong> {}</p>", info)?;
    }
    writeln!(writer, "<p><strong>生成时间:</strong> {}</p>", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))?;
    writeln!(writer, "<p><strong>恢复目录:</strong> <code>{}</code></p>", args.recovered_dir.display())?;
    writeln!(writer, "</div>")?;

    writeln!(writer, "<div class=\"stats\">")?;
    writeln!(writer, "<div class=\"stat-item\"><div>总文件数</div><div class=\"stat-value\">{}</div></div>", files.len())?;
    writeln!(writer, "<div class=\"stat-item\"><div>总大小</div><div class=\"stat-value\">{}</div></div>", format_size(total_size))?;
    writeln!(writer, "<div class=\"stat-item\"><div>文件类型数</div><div class=\"stat-value\">{}</div></div>", type_counts.len())?;
    writeln!(writer, "</div>")?;

    writeln!(writer, "<h2>按类型统计</h2>")?;
    writeln!(writer, "<table>")?;
    writeln!(writer, "<tr><th>文件类型</th><th>数量</th></tr>")?;
    let mut types: Vec<_> = type_counts.iter().collect();
    types.sort_by_key(|(k, _)| *k);
    for (ext, count) in types {
        writeln!(writer, "<tr><td>{}</td><td>{}</td></tr>", ext.to_uppercase(), count)?;
    }
    writeln!(writer, "</table>")?;

    writeln!(writer, "<h2>恢复文件列表</h2>")?;
    writeln!(writer, "<table>")?;
    writeln!(writer, "<tr><th>#</th><th>文件名</th><th>大小</th><th>类型</th><th>MD5</th><th>SHA256</th></tr>")?;

    for (i, (path_str, size, ext, md5, sha256)) in files.iter().enumerate() {
        let filename = std::path::Path::new(path_str)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let md5 = md5.as_deref().unwrap_or("-");
        let sha256 = sha256.as_deref().unwrap_or("-");

        writeln!(writer, "<tr>")?;
        writeln!(writer, "<td>{}</td>", i + 1)?;
        writeln!(writer, "<td>{}</td>", filename)?;
        writeln!(writer, "<td>{}</td>", format_size(*size))?;
        writeln!(writer, "<td>{}</td>", ext.to_uppercase())?;
        writeln!(writer, "<td><code>{}</code></td>", md5)?;
        writeln!(writer, "<td><code>{}</code></td>", sha256)?;
        writeln!(writer, "</tr>")?;
    }

    writeln!(writer, "</table>")?;
    writeln!(writer, "</div>")?;
    writeln!(writer, "</body>")?;
    writeln!(writer, "</html>")?;

    writer.flush()?;
    Ok(())
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
