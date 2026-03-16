use std::sync::{Mutex, OnceLock};
use std::collections::VecDeque;
use std::time::SystemTime;

pub struct Logger {
    logs: Mutex<VecDeque<String>>,
    max_logs: usize,
}

static INSTANCE: OnceLock<Logger> = OnceLock::new();

impl Logger {
    pub fn global() -> &'static Logger {
        INSTANCE.get_or_init(|| Logger {
            logs: Mutex::new(VecDeque::new()),
            max_logs: 100,
        })
    }

    pub fn log(&self, level: &str, msg: &str) {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        let log_entry = format!("[{}] [{}] {}", timestamp % 86400, level, msg);
        
        if let Ok(mut logs) = self.logs.lock() {
            logs.push_back(log_entry);
            if logs.len() > self.max_logs {
                logs.pop_front();
            }
        }
    }

    pub fn get_logs(&self) -> Vec<String> {
        if let Ok(logs) = self.logs.lock() {
            logs.iter().cloned().collect()
        } else {
            vec!["<Logger Locked>".to_string()]
        }
    }
}

pub fn info(msg: &str) {
    Logger::global().log("INFO", msg);
}

pub fn warn(msg: &str) {
    Logger::global().log("WARN", msg);
}

pub fn error(msg: &str) {
    Logger::global().log("ERROR", msg);
}
