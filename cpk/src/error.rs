//! 错误类型。

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// 不是 CPK 包（魔数不符）。
    NotCpk,
    /// 不支持的版本号。
    BadVersion(u32),
    /// 头/表字段自相矛盾。
    BadHeader(&'static str),
    /// 表区数据不足或越界。
    BadTable,
    /// 包内路径非法（含 ..、绝对路径或驱动器号）。
    UnsafePath(String),
    /// 表中找不到指定文件。
    NotFound(String),
    /// 只读句柄上执行了写操作。
    ReadOnly,
    /// 未实现的过滤器编号。
    UnsupportedFilter(u16),
    DecompressFailed(u32),
    CompressFailed,
    /// LZSS 解压输出超出预期长度（数据损坏）。
    LzssOverflow,
    /// MD5 校验不一致。
    Md5Mismatch { path: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "IO 错误: {e}"),
            Error::NotCpk => write!(f, "不是 CPK 包（魔数不符，应为 RST\\x1A）"),
            Error::BadVersion(v) => write!(f, "不支持的 CPK 版本: {v}"),
            Error::BadHeader(what) => write!(f, "CPK 头部字段异常: {what}"),
            Error::BadTable => write!(f, "文件表读取失败或长度越界"),
            Error::UnsafePath(p) => write!(f, "包内路径不安全，已拒绝: {p}"),
            Error::NotFound(p) => write!(f, "包内找不到: {p}"),
            Error::ReadOnly => write!(f, "包以只读方式打开，无法执行写操作"),
            Error::UnsupportedFilter(id) => write!(f, "未实现的过滤器编号: {id}"),
            Error::DecompressFailed(id) => write!(f, "解压失败（过滤器 {id}）"),
            Error::CompressFailed => write!(f, "压缩失败"),
            Error::LzssOverflow => write!(f, "LZSS 解压输出超出预期长度（数据损坏）"),
            Error::Md5Mismatch { path } => write!(f, "MD5 校验不一致: {path}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Error {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
