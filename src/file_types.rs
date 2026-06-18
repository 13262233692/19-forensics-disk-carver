use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileType {
    Jpeg,
    Png,
    Gif,
    Pdf,
    Zip,
    Rar,
    SevenZip,
    Tar,
    Gzip,
    Bzip2,
    Xz,
    Mpeg,
    Avi,
    Mp4,
    Mp3,
    Wav,
    Bmp,
    Tiff,
    Rtf,
    Exe,
    Elf,
    Dmg,
    Iso,
    Doc,
    Xls,
    Ppt,
    Docx,
    Xlsx,
    Pptx,
    Sqlite,
}

#[derive(Debug, Clone)]
pub struct FileSignature {
    pub file_type: FileType,
    pub extension: &'static str,
    pub description: &'static str,
    pub header: &'static [u8],
    pub footer: Option<&'static [u8]>,
    pub min_size: u64,
    pub max_size: u64,
}

impl FileType {
    pub fn all_types() -> &'static [FileType] {
        &[
            FileType::Jpeg,
            FileType::Png,
            FileType::Gif,
            FileType::Pdf,
            FileType::Zip,
            FileType::Rar,
            FileType::SevenZip,
            FileType::Tar,
            FileType::Gzip,
            FileType::Bzip2,
            FileType::Xz,
            FileType::Mpeg,
            FileType::Avi,
            FileType::Mp4,
            FileType::Mp3,
            FileType::Wav,
            FileType::Bmp,
            FileType::Tiff,
            FileType::Rtf,
            FileType::Exe,
            FileType::Elf,
            FileType::Dmg,
            FileType::Iso,
            FileType::Doc,
            FileType::Xls,
            FileType::Ppt,
            FileType::Docx,
            FileType::Xlsx,
            FileType::Pptx,
            FileType::Sqlite,
        ]
    }

    pub fn get_signature(&self) -> FileSignature {
        match self {
            FileType::Jpeg => FileSignature {
                file_type: *self,
                extension: "jpg",
                description: "JPEG 图像",
                header: &[0xFF, 0xD8, 0xFF],
                footer: Some(&[0xFF, 0xD9]),
                min_size: 100,
                max_size: 100 * 1024 * 1024,
            },
            FileType::Png => FileSignature {
                file_type: *self,
                extension: "png",
                description: "PNG 图像",
                header: &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
                footer: Some(&[0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82]),
                min_size: 100,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Gif => FileSignature {
                file_type: *self,
                extension: "gif",
                description: "GIF 图像",
                header: &[0x47, 0x49, 0x46, 0x38],
                footer: Some(&[0x00, 0x3B]),
                min_size: 50,
                max_size: 50 * 1024 * 1024,
            },
            FileType::Pdf => FileSignature {
                file_type: *self,
                extension: "pdf",
                description: "PDF 文档",
                header: &[0x25, 0x50, 0x44, 0x46],
                footer: Some(&[0x25, 0x25, 0x45, 0x4F, 0x46]),
                min_size: 100,
                max_size: 2 * 1024 * 1024 * 1024,
            },
            FileType::Zip => FileSignature {
                file_type: *self,
                extension: "zip",
                description: "ZIP 压缩包",
                header: &[0x50, 0x4B, 0x03, 0x04],
                footer: Some(&[0x50, 0x4B, 0x05, 0x06]),
                min_size: 100,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::Rar => FileSignature {
                file_type: *self,
                extension: "rar",
                description: "RAR 压缩包",
                header: &[0x52, 0x61, 0x72, 0x21, 0x1A, 0x07, 0x00],
                footer: None,
                min_size: 100,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::SevenZip => FileSignature {
                file_type: *self,
                extension: "7z",
                description: "7-Zip 压缩包",
                header: &[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C],
                footer: None,
                min_size: 100,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::Tar => FileSignature {
                file_type: *self,
                extension: "tar",
                description: "TAR 归档",
                header: &[0x75, 0x73, 0x74, 0x61, 0x72],
                footer: None,
                min_size: 512,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::Gzip => FileSignature {
                file_type: *self,
                extension: "gz",
                description: "GZIP 压缩",
                header: &[0x1F, 0x8B, 0x08],
                footer: None,
                min_size: 50,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::Bzip2 => FileSignature {
                file_type: *self,
                extension: "bz2",
                description: "BZIP2 压缩",
                header: &[0x42, 0x5A, 0x68],
                footer: None,
                min_size: 50,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::Xz => FileSignature {
                file_type: *self,
                extension: "xz",
                description: "XZ 压缩",
                header: &[0xFD, 0x37, 0x7A, 0x58, 0x5A, 0x00],
                footer: None,
                min_size: 50,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::Mpeg => FileSignature {
                file_type: *self,
                extension: "mpeg",
                description: "MPEG 视频",
                header: &[0x00, 0x00, 0x01, 0xB3],
                footer: None,
                min_size: 1024,
                max_size: 50 * 1024 * 1024 * 1024,
            },
            FileType::Avi => FileSignature {
                file_type: *self,
                extension: "avi",
                description: "AVI 视频",
                header: &[0x52, 0x49, 0x46, 0x46],
                footer: None,
                min_size: 1024,
                max_size: 50 * 1024 * 1024 * 1024,
            },
            FileType::Mp4 => FileSignature {
                file_type: *self,
                extension: "mp4",
                description: "MP4 视频",
                header: &[0x66, 0x74, 0x79, 0x70],
                footer: None,
                min_size: 1024,
                max_size: 50 * 1024 * 1024 * 1024,
            },
            FileType::Mp3 => FileSignature {
                file_type: *self,
                extension: "mp3",
                description: "MP3 音频",
                header: &[0x49, 0x44, 0x33],
                footer: None,
                min_size: 1024,
                max_size: 1 * 1024 * 1024 * 1024,
            },
            FileType::Wav => FileSignature {
                file_type: *self,
                extension: "wav",
                description: "WAV 音频",
                header: &[0x52, 0x49, 0x46, 0x46],
                footer: None,
                min_size: 1024,
                max_size: 1 * 1024 * 1024 * 1024,
            },
            FileType::Bmp => FileSignature {
                file_type: *self,
                extension: "bmp",
                description: "BMP 图像",
                header: &[0x42, 0x4D],
                footer: None,
                min_size: 100,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Tiff => FileSignature {
                file_type: *self,
                extension: "tiff",
                description: "TIFF 图像",
                header: &[0x49, 0x49, 0x2A, 0x00],
                footer: None,
                min_size: 100,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Rtf => FileSignature {
                file_type: *self,
                extension: "rtf",
                description: "RTF 文档",
                header: &[0x7B, 0x5C, 0x72, 0x74, 0x66],
                footer: None,
                min_size: 100,
                max_size: 100 * 1024 * 1024,
            },
            FileType::Exe => FileSignature {
                file_type: *self,
                extension: "exe",
                description: "Windows 可执行文件",
                header: &[0x4D, 0x5A],
                footer: None,
                min_size: 1024,
                max_size: 1 * 1024 * 1024 * 1024,
            },
            FileType::Elf => FileSignature {
                file_type: *self,
                extension: "elf",
                description: "ELF 可执行文件",
                header: &[0x7F, 0x45, 0x4C, 0x46],
                footer: None,
                min_size: 1024,
                max_size: 1 * 1024 * 1024 * 1024,
            },
            FileType::Dmg => FileSignature {
                file_type: *self,
                extension: "dmg",
                description: "Apple DMG 镜像",
                header: &[0x78, 0x01, 0x73, 0x0D, 0x62, 0x62, 0x60],
                footer: None,
                min_size: 1024,
                max_size: 10 * 1024 * 1024 * 1024,
            },
            FileType::Iso => FileSignature {
                file_type: *self,
                extension: "iso",
                description: "ISO 光盘镜像",
                header: &[0x43, 0x44, 0x30, 0x30, 0x31],
                footer: None,
                min_size: 32 * 1024,
                max_size: 50 * 1024 * 1024 * 1024,
            },
            FileType::Doc => FileSignature {
                file_type: *self,
                extension: "doc",
                description: "Microsoft Word 文档",
                header: &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
                footer: None,
                min_size: 1024,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Xls => FileSignature {
                file_type: *self,
                extension: "xls",
                description: "Microsoft Excel 表格",
                header: &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
                footer: None,
                min_size: 1024,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Ppt => FileSignature {
                file_type: *self,
                extension: "ppt",
                description: "Microsoft PowerPoint 演示",
                header: &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1],
                footer: None,
                min_size: 1024,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Docx => FileSignature {
                file_type: *self,
                extension: "docx",
                description: "Microsoft Word 2007+ 文档",
                header: &[0x50, 0x4B, 0x03, 0x04],
                footer: None,
                min_size: 1024,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Xlsx => FileSignature {
                file_type: *self,
                extension: "xlsx",
                description: "Microsoft Excel 2007+ 表格",
                header: &[0x50, 0x4B, 0x03, 0x04],
                footer: None,
                min_size: 1024,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Pptx => FileSignature {
                file_type: *self,
                extension: "pptx",
                description: "Microsoft PowerPoint 2007+ 演示",
                header: &[0x50, 0x4B, 0x03, 0x04],
                footer: None,
                min_size: 1024,
                max_size: 500 * 1024 * 1024,
            },
            FileType::Sqlite => FileSignature {
                file_type: *self,
                extension: "sqlite",
                description: "SQLite 数据库",
                header: &[0x53, 0x51, 0x4C, 0x69, 0x74, 0x65, 0x20, 0x66],
                footer: None,
                min_size: 1024,
                max_size: 10 * 1024 * 1024 * 1024,
            },
        }
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sig = self.get_signature();
        write!(f, "{}", sig.description)
    }
}

pub fn get_all_signatures() -> Vec<FileSignature> {
    FileType::all_types()
        .iter()
        .map(|ft| ft.get_signature())
        .collect()
}

pub fn get_max_header_length() -> usize {
    get_all_signatures()
        .iter()
        .map(|s| s.header.len())
        .max()
        .unwrap_or(16)
}

pub fn get_max_footer_length() -> usize {
    get_all_signatures()
        .iter()
        .map(|s| s.footer.map(|f| f.len()).unwrap_or(0))
        .max()
        .unwrap_or(16)
}
