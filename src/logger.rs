//! Registro delle operazioni: finestra dell'app + file con rotazione.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

pub const MAX_LOG_LINES: usize = 500;
const MAX_FILE_BYTES: u64 = 2_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub tag: Tag,
    pub text: String,
}

/// Logger clonabile, utilizzabile anche dai thread di lavoro.
#[derive(Clone)]
pub struct Logger {
    tx: Sender<LogLine>,
    file: Option<PathBuf>,
}

impl Logger {
    pub fn new(tx: Sender<LogLine>, dir: Option<PathBuf>) -> Self {
        let file = dir.and_then(|d| {
            std::fs::create_dir_all(&d).ok()?;
            Some(d.join("meditation_app.log"))
        });
        Self { tx, file }
    }

    pub fn log(&self, tag: Tag, msg: impl AsRef<str>) {
        let now = chrono::Local::now();
        let text = format!("[{}] {}", now.format("%H:%M:%S"), msg.as_ref());
        if let Some(path) = &self.file {
            if std::fs::metadata(path)
                .map(|m| m.len() > MAX_FILE_BYTES)
                .unwrap_or(false)
            {
                let _ = std::fs::rename(path, path.with_extension("log.1"));
            }
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    f,
                    "{} [{:?}] {}",
                    now.format("%Y-%m-%d %H:%M:%S"),
                    tag,
                    msg.as_ref()
                );
            }
        }
        let _ = self.tx.send(LogLine { tag, text });
    }

    pub fn info(&self, m: impl AsRef<str>) {
        self.log(Tag::Info, m);
    }
    pub fn warn(&self, m: impl AsRef<str>) {
        self.log(Tag::Warning, m);
    }
    pub fn error(&self, m: impl AsRef<str>) {
        self.log(Tag::Error, m);
    }
    pub fn success(&self, m: impl AsRef<str>) {
        self.log(Tag::Success, m);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Errors,
}

/// Buffer delle ultime righe, con filtro.
#[derive(Default)]
pub struct LogBuffer {
    lines: VecDeque<LogLine>,
}

impl LogBuffer {
    pub fn push(&mut self, line: LogLine) {
        if self.lines.len() == MAX_LOG_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn filtered(&self, filter: Filter) -> impl Iterator<Item = &LogLine> {
        self.lines
            .iter()
            .filter(move |l| filter == Filter::All || matches!(l.tag, Tag::Error | Tag::Warning))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn line(tag: Tag, t: &str) -> LogLine {
        LogLine {
            tag,
            text: t.into(),
        }
    }

    #[test]
    fn buffer_is_bounded_and_filterable() {
        let mut b = LogBuffer::default();
        for i in 0..(MAX_LOG_LINES + 10) {
            b.push(line(
                if i % 2 == 0 { Tag::Info } else { Tag::Error },
                &i.to_string(),
            ));
        }
        assert_eq!(b.len(), MAX_LOG_LINES);
        assert_eq!(b.filtered(Filter::All).count(), MAX_LOG_LINES);
        assert_eq!(b.filtered(Filter::Errors).count(), MAX_LOG_LINES / 2);
        assert_eq!(
            b.filtered(Filter::All).next().map(|l| l.text.as_str()),
            Some("10")
        );
        b.clear();
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn warnings_count_as_errors_in_filter() {
        let mut b = LogBuffer::default();
        b.push(line(Tag::Warning, "w"));
        b.push(line(Tag::Success, "s"));
        assert_eq!(b.filtered(Filter::Errors).count(), 1);
    }

    #[test]
    fn logger_sends_and_writes_file() {
        let dir = std::env::temp_dir().join(format!("medit_log_{}", std::process::id()));
        let (tx, rx) = channel();
        let l = Logger::new(tx, Some(dir.clone()));
        l.error("che disastro");
        let got = rx.try_recv().unwrap();
        assert_eq!(got.tag, Tag::Error);
        assert!(got.text.contains("che disastro"));
        let content = std::fs::read_to_string(dir.join("meditation_app.log")).unwrap();
        assert!(content.contains("che disastro"));
    }
}
