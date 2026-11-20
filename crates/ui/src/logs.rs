//! 日志环形缓冲（P2.5，矩阵 §7.5「日志查询/导出/清空 API」）。
//!
//! 纯逻辑的**固定容量环形缓冲** + 级别/关键字过滤 + 导出行视图，可离线单测。
//! **不依赖 `tracing`**——捕获适配（`tracing` Layer → [`push`]) 由宿主（`apps/ui-server`
//! 已有 `tracing-subscriber`）负责，本 crate 只持有数据 + 查询，守离线门纯净。
//!
//! 时间戳用 `std::time::SystemTime` 的 epoch **微秒**（不引 `time`/`chrono`，守离线门）；
//! 前端负责格式化展示。级别用字符串（`INFO`/`WARN`/`ERROR`/`DEBUG`，对齐前端 `LogLevel`）。

use serde::{Deserialize, Serialize};

/// 默认环形缓冲容量（对齐日志页「已保留最近 2000 条」文案）。
pub const DEFAULT_LOG_CAPACITY: usize = 2000;

/// 单行日志（对齐前端 `LogLine`；`t` 为 epoch 微秒，前端格式化）。
/// 字段 `module` 序列化键名归一到 `mod`（`mod` 是 Rust 保留字，不能直接作字段名）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// 时间戳（epoch **微秒**，前端 `new Date(t/1000)` 格式化）。
    pub t: u64,
    /// 级别（`INFO`/`WARN`/`ERROR`/`DEBUG`）。
    pub level: String,
    /// 模块/目标（tracing `target`，如 `session`/`encoder`；序列化键名 `mod`）。
    #[serde(rename = "mod")]
    pub module: String,
    /// 消息正文。
    pub msg: String,
    /// 键值字段串（`peer=... nonce=...`，可空）。
    pub kv: String,
}

impl LogLine {
    /// 构造一行（`t` 为 epoch 微秒）。
    pub fn new(level: &str, module: &str, msg: &str, kv: &str, t_micros: u64) -> Self {
        Self {
            t: t_micros,
            level: level.to_string(),
            module: module.to_string(),
            msg: msg.to_string(),
            kv: kv.to_string(),
        }
    }

    /// 当前 epoch 微秒（日志页/宿主写入时用）。
    pub fn now_micros() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }
}

/// 级别过滤（与前端 `LogFilter` 语义一致：`ALL` 不限）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LevelFilter {
    /// 全部（默认）。
    #[default]
    All,
    /// 仅 `INFO`。
    Info,
    /// 仅 `WARN`。
    Warn,
    /// 仅 `ERROR`。
    Error,
    /// 仅 `DEBUG`。
    Debug,
}

impl LevelFilter {
    /// 由前端字符串（`ALL`/`INFO`/…）解析；未知 → `All`。
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_uppercase().as_str() {
            "INFO" => Self::Info,
            "WARN" | "WARNING" => Self::Warn,
            "ERROR" => Self::Error,
            "DEBUG" => Self::Debug,
            _ => Self::All,
        }
    }

    /// 是否匹配一行（`All` 恒真；否则级别相等）。
    fn matches(&self, level: &str) -> bool {
        match self {
            Self::All => true,
            Self::Info => level.eq_ignore_ascii_case("INFO"),
            Self::Warn => level.eq_ignore_ascii_case("WARN"),
            Self::Error => level.eq_ignore_ascii_case("ERROR"),
            Self::Debug => level.eq_ignore_ascii_case("DEBUG"),
        }
    }
}

/// 固定容量环形缓冲（`push` 超容量丢弃最旧，对齐日志页「保留最近 2000 条」）。
#[derive(Debug, Clone, Default)]
pub struct LogRing {
    /// 环形数据（容量定后不复分配）。
    buf: Vec<LogLine>,
    /// 下一个写入位置（`buf.len()` = 容量；容量 0 时永不存）。
    next: usize,
    /// 已存行数（`<=` 容量；满后恒为容量）。
    count: usize,
}

impl LogRing {
    /// 新建容量为 `cap` 的环形缓冲（`0` → 永不存，`push` 立即丢）。
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: vec![LogLine::new("", "", "", "", 0); cap],
            next: 0,
            count: 0,
        }
    }

    /// 默认容量 [`DEFAULT_LOG_CAPACITY`]。
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_LOG_CAPACITY)
    }

    /// 当前容量（最大可存行数）。
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// 当前已存行数（`<= capacity`）。
    pub fn len(&self) -> usize {
        self.count
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// 写入一行（容量 0 → 丢；满 → 覆盖最旧并推进环）。
    pub fn push(&mut self, line: LogLine) {
        if self.buf.is_empty() {
            return;
        }
        self.buf[self.next] = line;
        self.next = (self.next + 1) % self.buf.len();
        self.count = (self.count + 1).min(self.buf.len());
    }

    /// 清空（计数归零；环形缓冲已定容，不复分配）。
    pub fn clear(&mut self) {
        self.count = 0;
        self.next = 0;
    }

    /// 按「最旧→最新」取当前所有行（环形展开）。
    pub fn iter_ordered(&self) -> Vec<&LogLine> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        // 未填满：`[0..count)` 即最旧→最新。
        if self.count < self.buf.len() {
            return self.buf[..self.count].iter().collect();
        }
        // 已满：`next` 指向最旧，从 `next` 绕到 `next-1` 为最旧→最新。
        let n = self.buf.len();
        (0..n)
            .map(|i| &self.buf[(self.next + i) % n])
            .collect()
    }

    /// 导出行视图（`min_level` + 非空 `keyword` 过滤 `module`/`msg`/`kv`，小写匹配），
    /// 保持最旧→最新。供「导出 log」端点。
    pub fn export_lines(&self, min_level: &str, keyword: &str) -> Vec<LogLine> {
        let filter = LevelFilter::parse(min_level);
        let q = keyword.to_ascii_lowercase();
        self.iter_ordered()
            .into_iter()
            .filter(|l| filter.matches(&l.level))
            .filter(|l| q.is_empty() || format!("{} {} {}", l.module, l.msg, l.kv).to_lowercase().contains(&q))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: &str, msg: &str, t: u64) -> LogLine {
        LogLine::new(level, "session", msg, "", t)
    }

    #[test]
    fn ring_fills_then_overwrites_oldest() {
        let mut r = LogRing::with_capacity(3);
        for i in 0..3 {
            r.push(line("INFO", &format!("m{i}"), i));
        }
        assert_eq!(r.len(), 3, "填满 3");
        assert_eq!(r.iter_ordered()[0].msg, "m0", "未满时最旧在前");
        r.push(line("INFO", "m3", 3));
        assert_eq!(r.len(), 3, "满后不涨");
        let ordered = r.iter_ordered();
        assert_eq!(ordered[0].msg, "m1", "覆盖最旧（m0）后从 m1 起");
        assert_eq!(ordered[2].msg, "m3", "最新 m3 在尾");
    }

    #[test]
    fn zero_capacity_drops() {
        let mut r = LogRing::with_capacity(0);
        r.push(line("INFO", "x", 0));
        assert!(r.is_empty(), "容量 0 永不存");
        assert_eq!(r.capacity(), 0);
    }

    #[test]
    fn clear_resets_count() {
        let mut r = LogRing::new();
        for i in 0..5 {
            r.push(line("INFO", &format!("a{i}"), i));
        }
        r.clear();
        assert!(r.is_empty(), "清空后空");
        assert_eq!(r.capacity(), DEFAULT_LOG_CAPACITY, "容量不变");
    }

    #[test]
    fn export_filters_level_and_keyword() {
        let mut r = LogRing::new();
        r.push(line("INFO", "ws connected", 1));
        r.push(line("WARN", "hole failed", 2));
        r.push(LogLine::new("ERROR", "auth failed", "auth", "peer=41C98B7E", 3));
        // 全级别
        let all = r.export_lines("ALL", "");
        assert_eq!(all.len(), 3);
        // 仅 ERROR
        let err = r.export_lines("ERROR", "");
        assert_eq!(err.len(), 1);
        assert_eq!(err[0].level, "ERROR");
        // 关键字命中 mod/msg/kv
        let kw = r.export_lines("ALL", "auth");
        assert_eq!(kw.len(), 1, "auth 命中 ERROR 行: {:?}", kw);
        assert!(kw[0].kv.contains("peer=41C98B7E"));
        // 级别 + 关键字叠加：WARN 且含 hole
        let wh = r.export_lines("WARN", "hole");
        assert_eq!(wh.len(), 1, "WARN∩hole");
    }

    #[test]
    fn level_parse_maps_synonyms() {
        assert_eq!(LevelFilter::parse("ALL"), LevelFilter::All);
        assert_eq!(LevelFilter::parse("warn"), LevelFilter::Warn);
        assert_eq!(LevelFilter::parse("warning"), LevelFilter::Warn);
        assert_eq!(LevelFilter::parse("debug"), LevelFilter::Debug);
        assert_eq!(LevelFilter::parse("bogus"), LevelFilter::All, "未知回 All");
    }

    #[test]
    fn logline_now_micros_and_camel_serialization() {
        let now = LogLine::now_micros();
        assert!(now > 0, "epoch 微秒非零: {now}");
        let l = LogLine::new("INFO", "mdns", "start", "domain=_landesk._tcp.local.", now);
        let json = serde_json::to_value(&l).expect("serialize");
        assert_eq!(json["mod"], "mdns", "serde 字段名 mod（非保留字冲突）");
        assert_eq!(json["t"], now);
        assert_eq!(json["kv"], "domain=_landesk._tcp.local.");
    }
}
