//! CPK 包读写核心（对应 C++ 源 `CPKCore/CPKCore.cpp`，"game resource file
//! system 2.0"）。
//!
//! 布局：`头(124B) | 数据区(若干 [数据|简单名|00 00|附加信息56B] 记录) | 表区(尾部)`。
//! 表按 CRC 升序；删除只置 0x10 标志变碎片；新数据优先塞碎片，否则写旧表位置、
//! 表整体右移；表满按 5000 项扩容。偏移全部 32 位，包上限 4GB。

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use md5::{Digest, Md5};

use crate::crc::{crc32, crc_of_path};
use crate::error::{Error, Result};
use crate::filter::{self, FILTER_CHANGELESS};
use crate::format::{
    filter_id, is_live, make_flag, COMPRESS_SIZE, CPK_LABEL, CPK_VERSION, EXTRA_INFO_SIZE,
    FileExtraInfo, Header, HEADER_SIZE, LEN_OF_STRING_END, TABLE_ADD, TABLE_ENTRY_SIZE, FLAG_DIR,
    FLAG_LARGE, FLAG_REMOVED, FLAG_VALID, TableEntry, unix_to_filetime,
};
use crate::names::{gbk_lossy, gbk_to_os, os_to_gbk};

/// 进度回调：(当前条目名, 已完成, 总数)。
pub type Progress<'a> = &'a mut dyn FnMut(&str, u64, u64);

/// 包内一个有效条目（文件或目录）。
#[derive(Clone, Debug)]
pub struct Entry {
    pub index: usize,
    pub crc: u32,
    pub father_crc: u32,
    pub is_dir: bool,
    /// 大文件标志（未压缩存储）。
    pub large_file: bool,
    pub filter: u32,
    pub start_pos: u32,
    pub packed_size: u32,
    pub origin_size: u32,
    pub extra_info_size: u32,
    /// 原始数据的 MD5（打包时记录）。
    pub md5: [u8; 16],
    /// 简单文件名（GBK 原始字节）。
    pub name_bytes: Vec<u8>,
    /// 包内全路径，'\\' 分隔（GBK 原始字节）。
    pub full_path_bytes: Vec<u8>,
    /// 简单文件名（显示用，有损转换）。
    pub name: String,
    /// 包内全路径（显示用，有损转换）。
    pub full_path: String,
}

/// 打包选项。
#[derive(Clone, Copy, Debug, Default)]
pub struct AddOpts {
    /// 强制不压缩（直通存储）。
    pub no_compress: bool,
}

/// `info` 输出的包概况。
#[derive(Clone, Debug)]
pub struct CpkInfo {
    pub path: PathBuf,
    pub file_len: u64,
    pub header: Header,
    pub file_count: u64,
    pub dir_count: u64,
    pub fragment_count: u64,
    pub fragment_space: u64,
    pub packed_bytes: u64,
    pub origin_bytes: u64,
}

/// `verify` 结果统计。
#[derive(Clone, Debug, Default)]
pub struct VerifyReport {
    pub checked: u64,
    pub md5_ok: u64,
    /// 空文件/目录不做 MD5 比对（原工具对空内容记录的是打包机路径的 MD5，无法复算）
    pub md5_skipped: u64,
    pub md5_bad: Vec<String>,
    pub read_bad: Vec<String>,
    pub crc_checked: u64,
    pub crc_bad: Vec<String>,
}

/// `extract` 结果统计。
#[derive(Clone, Copy, Debug, Default)]
pub struct ExtractReport {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
}

/// 打开的 CPK 包。
pub struct Cpk {
    file: File,
    writable: bool,
    path: PathBuf,
    file_len: u64,
    header: Header,
    /// 表项数组，长度 == max_table_num，前 valid_table_num 个有效。
    table: Vec<TableEntry>,
    /// 与 table 平行：有效表项的简单名（GBK 字节），碎片为空。
    names: Vec<Vec<u8>>,
    /// 与 table 平行：有效表项记录的 MD5。
    md5s: Vec<[u8; 16]>,
}

impl Cpk {
    // ------------------------------------------------------------------
    // 打开 / 新建
    // ------------------------------------------------------------------

    /// 打开已有 CPK 包。优先以读写方式打开，失败时退化为只读
    /// （只读句柄上执行写操作会返回 [`Error::ReadOnly`]）。
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Cpk> {
        let path = path.as_ref().to_path_buf();
        let file_len = fs::metadata(&path)?.len();
        let (file, writable) = match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(f) => (f, true),
            Err(_) => (File::open(&path)?, false),
        };
        let mut cpk = Cpk {
            file,
            writable,
            file_len,
            path,
            header: Header::default(),
            table: Vec::new(),
            names: Vec::new(),
            md5s: Vec::new(),
        };
        cpk.load()?;
        Ok(cpk)
    }

    /// 新建空 CPK 包。文件已存在时报错。
    pub fn create<P: AsRef<Path>>(path: P, slots: u32) -> Result<Cpk> {
        let path = path.as_ref().to_path_buf();
        let slots = slots.max(1);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        let header = Header {
            label: CPK_LABEL,
            version: CPK_VERSION,
            table_start: HEADER_SIZE as u32,
            data_start: HEADER_SIZE as u32,
            file_num: 0,
            is_formatted: 0,
            size_of_header: HEADER_SIZE as u32,
            valid_table_num: 0,
            max_table_num: slots,
            fragment_num: 0,
            package_size: (HEADER_SIZE + slots as usize * TABLE_ENTRY_SIZE) as u32,
            reserved: [0; 20],
        };
        file.write_all(&header.to_bytes())?;
        let zeros = vec![0u8; slots as usize * TABLE_ENTRY_SIZE];
        file.write_all(&zeros)?;
        let file_len = file.metadata()?.len();
        Ok(Cpk {
            file,
            writable: true,
            path,
            file_len,
            header,
            table: vec![TableEntry::default(); slots as usize],
            names: vec![Vec::new(); slots as usize],
            md5s: vec![[0u8; 16]; slots as usize],
        })
    }

    fn load(&mut self) -> Result<()> {
        let mut buf = [0u8; HEADER_SIZE];
        self.read_exact_at(0, &mut buf)?;
        self.header = Header::from_bytes(&buf);
        if self.header.label != CPK_LABEL {
            return Err(Error::NotCpk);
        }
        if self.header.version != CPK_VERSION {
            return Err(Error::BadVersion(self.header.version));
        }
        let valid = self.header.valid_table_num as usize;
        let max = self.header.max_table_num as usize;
        if max < valid || self.header.file_num > self.header.valid_table_num {
            return Err(Error::BadHeader(
                "dwFileNum/dwValidTableNum/dwMaxTableNum 关系不成立",
            ));
        }
        let table_start = self.header.table_start as u64;
        if table_start + (valid * TABLE_ENTRY_SIZE) as u64 > self.file_len {
            return Err(Error::BadTable);
        }
        // 游戏端写入的包表区可能不足 max*28（只写了实际需要的条目），按可用字节读取
        let avail = (self.file_len - table_start).min((max * TABLE_ENTRY_SIZE) as u64) as usize;
        let want = (valid * TABLE_ENTRY_SIZE).max(avail);
        let mut raw = vec![0u8; want];
        self.read_exact_at(table_start, &mut raw)?;
        self.table = Vec::with_capacity(max);
        for i in 0..max {
            let off = i * TABLE_ENTRY_SIZE;
            self.table.push(if off + TABLE_ENTRY_SIZE <= raw.len() {
                TableEntry::from_bytes(&raw[off..off + TABLE_ENTRY_SIZE])
            } else {
                TableEntry::default()
            });
        }
        self.names = vec![Vec::new(); max];
        self.md5s = vec![[0u8; 16]; max];
        self.load_meta()?;
        Ok(())
    }

    /// 读取每个有效表项的附加区，取出简单名与 MD5。
    fn load_meta(&mut self) -> Result<()> {
        let valid = self.header.valid_table_num as usize;
        let mut blob: Vec<u8> = Vec::new();
        for i in 0..valid {
            let e = self.table[i];
            if !is_live(e.flag) {
                continue;
            }
            let need = e.extra_info_size as usize;
            if need < LEN_OF_STRING_END + EXTRA_INFO_SIZE {
                continue; // 附加区不完整（损坏或非标准），名字留空
            }
            let pos = e.start_pos as u64 + e.packed_size as u64;
            if pos + need as u64 > self.file_len {
                continue;
            }
            blob.clear();
            blob.resize(need, 0);
            self.read_exact_at(pos, &mut blob)?;
            let name_end = blob.iter().position(|&b| b == 0).unwrap_or(need);
            self.names[i] = blob[..name_end].to_vec();
            let md5_off = need - 16;
            self.md5s[i].copy_from_slice(&blob[md5_off..need]);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // 查询
    // ------------------------------------------------------------------

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn writable(&self) -> bool {
        self.writable
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn info(&self) -> CpkInfo {
        let valid = (self.header.valid_table_num as usize).min(self.table.len());
        let mut file_count = 0u64;
        let mut dir_count = 0u64;
        let mut fragment_space = 0u64;
        let mut packed_bytes = 0u64;
        let mut origin_bytes = 0u64;
        for e in &self.table[..valid] {
            if e.flag & FLAG_REMOVED != 0 {
                fragment_space += e.packed_size as u64;
            } else if e.flag & FLAG_DIR != 0 {
                dir_count += 1;
            } else {
                file_count += 1;
                packed_bytes += e.packed_size as u64;
                origin_bytes += e.origin_size as u64;
            }
        }
        CpkInfo {
            path: self.path.clone(),
            file_len: self.file_len,
            header: self.header,
            file_count,
            dir_count,
            fragment_count: self.header.fragment_num as u64,
            fragment_space,
            packed_bytes,
            origin_bytes,
        }
    }

    /// 全部有效条目（文件与目录），按表序（CRC 升序）返回。
    pub fn entries(&self) -> Vec<Entry> {
        let valid = (self.header.valid_table_num as usize).min(self.table.len());
        let mut out = Vec::new();
        for i in 0..valid {
            let e = self.table[i];
            if !is_live(e.flag) {
                continue;
            }
            let name_bytes = self.names.get(i).cloned().unwrap_or_default();
            let full_path_bytes = self.full_path_bytes(i);
            out.push(Entry {
                index: i,
                crc: e.crc,
                father_crc: e.father_crc,
                is_dir: e.flag & FLAG_DIR != 0,
                large_file: e.flag & FLAG_LARGE != 0,
                filter: filter_id(e.flag),
                start_pos: e.start_pos,
                packed_size: e.packed_size,
                origin_size: e.origin_size,
                extra_info_size: e.extra_info_size,
                md5: self.md5s.get(i).copied().unwrap_or([0; 16]),
                name: gbk_lossy(&name_bytes),
                full_path: gbk_lossy(&full_path_bytes),
                name_bytes,
                full_path_bytes,
            });
        }
        out
    }

    /// 沿父目录链拼出包内全路径（GBK 字节，存储大小写）。
    fn full_path_bytes(&self, mut idx: usize) -> Vec<u8> {
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let mut guard = 0usize;
        loop {
            guard += 1;
            if guard > 4096 {
                break; // 父链成环等异常，防死循环
            }
            let e = match self.table.get(idx) {
                Some(e) if is_live(e.flag) => *e,
                _ => break,
            };
            let name = self.names.get(idx).cloned().unwrap_or_default();
            parts.push(name);
            if e.father_crc == 0 {
                break;
            }
            match self.find_live_by_crc(e.father_crc) {
                Some(p) => idx = p,
                None => break,
            }
        }
        let mut out = Vec::new();
        for (k, p) in parts.iter().rev().enumerate() {
            if k > 0 {
                out.push(b'\\');
            }
            out.extend_from_slice(p);
        }
        out
    }

    /// 表内按 CRC 二分查找有效表项（同原版 `GetTableIndexFromCRC`：
    /// 同 CRC 的重复项中，新插入的有效项排在最后）。
    fn find_live_by_crc(&self, crc: u32) -> Option<usize> {
        let valid = (self.header.valid_table_num as usize).min(self.table.len());
        // 上界：第一个 crc > target 的位置
        let mut lo = 0usize;
        let mut hi = valid;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.table[mid].crc <= crc {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let mut i = lo;
        while i > 0 {
            let e = self.table[i - 1];
            if e.crc != crc {
                break;
            }
            if is_live(e.flag) {
                return Some(i - 1);
            }
            i -= 1;
        }
        None
    }

    /// 读出并解压一个条目的原始内容。
    pub fn read_file(&mut self, e: &Entry) -> Result<Vec<u8>> {
        if e.is_dir {
            return Ok(Vec::new());
        }
        let te = self.table[e.index];
        self.read_entry_data(&te)
    }

    /// 读出条目前 `max_len` 字节（自动钳到条目原始大小）。
    /// 未压缩条目（Changeless / 大文件标志）只读所需长度，磁盘 I/O 同步省下；
    /// 压缩条目读整个压缩块后只解压前缀（LZO/LZSS 均为流式编码，
    /// 见 filter::decompress_prefix）。供只取文件头的轻量解析使用。
    pub fn read_file_prefix(&mut self, e: &Entry, max_len: usize) -> Result<Vec<u8>> {
        if e.is_dir {
            return Ok(Vec::new());
        }
        let te = self.table[e.index];
        let want = max_len.min(te.origin_size as usize);
        if te.flag & FLAG_LARGE != 0 || filter_id(te.flag) == FILTER_CHANGELESS {
            let mut raw = vec![0u8; want];
            if want > 0 {
                self.read_exact_at(te.start_pos as u64, &mut raw)?;
            }
            return Ok(raw);
        }
        let (packed, filter) = self.read_packed_entry(&te)?;
        filter::decompress_prefix(filter, &packed, te.origin_size, want)
    }

    /// 读出条目的压缩数据块与过滤器编号（大文件标志视为 Changeless 直通，
    /// 返回的编号统一可直接喂给 filter::decompress / decompress_prefix）。
    /// 供调用方缓存压缩块、多次解不同长度前缀，避免重复磁盘读。
    pub fn read_packed(&mut self, e: &Entry) -> Result<(Vec<u8>, u32)> {
        if e.is_dir {
            return Ok((Vec::new(), FILTER_CHANGELESS));
        }
        let te = self.table[e.index];
        self.read_packed_entry(&te)
    }

    fn read_packed_entry(&mut self, e: &TableEntry) -> Result<(Vec<u8>, u32)> {
        let mut raw = vec![0u8; e.packed_size as usize];
        if e.packed_size > 0 {
            self.read_exact_at(e.start_pos as u64, &mut raw)?;
        }
        let filter = if e.flag & FLAG_LARGE != 0 {
            FILTER_CHANGELESS
        } else {
            filter_id(e.flag)
        };
        Ok((raw, filter))
    }

    fn read_entry_data(&mut self, e: &TableEntry) -> Result<Vec<u8>> {
        let packed = e.packed_size as usize;
        let mut raw = vec![0u8; packed];
        if packed > 0 {
            self.read_exact_at(e.start_pos as u64, &mut raw)?;
        }
        if e.flag & FLAG_LARGE != 0 || filter_id(e.flag) == filter::FILTER_CHANGELESS {
            return Ok(raw);
        }
        filter::decompress(filter_id(e.flag), &raw, e.origin_size)
    }

    // ------------------------------------------------------------------
    // 解包 / 校验
    // ------------------------------------------------------------------

    /// 解包匹配的条目到 `dest`，保持包内目录结构。
    ///
    /// 匹配到目录时自动包含其全部子项（对应原工具递归释放目录的行为）。
    pub fn extract(
        &mut self,
        dest: &Path,
        keep: &mut dyn FnMut(&Entry) -> bool,
        prog: Progress,
    ) -> Result<ExtractReport> {
        let all = self.entries();
        let matched: Vec<&Entry> = all.iter().filter(|e| keep(e)).collect();
        let matched_crcs: std::collections::HashSet<u32> =
            matched.iter().map(|e| e.crc).collect();
        let dir_prefixes: Vec<String> = matched
            .iter()
            .filter(|e| e.is_dir)
            .map(|e| format!("{}\\", e.full_path.to_lowercase()))
            .collect();
        let selected: Vec<&Entry> = all
            .iter()
            .filter(|e| {
                if matched_crcs.contains(&e.crc) {
                    return true;
                }
                let lp = e.full_path.to_lowercase();
                dir_prefixes.iter().any(|p| lp.starts_with(p.as_str()))
            })
            .collect();
        let total = selected.len() as u64;
        let mut report = ExtractReport::default();
        let mut done = 0u64;
        for e in &selected {
            done += 1;
            let rel = safe_components(&e.full_path_bytes)?;
            let mut target = dest.to_path_buf();
            for comp in &rel {
                target.push(gbk_to_os(comp));
            }
            if e.is_dir {
                fs::create_dir_all(&target)?;
                report.dirs += 1;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let te = self.table[e.index];
                let data = self.read_entry_data(&te)?;
                fs::write(&target, &data)?;
                report.files += 1;
                report.bytes += data.len() as u64;
            }
            prog(&e.full_path, done, total);
        }
        Ok(report)
    }

    /// 全量校验：解压每个文件比对长度与 MD5，并对全名重算 CRC 比对表键。
    pub fn verify(&mut self, prog: Progress) -> Result<VerifyReport> {
        let entries = self.entries();
        let total = entries.len() as u64;
        let mut report = VerifyReport::default();
        let mut done = 0u64;
        for e in &entries {
            done += 1;
            prog(&e.full_path, done, total);
            // CRC 自检：全名（ASCII 小写折叠后）重算应等于表键
            if crc_of_path(&e.full_path_bytes) == e.crc {
                report.crc_checked += 1;
            } else {
                report.crc_bad.push(e.full_path.clone());
            }
            if e.is_dir {
                report.md5_skipped += 1;
                continue;
            }
            report.checked += 1;
            let te = self.table[e.index];
            let data = match self.read_entry_data(&te) {
                Ok(d) => d,
                Err(err) => {
                    report.read_bad.push(format!("{}: {err}", e.full_path));
                    continue;
                }
            };
            if data.len() != e.origin_size as usize {
                report.read_bad.push(format!(
                    "{}: 解压后长度 {} 与表项 {} 不符",
                    e.full_path,
                    data.len(),
                    e.origin_size
                ));
                continue;
            }
            if e.origin_size == 0 || e.packed_size == 0 {
                // 原工具对空内容记录的是打包机磁盘路径/目录名的 MD5，无法复算
                report.md5_skipped += 1;
                continue;
            }
            let digest: [u8; 16] = Md5::digest(&data).into();
            if digest == e.md5 {
                report.md5_ok += 1;
            } else {
                report.md5_bad.push(e.full_path.clone());
            }
        }
        Ok(report)
    }

    // ------------------------------------------------------------------
    // 打包 / 删除
    // ------------------------------------------------------------------

    /// 把磁盘文件或目录（递归）加入包内根目录下，返回新增条目数。
    ///
    /// 打包目录时取其**内容**进入包根目录（子目录保留为包内目录），
    /// 与「解包 → 修改 → 重新打包」的往返语义一致。
    pub fn add_path(&mut self, src: &Path, opts: &AddOpts, prog: Progress) -> Result<u64> {
        self.require_writable()?;
        let meta = fs::metadata(src)?;
        if !meta.is_file() && !meta.is_dir() {
            return Err(Error::NotFound(src.display().to_string()));
        }
        let total = count_items(src)?;
        let mut done = 0u64;
        let count = if meta.is_dir() {
            let mut children: Vec<PathBuf> = fs::read_dir(src)?
                .filter_map(|en| en.ok().map(|en| en.path()))
                .collect();
            children.sort();
            let mut n = 0u64;
            for child in &children {
                n += self.add_recursive(child, 0, opts, &mut done, total, prog)?;
            }
            n
        } else {
            self.add_recursive(src, 0, opts, &mut done, total, prog)?
        };
        self.flush()?;
        Ok(count)
    }

    fn add_recursive(
        &mut self,
        src: &Path,
        father_crc: u32,
        opts: &AddOpts,
        done: &mut u64,
        total: u64,
        prog: Progress,
    ) -> Result<u64> {
        let name_bytes = os_to_gbk(&src.file_name().unwrap_or_default());
        if name_bytes.is_empty() {
            return Err(Error::UnsafePath(src.display().to_string()));
        }
        let display = gbk_lossy(&name_bytes);
        let meta = fs::metadata(src)?;
        if meta.is_dir() {
            let crc = self.ensure_dir(&name_bytes, father_crc)?;
            *done += 1;
            prog(&display, *done, total);
            let mut count = 1u64;
            let mut children: Vec<PathBuf> = fs::read_dir(src)?
                .filter_map(|en| en.ok().map(|en| en.path()))
                .collect();
            children.sort();
            for child in children {
                count += self.add_recursive(&child, crc, opts, done, total, prog)?;
            }
            Ok(count)
        } else {
            self.add_single_file(src, &name_bytes, father_crc, opts)?;
            *done += 1;
            prog(&display, *done, total);
            Ok(1)
        }
    }

    /// 计算包内全路径字节：父目录全路径 + '\\' + 简单名（存储大小写）。
    fn full_name_bytes(&self, simple: &[u8], father_crc: u32) -> Vec<u8> {
        let mut full = if father_crc == 0 {
            Vec::new()
        } else {
            match self.find_live_by_crc(father_crc) {
                Some(i) => self.full_path_bytes(i),
                None => Vec::new(),
            }
        };
        if !full.is_empty() {
            full.push(b'\\');
        }
        full.extend_from_slice(simple);
        full
    }

    /// 在包内建目录（已存在则直接返回其 CRC），返回目录 CRC。
    fn ensure_dir(&mut self, simple: &[u8], father_crc: u32) -> Result<u32> {
        let full = self.full_name_bytes(simple, father_crc);
        let crc = crc_of_path(&full);
        if let Some(i) = self.find_live_by_crc(crc) {
            return Ok(self.table[i].crc); // 幂等：目录已存在
        }
        self.ensure_capacity();
        let idx = self.free_table_index(crc);
        let extra = (simple.len() + LEN_OF_STRING_END + EXTRA_INFO_SIZE) as u32;
        let pos = self.place_data(extra);
        let md5: [u8; 16] = Md5::digest(simple).into(); // 原版对空内容记录名字的 MD5
        self.table[idx] = TableEntry {
            crc,
            flag: FLAG_VALID | FLAG_DIR,
            father_crc,
            start_pos: pos,
            packed_size: 0,
            origin_size: 0,
            extra_info_size: extra,
        };
        self.header.file_num += 1;
        self.header.valid_table_num += 1;
        self.names[idx] = simple.to_vec();
        self.md5s[idx] = md5;
        self.write_record(pos, &[], simple, md5)?;
        Ok(crc)
    }

    /// 打包单个文件。同名（同 CRC）条目按覆盖处理：旧的先标记删除。
    fn add_single_file(
        &mut self,
        src: &Path,
        simple: &[u8],
        father_crc: u32,
        opts: &AddOpts,
    ) -> Result<()> {
        let full = self.full_name_bytes(simple, father_crc);
        let crc = crc_of_path(&full);
        if let Some(old) = self.find_live_by_crc(crc) {
            self.mark_removed(old); // 覆盖：旧数据变碎片
        }

        let data = fs::read(src)?;

        // 过滤器决策（对应 AddSingleFile：≥50MB 或压缩收益不足则直通存储）
        let mut use_store =
            opts.no_compress || data.is_empty() || data.len() as u64 >= COMPRESS_SIZE;
        let mut packed: Option<Vec<u8>> = None;
        if !use_store {
            let z = filter::compress_lzo(&data)?;
            // 压缩后 > 95% 原大则放弃压缩（原版按截断整数百分比比较）
            if (z.len() as u64) * 100 > (data.len() as u64) * 95 {
                use_store = true;
            } else {
                packed = Some(z);
            }
        }
        let filter_sel = if use_store {
            filter::FILTER_CHANGELESS
        } else {
            filter::FILTER_MINILZO
        };
        let low = FLAG_VALID | u32::from(use_store) * FLAG_LARGE;

        self.ensure_capacity();
        let idx = self.free_table_index(crc);
        let md5: [u8; 16] = Md5::digest(&data).into();
        let extra = (simple.len() + LEN_OF_STRING_END + EXTRA_INFO_SIZE) as u32;
        let packed_len = packed.as_ref().map_or(data.len(), |z| z.len()) as u64;
        let total = packed_len + extra as u64;
        if total > u32::MAX as u64 {
            return Err(Error::BadHeader("单条记录超过 4GB 上限"));
        }
        let pos = self.place_data(total as u32);
        self.table[idx] = TableEntry {
            crc,
            flag: make_flag(low, filter_sel),
            father_crc,
            start_pos: pos,
            packed_size: packed_len as u32,
            origin_size: data.len() as u32,
            extra_info_size: extra,
        };
        self.header.file_num += 1;
        self.header.valid_table_num += 1;
        self.names[idx] = simple.to_vec();
        self.md5s[idx] = md5;
        let record_data: &[u8] = packed.as_deref().unwrap_or(&data);
        self.write_record(pos, record_data, simple, md5)?;
        Ok(())
    }

    /// 删除匹配的条目（目录递归删除其下所有子项），返回删除的条目数。
    pub fn remove(&mut self, keep: &mut dyn FnMut(&Entry) -> bool, prog: Progress) -> Result<u64> {
        self.require_writable()?;
        let selected: Vec<Entry> = self.entries().into_iter().filter(|e| keep(e)).collect();
        let total = selected.len() as u64;
        let mut removed = 0u64;
        let mut done = 0u64;
        for e in &selected {
            done += 1;
            prog(&e.full_path, done, total);
            if !is_live(self.table[e.index].flag) {
                continue; // 已随父目录删除
            }
            let mut n = 0u64;
            self.mark_removed_count(e.index, &mut n);
            removed += n;
        }
        self.flush()?;
        Ok(removed)
    }

    /// 标记删除：表项置碎片标志，空间并入 dwPackedSize。
    fn mark_removed(&mut self, idx: usize) {
        let mut n = 0u64;
        self.mark_removed_count(idx, &mut n);
    }

    fn mark_removed_count(&mut self, idx: usize, count: &mut u64) {
        if self.table[idx].flag & FLAG_REMOVED != 0 {
            return;
        }
        *count += 1;
        self.table[idx].flag |= FLAG_REMOVED;
        // 碎片空间 = 数据 + 附加信息；父链断开
        let e = &mut self.table[idx];
        e.packed_size += e.extra_info_size;
        e.extra_info_size = 0;
        e.father_crc = 0;
        self.header.file_num = self.header.file_num.saturating_sub(1);
        self.header.fragment_num += 1;
        self.names[idx] = Vec::new();
        self.md5s[idx] = [0; 16];
        let crc = self.table[idx].crc;
        // 递归删除子项（表未按父序排列，只能全表扫描）
        let valid = (self.header.valid_table_num as usize).min(self.table.len());
        let mut child = 0usize;
        while child < valid {
            let te = self.table[child];
            if te.flag & FLAG_REMOVED == 0 && te.father_crc == crc {
                self.mark_removed_count(child, count);
            }
            child += 1;
        }
    }

    // ------------------------------------------------------------------
    // 底层：表维护与落盘
    // ------------------------------------------------------------------

    fn require_writable(&self) -> Result<()> {
        if self.writable {
            Ok(())
        } else {
            Err(Error::ReadOnly)
        }
    }

    fn read_exact_at(&mut self, pos: u64, buf: &mut [u8]) -> Result<()> {
        self.file.seek(SeekFrom::Start(pos))?;
        self.file.read_exact(buf)?;
        Ok(())
    }

    /// 表容量不足时扩容（步进 5000，对应 `IncreaseTable`）。
    fn ensure_capacity(&mut self) {
        if self.header.valid_table_num + 1 >= self.header.max_table_num {
            let new_max = self.header.max_table_num + TABLE_ADD;
            self.table.resize(new_max as usize, TableEntry::default());
            self.names.resize(new_max as usize, Vec::new());
            self.md5s.resize(new_max as usize, [0; 16]);
            self.header.max_table_num = new_max;
        }
    }

    /// 找到 CRC 有序表中的插入点，把后续表项整体后移一格，返回插入下标
    /// （对应 `GetFreeTableItem`；碎片无需单独的索引映射，动态推导即可）。
    fn free_table_index(&mut self, crc: u32) -> usize {
        let valid = (self.header.valid_table_num as usize).min(self.table.len());
        let mut insert = valid;
        for i in 0..valid {
            if self.table[i].crc > crc {
                insert = i;
                break;
            }
        }
        self.table.insert(insert, TableEntry::default());
        self.table.truncate(self.header.max_table_num as usize);
        self.names.insert(insert, Vec::new());
        self.names.truncate(self.header.max_table_num as usize);
        self.md5s.insert(insert, [0; 16]);
        self.md5s.truncate(self.header.max_table_num as usize);
        insert
    }

    /// 为 `size` 字节的记录找写入位置：优先复用碎片，否则写到表区之前
    /// （表整体右移，对应 `FindCPKFreePlace`）。
    fn place_data(&mut self, size: u32) -> u32 {
        let valid = (self.header.valid_table_num as usize).min(self.table.len());
        // 1) 碎片复用
        for i in 0..valid {
            if self.table[i].flag & FLAG_REMOVED != 0 && self.table[i].packed_size >= size {
                let pos = self.table[i].start_pos;
                self.table[i].start_pos += size;
                self.table[i].packed_size -= size;
                return pos;
            }
        }
        // 2) 追加：新数据落在旧表位置，表右移
        let max_bytes = self.header.max_table_num as u64 * TABLE_ENTRY_SIZE as u64;
        let ideal_end = self.header.table_start as u64 + max_bytes;
        let pos = if self.header.package_size as u64 == ideal_end {
            // 头部统计自洽（原工具产物）：与原算法一致
            self.header.table_start as u64
        } else {
            // 游戏端写入的包表区偏短、或包尾有出入：保守地追加在实际文件尾之后
            self.file_len.max(ideal_end)
        };
        self.header.table_start = pos as u32 + size;
        self.header.package_size = self.header.table_start + max_bytes as u32;
        pos as u32
    }

    /// 把一条 `[数据|简单名|00 00|附加信息]` 记录写到包内 `pos` 处。
    fn write_record(&mut self, pos: u32, data: &[u8], simple: &[u8], md5: [u8; 16]) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let ft = unix_to_filetime(now);
        let extra = FileExtraInfo {
            create_time: ft,
            last_access_time: ft,
            last_write_time: ft,
            add_to_package_time: ft,
            del_from_package_time: 0,
            md5,
        };
        self.file.seek(SeekFrom::Start(pos as u64))?;
        self.file.write_all(data)?;
        self.file.write_all(simple)?;
        self.file.write_all(&[0u8; LEN_OF_STRING_END])?;
        self.file.write_all(&extra.to_bytes())?;
        let end =
            pos as u64 + (data.len() + simple.len() + LEN_OF_STRING_END + EXTRA_INFO_SIZE) as u64;
        if end > self.file_len {
            self.file_len = end;
        }
        Ok(())
    }

    /// 写操作收尾：必要时重定位表区，写回整表与文件头。
    fn flush(&mut self) -> Result<()> {
        self.require_writable()?;
        self.file_len = self.file.metadata()?.len();
        let max_bytes = self.header.max_table_num as u64 * TABLE_ENTRY_SIZE as u64;
        let ideal_end = self.header.table_start as u64 + max_bytes;
        if ideal_end > self.file_len {
            // 表区越出现有文件（扩容过、或游戏端写出的短表包）：重定位到实际文件尾
            let new_start = self.file_len;
            if new_start + max_bytes > u32::MAX as u64 {
                return Err(Error::BadHeader("包大小超过 4GB 上限"));
            }
            self.header.table_start = new_start as u32;
            self.header.package_size = (new_start + max_bytes) as u32;
        } else if self.header.package_size as u64 != ideal_end {
            self.header.package_size = ideal_end as u32;
        }
        let mut raw = Vec::with_capacity(self.header.max_table_num as usize * TABLE_ENTRY_SIZE);
        for e in &self.table {
            raw.extend_from_slice(&e.to_bytes());
        }
        self.file
            .seek(SeekFrom::Start(self.header.table_start as u64))?;
        self.file.write_all(&raw)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&self.header.to_bytes())?;
        self.file_len = self.file.metadata()?.len();
        Ok(())
    }
}

/// 递归统计磁盘路径下的文件与目录总数（用于进度）。
fn count_items(src: &Path) -> Result<u64> {
    let meta = fs::metadata(src)?;
    if meta.is_file() {
        return Ok(1);
    }
    let mut n = 1u64;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        n += count_items(&entry.path())?;
    }
    Ok(n)
}

/// 把包内全路径拆成安全分量：拒绝 `..`、`.`、驱动器号与正斜杠。
fn safe_components(full: &[u8]) -> Result<Vec<&[u8]>> {
    let comps: Vec<&[u8]> = full
        .split(|&b| b == b'\\')
        .filter(|c| !c.is_empty())
        .collect();
    for c in &comps {
        if *c == b".." || *c == b"." || c.iter().any(|&b| b == b':' || b == b'/') {
            return Err(Error::UnsafePath(gbk_lossy(full)));
        }
    }
    Ok(comps)
}

/// 供外部使用：对任意路径计算 CPK CRC（ASCII 小写折叠后）。
pub fn crc_for_path(full: &str) -> u32 {
    crc32(&crate::crc::ascii_lower(full.as_bytes()))
}
