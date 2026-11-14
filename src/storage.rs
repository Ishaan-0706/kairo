use crate::event::Event;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// Errors that can occur during JSONL event storage operations.
#[derive(Debug)]
pub enum StorageError {
    /// Underlying filesystem I/O error (permission denied, disk full, etc.)
    Io(io::Error),
    /// Error serializing an event to JSON string
    Serialization(serde_json::Error),
    /// A line in the JSONL file could not be parsed into a valid Event
    MalformedLine {
        line_number: usize,
        source: serde_json::Error,
    },
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::Io(err) => write!(f, "Storage I/O error: {err}"),
            StorageError::Serialization(err) => write!(f, "Serialization error: {err}"),
            StorageError::MalformedLine { line_number, source } => {
                write!(f, "Malformed JSON on line {line_number}: {source}")
            }
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            StorageError::Io(err) => Some(err),
            StorageError::Serialization(err) => Some(err),
            StorageError::MalformedLine { source, .. } => Some(source),
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(err: io::Error) -> Self {
        StorageError::Io(err)
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(err: serde_json::Error) -> Self {
        StorageError::Serialization(err)
    }
}

/// JSONL (JSON Lines) event storage engine.
///
/// Each event is stored as a single, self-contained JSON line terminated by a newline.
/// Events are appended sequentially to preserve execution history without overwriting.
pub struct JsonlStorage;

impl JsonlStorage {
    /// Appends a single event to a JSONL file. Creates the file if it does not exist.
    pub fn append_event(path: impl AsRef<Path>, event: &Event) -> Result<(), StorageError> {
        Self::append_events(path, std::slice::from_ref(event))
    }

    /// Appends a slice of events in sequence to a JSONL file.
    ///
    /// Uses append mode to ensure existing events are never overwritten.
    /// Uses a buffered writer for efficient batch I/O syscalls.
    pub fn append_events(path: impl AsRef<Path>, events: &[Event]) -> Result<(), StorageError> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        let mut writer = BufWriter::new(file);
        for event in events {
            let line = serde_json::to_string(event)?;
            writeln!(writer, "{line}")?;
        }
        writer.flush()?;
        Ok(())
    }

    /// Reads all events from a JSONL file in order.
    ///
    /// - Returns `Ok(empty)` if the file does not exist or is empty.
    /// - Safely skips empty or whitespace-only lines.
    /// - Returns `Err(StorageError::MalformedLine)` on the first corrupted line.
    pub fn read_events(path: impl AsRef<Path>) -> Result<Vec<Event>, StorageError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();

        for (idx, line_res) in reader.lines().enumerate() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let event = serde_json::from_str::<Event>(trimmed).map_err(|err| {
                StorageError::MalformedLine {
                    line_number: idx + 1,
                    source: err,
                }
            })?;
            events.push(event);
        }

        Ok(events)
    }

    /// Reads events tolerantly, collecting valid events and logging corrupted lines
    /// rather than aborting. Returns `(valid_events, malformed_line_errors)`.
    pub fn read_events_tolerant(
        path: impl AsRef<Path>,
    ) -> Result<(Vec<Event>, Vec<(usize, String)>), StorageError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok((Vec::new(), Vec::new()));
        }

        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        let mut malformed = Vec::new();

        for (idx, line_res) in reader.lines().enumerate() {
            let line = line_res?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Event>(trimmed) {
                Ok(ev) => events.push(ev),
                Err(err) => malformed.push((idx + 1, err.to_string())),
            }
        }

        Ok((events, malformed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventType;
    use serde_json::json;
    use std::fs;

    fn temp_test_file(name: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        let unique = uuid::Uuid::new_v4().to_string();
        path.push(format!("kairo_test_{name}_{unique}.jsonl"));
        path
    }

    #[test]
    fn test_write_and_read_single_event() {
        let file_path = temp_test_file("single");
        let event = Event::new("run-1", EventType::RunStarted, json!({"agent": "test"}));

        // Append one event
        JsonlStorage::append_event(&file_path, &event).expect("Append failed");

        // Read back
        let events = JsonlStorage::read_events(&file_path).expect("Read failed");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);

        let _ = fs::remove_file(file_path);
    }

    #[test]
    fn test_write_multiple_events() {
        let file_path = temp_test_file("multiple");
        let event1 = Event::new("run-1", EventType::RunStarted, json!({}));
        let event2 = Event::new("run-1", EventType::ToolStarted, json!({"tool": "search"}));
        let event3 = Event::new("run-1", EventType::RunCompleted, json!({"status": "ok"}));

        JsonlStorage::append_events(&file_path, &[event1.clone(), event2.clone(), event3.clone()])
            .expect("Batch append failed");

        let events = JsonlStorage::read_events(&file_path).expect("Read failed");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0], event1);
        assert_eq!(events[1], event2);
        assert_eq!(events[2], event3);

        let _ = fs::remove_file(file_path);
    }

    #[test]
    fn test_read_empty_file() {
        let file_path = temp_test_file("empty");
        File::create(&file_path).expect("Failed to create empty file");

        let events = JsonlStorage::read_events(&file_path).expect("Read empty file failed");
        assert!(events.is_empty());

        let _ = fs::remove_file(file_path);
    }

    #[test]
    fn test_read_missing_file_returns_empty() {
        let file_path = temp_test_file("nonexistent");
        assert!(!file_path.exists());

        let events = JsonlStorage::read_events(&file_path).expect("Missing file should return empty");
        assert!(events.is_empty());
    }

    #[test]
    fn test_handle_malformed_lines() {
        let file_path = temp_test_file("malformed");
        let valid_event = Event::new("run-1", EventType::RunStarted, json!({}));

        // Write: valid line, malformed line, another valid line
        JsonlStorage::append_event(&file_path, &valid_event).unwrap();
        {
            let mut file = OpenOptions::new().append(true).open(&file_path).unwrap();
            writeln!(file, "{{corrupted json line}}").unwrap();
        }
        let valid_event2 = Event::new("run-1", EventType::RunCompleted, json!({}));
        JsonlStorage::append_event(&file_path, &valid_event2).unwrap();

        // 1. Strict read returns MalformedLine error pointing to line 2
        let strict_res = JsonlStorage::read_events(&file_path);
        match strict_res {
            Err(StorageError::MalformedLine { line_number, .. }) => {
                assert_eq!(line_number, 2);
            }
            other => panic!("Expected MalformedLine error on line 2, got: {other:?}"),
        }

        // 2. Tolerant read retrieves the 2 valid events and records 1 malformed line
        let (valid, malformed) = JsonlStorage::read_events_tolerant(&file_path).unwrap();
        assert_eq!(valid.len(), 2);
        assert_eq!(malformed.len(), 1);
        assert_eq!(malformed[0].0, 2); // line 2 was malformed

        let _ = fs::remove_file(file_path);
    }

    #[test]
    fn test_append_does_not_overwrite_previous_events() {
        let file_path = temp_test_file("append_preserve");
        let event1 = Event::new("run-1", EventType::RunStarted, json!({"step": 1}));
        let event2 = Event::new("run-1", EventType::ToolStarted, json!({"step": 2}));

        // Write first event
        JsonlStorage::append_event(&file_path, &event1).unwrap();
        let intermediate = JsonlStorage::read_events(&file_path).unwrap();
        assert_eq!(intermediate.len(), 1);

        // Append second event later
        JsonlStorage::append_event(&file_path, &event2).unwrap();
        let final_events = JsonlStorage::read_events(&file_path).unwrap();
        assert_eq!(final_events.len(), 2);
        assert_eq!(final_events[0], event1);
        assert_eq!(final_events[1], event2);

        let _ = fs::remove_file(file_path);
    }
}
