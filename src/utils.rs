use std::fs;
use std::io::{Error, ErrorKind};
use std::path::Path;

use crate::error::{Result, ServerError};

/// Safely creates a directory and all parent directories if they don't exist
/// Throws an error if path exists but is not a directory
pub fn ensure_dir_exists(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        if !path.is_dir() {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                format!("Path exists but is not a directory: {}", path.display()),
            ));
        }
    } else {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Wraps a Result with additional context in the error message
pub fn with_context<T, E: Into<ServerError>, C: FnOnce() -> String>(
    result: std::result::Result<T, E>,
    context: C,
) -> Result<T> {
    result.map_err(|e| {
        let err: ServerError = e.into();
        match err {
            ServerError::Io(io_err) => ServerError::Io(Error::new(
                io_err.kind(),
                format!("{}: {}", context(), io_err),
            )),
            ServerError::DocLoader(doc_err) => ServerError::DocLoader(doc_err),
            ServerError::Json(json_err) => ServerError::Json(json_err),
            ServerError::OpenAI(openai_err) => ServerError::OpenAI(openai_err),
            // Handle other error types
            _ => ServerError::Config(format!("{}: {:?}", context(), err)),
        }
    })
}
