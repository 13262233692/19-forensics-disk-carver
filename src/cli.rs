use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use crate::file_types::FileType;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "disk-carver",
    version = "1.0.0",
    author = "Forensics Team",
    about = "跨平台磁盘数据恢复工具 - 为网络警察和取证专家设计",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[arg(short, long, global = true, help = "显示详细输出")]
    pub verbose: bool,

    #[arg(short, long, global = true, help = "安静模式，只显示错误信息")]
    pub quiet: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    #[command(about = "扫描磁盘镜像并恢复文件", alias = "s")]
    Scan(ScanArgs),
    
    #[command(about = "列出支持的文件类型", alias = "lt")]
    ListTypes,
    
    #[command(about = "显示镜像信息", alias = "info")]
    Info(InfoArgs),
    
    #[command(about = "生成司法鉴定报告", alias = "rep")]
    Report(ReportArgs),
}

#[derive(Parser, Debug, Clone)]
pub struct ScanArgs {
    #[arg(help = "磁盘镜像文件路径 (.dd, .e01 等)")]
    pub image: PathBuf,

    #[arg(short, long, help = "输出目录", default_value = "./recovered")]
    pub output: PathBuf,

    #[arg(
        short = 't',
        long = "type",
        help = "指定要恢复的文件类型（可多次指定）",
        value_delimiter = ',',
        num_args = 1..
    )]
    pub file_types: Vec<String>,

    #[arg(short, long, help = "线程数量 (默认使用所有 CPU 核心数)")]
    pub threads: Option<usize>,

    #[arg(
        short = 'b',
        long = "block-size",
        help = "读取区块大小 (KB)",
        default_value_t = 65536
    )]
    pub block_size_kb: usize,

    #[arg(long, help = "禁用扇区对齐检查")]
    pub no_sector_align: bool,

    #[arg(long, help = "跳过 MD5/SHA256 哈希计算")]
    pub no_hash: bool,

    #[arg(
        long = "min-size",
        help = "最小文件大小过滤 (字节)",
        default_value_t = 100
    )]
    pub min_size: u64,

    #[arg(
        long = "max-size",
        help = "最大文件大小过滤 (字节)",
        default_value_t = 10 * 1024 * 1024 * 1024
    )]
    pub max_size: u64,

    #[arg(long, help = "生成 CSV 格式的报告文件")]
    pub csv_report: Option<PathBuf>,

    #[arg(long, help = "生成 JSON 格式的报告文件")]
    pub json_report: Option<PathBuf>,

    #[arg(long, help = "案件信息，用于司法鉴定报告")]
    pub case_info: Option<String>,

    #[arg(long, help = "审查员姓名，用于司法鉴定报告")]
    pub examiner: Option<String>,

    #[arg(long, help = "仅扫描，不实际恢复文件")]
    pub scan_only: bool,

    #[arg(long, help = "禁用文件系统元数据辅助雕刻（仅使用魔数匹配）")]
    pub no_fs_metadata: bool,

    #[arg(long, help = "禁用文件系统元数据失败时的回退雕刻")]
    pub no_fs_fallback: bool,

    #[arg(long, help = "分区偏移量（字节），用于非第一个分区的镜像", default_value_t = 0)]
    pub partition_offset: u64,
}

#[derive(Parser, Debug, Clone)]
pub struct InfoArgs {
    #[arg(help = "磁盘镜像文件路径")]
    pub image: PathBuf,
}

#[derive(Parser, Debug, Clone)]
pub struct ReportArgs {
    #[arg(help = "恢复结果的目录")]
    pub recovered_dir: PathBuf,

    #[arg(short, long, help = "输出报告文件路径")]
    pub output: Option<PathBuf>,

    #[arg(short, long, help = "案件信息")]
    pub case_info: Option<String>,

    #[arg(long, help = "输出格式", value_enum, default_value_t = ReportFormat::Text)]
    pub format: ReportFormat,

    #[arg(long, help = "不计算文件哈希")]
    pub no_hash: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Text,
    Csv,
    Json,
    Html,
}

impl ScanArgs {
    pub fn get_selected_types(&self) -> Option<Vec<FileType>> {
        if self.file_types.is_empty() {
            return None;
        }

        let mut types = Vec::new();
        for t in &self.file_types {
            if let Some(ft) = parse_file_type(t) {
                types.push(ft);
            }
        }

        if types.is_empty() {
            None
        } else {
            Some(types)
        }
    }
}

fn parse_file_type(input: &str) -> Option<FileType> {
    let input_lower = input.to_lowercase();
    match input_lower.as_str() {
        "jpg" | "jpeg" => Some(FileType::Jpeg),
        "png" => Some(FileType::Png),
        "gif" => Some(FileType::Gif),
        "pdf" => Some(FileType::Pdf),
        "zip" => Some(FileType::Zip),
        "rar" => Some(FileType::Rar),
        "7z" | "7zip" | "sevenzip" => Some(FileType::SevenZip),
        "tar" => Some(FileType::Tar),
        "gz" | "gzip" => Some(FileType::Gzip),
        "bz2" | "bzip2" => Some(FileType::Bzip2),
        "xz" => Some(FileType::Xz),
        "mpeg" | "mpg" => Some(FileType::Mpeg),
        "avi" => Some(FileType::Avi),
        "mp4" => Some(FileType::Mp4),
        "mp3" => Some(FileType::Mp3),
        "wav" => Some(FileType::Wav),
        "bmp" => Some(FileType::Bmp),
        "tiff" | "tif" => Some(FileType::Tiff),
        "rtf" => Some(FileType::Rtf),
        "exe" | "pe" => Some(FileType::Exe),
        "elf" => Some(FileType::Elf),
        "dmg" => Some(FileType::Dmg),
        "iso" => Some(FileType::Iso),
        "doc" => Some(FileType::Doc),
        "xls" => Some(FileType::Xls),
        "ppt" => Some(FileType::Ppt),
        "docx" => Some(FileType::Docx),
        "xlsx" => Some(FileType::Xlsx),
        "pptx" => Some(FileType::Pptx),
        "sqlite" | "db" => Some(FileType::Sqlite),
        _ => None,
    }
}

pub fn print_supported_types() {
    use comfy_table::{Table, Cell, Color, Attribute, ContentArrangement};
    use colored::*;

    println!("\n{}", "📋 支持的文件类型列表:".white().bold());
    println!();
    
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(vec![
        Cell::new("扩展名").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("描述").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("头部魔数").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("尾部魔数").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("最小大小").add_attribute(Attribute::Bold).fg(Color::Cyan),
        Cell::new("最大大小").add_attribute(Attribute::Bold).fg(Color::Cyan),
    ]);

    for ft in FileType::all_types() {
        let sig = ft.get_signature();
        
        let header_hex: String = sig.header
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" ");
        
        let footer_hex = sig.footer
            .map(|f| {
                f.iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| "-".to_string());

        table.add_row(vec![
            Cell::new(sig.extension.to_uppercase()).fg(Color::Magenta),
            Cell::new(sig.description),
            Cell::new(header_hex).fg(Color::Yellow),
            Cell::new(footer_hex).fg(Color::Yellow),
            Cell::new(format_size(sig.min_size)),
            Cell::new(format_size(sig.max_size)),
        ]);
    }

    println!("{table}");
    println!();
    println!("{}", format!("共支持 {} 种文件类型", FileType::all_types().len()).cyan());
    println!();
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.0} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes / KB)
    } else {
        format!("{} 字节", bytes)
    }
}
